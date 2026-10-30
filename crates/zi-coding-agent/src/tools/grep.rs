//! grep tool: search file contents (ripgrep preferred; falls back to `grep -R`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use zi_agent::{AgentToolResult, FnAgentTool};

use super::path::resolve_to_cwd;
use super::spill::{spill_if_large, DEFAULT_SPILL_THRESHOLD};

const DEFAULT_LIMIT: usize = 100;
/// Hard cap so a broad pattern cannot freeze the agent loop.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const EXCLUDE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    "vendor",
];

static PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "pattern": { "type": "string", "description": "Search pattern (regex or literal)" },
    "path": { "type": "string", "description": "Directory or file to search (default: cwd)" },
    "glob": { "type": "string", "description": "Filter files by glob, e.g. '*.rs'" },
    "ignoreCase": { "type": "boolean", "description": "Case-insensitive search" },
    "literal": { "type": "boolean", "description": "Treat pattern as literal string" },
    "context": { "type": "integer", "description": "Lines of context before/after each match" },
    "limit": { "type": "integer", "description": "Maximum matches (default 100)" }
  },
  "required": ["pattern"]
}"#;

/// Build the grep [`FnAgentTool`].
pub fn grep_tool(cwd: PathBuf) -> Arc<FnAgentTool> {
    FnAgentTool::new(
        "grep",
        "Grepped: search file contents with ripgrep when available, else system grep (cheap; prefer over bash). Default limit 100. Use path/glob to narrow. For wide unknown exploration use explore.",
        serde_json::from_str(PARAMETERS).expect("grep parameters schema"),
        move |_id, args, _on_update| {
            let cwd = cwd.clone();
            async move { run_grep(&cwd, args).await }
        },
    )
}

async fn run_grep(cwd: &PathBuf, args: serde_json::Value) -> AgentToolResult {
    let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return AgentToolResult::error("pattern is required"),
    };
    let search_path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => match resolve_to_cwd(cwd, Some(p)) {
            Ok(path) => path,
            Err(err) => return AgentToolResult::error(err),
        },
        None => match resolve_to_cwd(cwd, None) {
            Ok(path) => path,
            Err(err) => return AgentToolResult::error(err),
        },
    };
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .max(1);
    let ignore_case = args
        .get("ignoreCase")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let literal = args.get("literal").and_then(|v| v.as_bool()).unwrap_or(false);
    let context = args
        .get("context")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let glob = args.get("glob").and_then(|v| v.as_str());

    if let Some(rg) = resolve_rg_bin() {
        match run_rg(
            &rg,
            &search_path,
            pattern,
            glob,
            ignore_case,
            literal,
            context,
            limit,
        )
        .await
        {
            Ok(result) => return result,
            Err(err) => {
                tracing::warn!(%err, "rg failed; falling back to system grep");
            }
        }
    }

    run_system_grep(
        &search_path,
        pattern,
        glob,
        ignore_case,
        literal,
        context,
        limit,
    )
    .await
}

