//! find tool: locate files by glob (prefers fd when installed).

use std::path::PathBuf;
use std::sync::Arc;

use zi_agent::{AgentToolResult, FnAgentTool};
use tokio::process::Command;

use super::path::resolve_to_cwd;

const DEFAULT_LIMIT: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

static PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "pattern": { "type": "string", "description": "Glob pattern, e.g. '*.rs' or '**/*.json'" },
    "path": { "type": "string", "description": "Directory to search (default: cwd)" },
    "limit": { "type": "integer", "description": "Maximum results (default 1000)" }
  },
  "required": ["pattern"]
}"#;

/// Build the find [`FnAgentTool`].
pub fn find_tool(cwd: PathBuf) -> Arc<FnAgentTool> {
    FnAgentTool::new(
        "find",
        "Find files by glob pattern. Prefers fd when available; falls back to POSIX find.",
        serde_json::from_str(PARAMETERS).expect("find parameters schema"),
        move |_id, args, _on_update| {
            let cwd = cwd.clone();
            async move { run_find(&cwd, args).await }
        },
    )
}

async fn run_find(cwd: &PathBuf, args: serde_json::Value) -> AgentToolResult {
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

    if fd_available().await {
        run_fd(&search_path, pattern, limit).await
    } else {
        run_posix_find(&search_path, pattern, limit).await
    }
}

async fn fd_available() -> bool {
    Command::new("fd")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn run_fd(search_path: &PathBuf, pattern: &str, limit: usize) -> AgentToolResult {
    let output = Command::new("fd")
        .arg("--glob")
        .arg(pattern)
        .arg("--max-results")
        .arg(limit.to_string())
        .arg("--color=never")
        .arg(search_path)
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !out.status.success() && out.status.code() != Some(1) {
                return AgentToolResult::error(format!("fd failed: {stderr}"));
            }
            truncate_output(stdout.trim_end())
        }
        Err(err) => AgentToolResult::error(format!("failed to run fd: {err}")),
    }
}

async fn run_posix_find(search_path: &PathBuf, pattern: &str, limit: usize) -> AgentToolResult {
    let name_glob = pattern.trim_start_matches("./").trim_start_matches("**/");
    let output = Command::new("find")
        .arg(search_path)
        .arg("-name")
        .arg(name_glob)
        .output()
        .await;

    match output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return AgentToolResult::error(format!("find failed: {stderr}"));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout.lines().take(limit).collect();
            if lines.is_empty() {
                return AgentToolResult::text("(no matches)");
            }
            truncate_output(&lines.join("\n"))
        }
        Err(err) => AgentToolResult::error(format!("failed to run find: {err}")),
    }
}

fn truncate_output(text: &str) -> AgentToolResult {
    if text.is_empty() {
        return AgentToolResult::text("(no matches)");
    }
    let bytes = text.as_bytes();
    if bytes.len() <= MAX_OUTPUT_BYTES {
        return AgentToolResult::text(text);
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    AgentToolResult::text(format!(
        "{}\n\n[truncated at {}KB]",
        &text[..end],
        MAX_OUTPUT_BYTES / 1024
    ))
}
