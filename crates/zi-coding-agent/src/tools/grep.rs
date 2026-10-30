//! grep tool: search file contents (ripgrep preferred; falls back to `grep -R`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zi_agent::{AgentToolResult, FnAgentTool};
use tokio::process::Command;

use super::path::resolve_to_cwd;
use super::spill::{spill_if_large, DEFAULT_SPILL_THRESHOLD};

const DEFAULT_LIMIT: usize = 100;

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
    let ignore_case = args.get("ignoreCase").and_then(|v| v.as_bool()).unwrap_or(false);
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
    cmd.arg(pattern).arg(search_path);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() == Some(127)
                || stderr.contains("No such file or directory")
            {
                return Err("rg not executable".into());
            }
            if !output.status.success() && output.status.code() != Some(1) {
                return Err(format!("rg failed: {stderr}"));
            }
            Ok(truncate_output(stdout.trim_end()))
        }
        Err(err) => Err(format!("failed to run rg: {err}")),
    }
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
    cmd.arg(pattern).arg(search_path);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // 0 = matches, 1 = no matches; other = error (unless only --include warn).
            if !output.status.success()
                && output.status.code() != Some(1)
                && stdout.trim().is_empty()
            {
                return AgentToolResult::error(format!(
                    "grep fallback failed (rg unavailable): {stderr}"
                ));
            }
            let text = limit_match_lines(stdout.trim_end(), limit);
            let mut body = if text.is_empty() {
                "(no matches)".to_string()
            } else {
                text
            };
            body.push_str("\n\n[grep] used system grep fallback (ripgrep not on PATH)");
            truncate_output(&body)
        }
        Err(err) => AgentToolResult::error(format!(
            "ripgrep and system grep unavailable: {err}"
        )),
    }
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
        let text = (0..20).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n");
        let limited = limit_match_lines(&text, 5);
        assert_eq!(limited.lines().count(), 6); // 5 + truncated notice
        assert!(limited.contains("truncated"));
    }

    #[test]
    fn resolve_rg_bin_does_not_panic() {
        let _ = resolve_rg_bin();
    }
}