/// Resolve a ripgrep binary: PATH first, then common install locations.
pub(crate) fn resolve_rg_bin() -> Option<PathBuf> {
    if which_ok("rg") {
        return Some(PathBuf::from("rg"));
    }
    for candidate in [
        "/opt/homebrew/bin/rg",
        "/usr/local/bin/rg",
        "/usr/bin/rg",
    ] {
        let path = Path::new(candidate);
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn which_ok(name: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run_rg(
    rg: &Path,
    search_path: &Path,
    pattern: &str,
    glob: Option<&str>,
    ignore_case: bool,
    literal: bool,
    context: usize,
    limit: usize,
) -> Result<AgentToolResult, String> {
    let mut cmd = Command::new(rg);
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        // Per-file cap; total cap is enforced while reading stdout.
        .arg("--max-count")
        .arg(limit.to_string());
    if ignore_case {
        cmd.arg("-i");
    }
    if literal {
        cmd.arg("-F");
    }
    if context > 0 {
        cmd.arg("-C").arg(context.to_string());
    }
    if let Some(g) = glob {
        cmd.arg("-g").arg(g);
    }
    for dir in EXCLUDE_DIRS {
        cmd.arg("-g").arg(format!("!{dir}/"));
        cmd.arg("-g").arg(format!("!{dir}/**"));
    }
    cmd.arg("--").arg(pattern).arg(search_path);

    let captured = run_capped(cmd, limit).await?;
    if captured.spawn_missing {
        return Err("rg not executable".into());
    }
    if captured.timed_out {
        return Ok(finish_matches(
            &captured.stdout,
            limit,
            Some(&format!(
                "[grep] timed out after {}s (narrow path/glob or install ripgrep)",
                DEFAULT_TIMEOUT.as_secs()
            )),
            false,
        ));
    }
    if let Some(err) = captured.fatal_stderr() {
        return Err(format!("rg failed: {err}"));
    }
    Ok(finish_matches(
        &captured.stdout,
        limit,
        None,
        captured.truncated,
    ))
}

async fn run_system_grep(
    search_path: &Path,
    pattern: &str,
    glob: Option<&str>,
    ignore_case: bool,
    literal: bool,
    context: usize,
    limit: usize,
) -> AgentToolResult {
    let mut cmd = Command::new("grep");
    cmd.arg("-RHn").arg("--color=never");
    if ignore_case {
        cmd.arg("-i");
    }
    if literal {
        cmd.arg("-F");
    } else {
        cmd.arg("-E");
    }
    if context > 0 {
        cmd.arg("-C").arg(context.to_string());
    }
    if let Some(g) = glob {
        // GNU/BSD grep --include; ignore if unsupported (still search, noisier).
        cmd.arg(format!("--include={g}"));
    }
    for dir in EXCLUDE_DIRS {
        cmd.arg(format!("--exclude-dir={dir}"));
    }
    cmd.arg("--").arg(pattern).arg(search_path);

    match run_capped(cmd, limit).await {
        Ok(captured) => {
            if captured.spawn_missing {
                return AgentToolResult::error(
                    "ripgrep and system grep unavailable: grep binary missing",
                );
            }
            if captured.timed_out {
                return finish_matches(
                    &captured.stdout,
                    limit,
                    Some(&format!(
                        "[grep] system grep timed out after {}s (install ripgrep; narrow path/glob)",
                        DEFAULT_TIMEOUT.as_secs()
                    )),
                    false,
                );
            }
            if let Some(err) = captured.fatal_stderr() {
                if captured.stdout.trim().is_empty() {
                    return AgentToolResult::error(format!(
                        "grep fallback failed (rg unavailable): {err}"
                    ));
                }
            }
            finish_matches(
                &captured.stdout,
                limit,
                Some("[grep] used system grep fallback (ripgrep not on PATH)"),
                captured.truncated,
            )
        }
        Err(err) => AgentToolResult::error(format!("ripgrep and system grep unavailable: {err}")),
    }
}

struct Captured {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    timed_out: bool,
    truncated: bool,
    spawn_missing: bool,
}

impl Captured {
    fn fatal_stderr(&self) -> Option<&str> {
        if self.timed_out {
            return None;
        }
        // 0 = matches, 1 = no matches (rg/grep).
        if self.code == Some(0) || self.code == Some(1) {
            return None;
        }
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        }
    }
}

/// Spawn `cmd`, collect at most `limit` stdout lines, kill on timeout or early fill.
async fn run_capped(mut cmd: Command, limit: usize) -> Result<Captured, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                return Ok(Captured {
                    stdout: String::new(),
                    stderr: String::new(),
                    code: Some(127),
                    timed_out: false,
                    truncated: false,
                    spawn_missing: true,
                });
            }
            return Err(format!("failed to spawn: {err}"));
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let limit = limit.max(1);

    let read_out = async move {
        let mut lines = Vec::new();
        let mut truncated = false;
        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                lines.push(line);
                if lines.len() >= limit {
                    truncated = true;
                    break;
                }
            }
        }
        (lines.join("\n"), truncated)
    };
    let read_err = async move {
        let mut err_buf = String::new();
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err);
            let mut buf = String::new();
            while err_buf.len() < 8 * 1024 {
                buf.clear();
                match reader.read_line(&mut buf).await {
                    Ok(0) => break,
                    Ok(_) => err_buf.push_str(&buf),
                    Err(_) => break,
                }
            }
        }
        err_buf
    };

    let out_task = tokio::spawn(read_out);
    let err_task = tokio::spawn(read_err);

    let mut timed_out = false;
    let mut stdout_text = String::new();
    let mut truncated = false;

    tokio::select! {
        result = out_task => {
            match result {
                Ok((out, hit_limit)) => {
                    stdout_text = out;
                    truncated = hit_limit;
                }
                Err(_) => {}
            }
        }
        _ = tokio::time::sleep(DEFAULT_TIMEOUT) => {
            timed_out = true;
            truncated = true;
        }
    }

    // Enough lines or deadline: stop the child so pipe readers unblock.
    let _ = child.start_kill();
    let code = match child.wait().await {
        Ok(status) => status.code(),
        Err(_) => None,
    };

    let stderr_text = match tokio::time::timeout(Duration::from_secs(1), err_task).await {
        Ok(Ok(err)) => {
            if timed_out && err.trim().is_empty() {
                format!("timed out after {}s", DEFAULT_TIMEOUT.as_secs())
            } else {
                err
            }
        }
        _ if timed_out => format!("timed out after {}s", DEFAULT_TIMEOUT.as_secs()),
        _ => String::new(),
    };

    Ok(Captured {
        stdout: stdout_text,
        stderr: stderr_text,
        code,
        timed_out,
        truncated,
        spawn_missing: false,
    })
}

