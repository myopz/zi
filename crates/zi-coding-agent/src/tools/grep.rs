//! grep tool: search file contents (requires ripgrep on PATH).

use std::path::PathBuf;
use std::sync::Arc;

use zi_agent::{AgentToolResult, FnAgentTool};
use tokio::process::Command;

use super::path::resolve_to_cwd;

const DEFAULT_LIMIT: usize = 100;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

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
        "Search file contents for a pattern via ripgrep (rg). Respects .gitignore when rg is used.",
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

    run_rg(
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

async fn run_rg(
    search_path: &PathBuf,
    pattern: &str,
    glob: Option<&str>,
    ignore_case: bool,
    literal: bool,
    context: usize,
    limit: usize,
) -> AgentToolResult {
    let mut cmd = Command::new("rg");
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
            if output.status.code() == Some(127) || stderr.contains("No such file") {
                return AgentToolResult::error(
                    "ripgrep (rg) not found on PATH; install ripgrep to use grep",
                );
            }
            if !output.status.success() && output.status.code() != Some(1) {
                return AgentToolResult::error(format!("rg failed: {stderr}"));
            }
            truncate_output(stdout.trim_end())
        }
        Err(err) => AgentToolResult::error(format!("failed to run rg: {err}")),
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
