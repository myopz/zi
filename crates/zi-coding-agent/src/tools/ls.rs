//! ls tool: list directory entries.

use std::path::PathBuf;
use std::sync::Arc;

use zi_agent::{AgentToolResult, FnAgentTool};
use tokio::fs;

use super::path::resolve_to_cwd;

const DEFAULT_LIMIT: usize = 500;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

static PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "path": { "type": "string", "description": "Directory to list (default: cwd)" },
    "limit": { "type": "integer", "description": "Maximum entries (default 500)" }
  }
}"#;

/// Build the ls [`FnAgentTool`].
pub fn ls_tool(cwd: PathBuf) -> Arc<FnAgentTool> {
    FnAgentTool::new(
        "ls",
        "List directory contents. Directories are suffixed with '/'. Includes dotfiles.",
        serde_json::from_str(PARAMETERS).expect("ls parameters schema"),
        move |_id, args, _on_update| {
            let cwd = cwd.clone();
            async move { run_ls(&cwd, args).await }
        },
    )
}

async fn run_ls(cwd: &PathBuf, args: serde_json::Value) -> AgentToolResult {
    let dir = match args.get("path").and_then(|v| v.as_str()) {
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

    let meta = match fs::metadata(&dir).await {
        Ok(m) => m,
        Err(err) => return AgentToolResult::error(format!("{}: {err}", dir.display())),
    };
    if !meta.is_dir() {
        return AgentToolResult::error(format!("not a directory: {}", dir.display()));
    }

    let mut entries = match fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(err) => return AgentToolResult::error(format!("read_dir failed: {err}")),
    };

    let mut names: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if names.len() >= limit {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let suffix = if path.is_dir() { "/" } else { "" };
        names.push(format!("{name}{suffix}"));
    }
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    let text = if names.is_empty() {
        "(empty directory)".to_string()
    } else {
        names.join("\n")
    };
    truncate_output(&text)
}

fn truncate_output(text: &str) -> AgentToolResult {
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