fn finish_matches(
    text: &str,
    limit: usize,
    footer: Option<&str>,
    stream_capped: bool,
) -> AgentToolResult {
    let mut body = if text.trim().is_empty() {
        "(no matches)".to_string()
    } else {
        limit_match_lines(text.trim_end(), limit)
    };
    // `run_capped` already stops at `limit` lines, so `limit_match_lines` sees no excess —
    // surface early kill when the stream hit the cap.
    if stream_capped && !text.trim().is_empty() && !body.contains("… truncated") {
        body.push_str(&format!("\n… truncated to {limit} lines"));
    }
    if let Some(note) = footer {
        body.push_str("\n\n");
        body.push_str(note);
    }
    truncate_output(&body)
}

fn limit_match_lines(text: &str, limit: usize) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= limit {
        return text.to_string();
    }
    let mut out = lines[..limit].join("\n");
    out.push_str(&format!("\n… truncated to {limit} lines"));
    out
}

fn truncate_output(text: &str) -> AgentToolResult {
    if text.is_empty() {
        return AgentToolResult::text("(no matches)");
    }
    AgentToolResult::text(spill_if_large(text, DEFAULT_SPILL_THRESHOLD))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_match_lines_caps() {
        let text = (0..20)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let limited = limit_match_lines(&text, 5);
        assert_eq!(limited.lines().count(), 6); // 5 + truncated notice
        assert!(limited.contains("truncated"));
    }

    #[test]
    fn resolve_rg_bin_does_not_panic() {
        let _ = resolve_rg_bin();
    }

    #[tokio::test]
    async fn capped_runner_stops_after_limit() {
        // A process that would print forever; we must stop after `limit` lines.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("i=0; while true; do echo line$i; i=$((i+1)); done");
        let captured = run_capped(cmd, 5).await.expect("spawn");
        assert!(!captured.spawn_missing);
        assert_eq!(captured.stdout.lines().count(), 5);
        assert!(captured.truncated);
        assert!(!captured.timed_out);
    }

    #[tokio::test]
    async fn capped_runner_reads_short_output() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo ok");
        let captured = run_capped(cmd, 10).await.expect("spawn");
        assert_eq!(captured.stdout.trim(), "ok");
        assert!(!captured.timed_out);
        assert!(!captured.truncated);
    }
}
