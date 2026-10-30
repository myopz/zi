//! Read tool — file contents with optional line range and truncation.

use std::path::PathBuf;

use async_trait::async_trait;
use zi_agent::{AgentTool, AgentToolResult};
use serde_json::{json, Value};

use super::resolve_in_workspace;
use super::spill::{spill_if_large, DEFAULT_SPILL_THRESHOLD};

const DEFAULT_MAX_LINES: usize = 120;
const DEFAULT_MAX_BYTES: usize = 64 * 1024;

/// Read files under the workspace cwd.
pub struct ReadTool {
    cwd: PathBuf,
}

impl ReadTool {
    /// Bind reads to `cwd`.
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read one text file (default ≤120 lines; line numbers every 10 lines). Use offset/limit for more. Prefer over bash cat/sed. For multi-file discovery use grep/find or explore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file (relative or absolute)" },
                "offset": { "type": "number", "description": "Line number to start reading from (1-indexed)" },
                "limit": { "type": "number", "description": "Maximum number of lines to read" }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        _on_update: Option<zi_agent::AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return AgentToolResult::error("missing required parameter: path"),
        };
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize);

        let absolute = resolve_in_workspace(&self.cwd, path);
        let bytes = match tokio::fs::read(&absolute).await {
            Ok(b) => b,
            Err(err) => {
                return AgentToolResult::error(format!(
                    "failed to read {}: {err}",
                    absolute.display()
                ));
            }
        };

        if bytes.iter().take(8192).any(|b| *b == 0) {
            return AgentToolResult::error(format!(
                "{} appears to be binary; read supports text files only",
                absolute.display()
            ));
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let start = offset.map(|o| o.saturating_sub(1)).unwrap_or(0);
        if start >= lines.len() && !lines.is_empty() {
            return AgentToolResult::text(format!(
                "offset {} is past end of file ({} lines)",
                start + 1,
                lines.len()
            ));
        }

        let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);
        let selected: Vec<&str> = lines.iter().skip(start).take(max_lines).copied().collect();
        if lines.is_empty() {
            return AgentToolResult::text("(empty file)".to_string());
        }

        let mut numbered: String = selected
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let line_no = start + i + 1;
                // Cursor-style: number every 10th line (and the first shown line).
                if i == 0 || line_no % 10 == 1 {
                    format!("{line_no:6}|{line}")
                } else {
                    format!("{:6}|{line}", "")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if start + selected.len() < lines.len() {
            let next = start + selected.len() + 1;
            numbered.push_str(&format!(
                "\n… truncated; call read again with offset={next} limit={} (showing {} lines from line {})",
                max_lines,
                selected.len(),
                start + 1
            ));
        }

        // Cap raw assembly, then spill oversized bodies so the transcript stays short.
        if numbered.len() > DEFAULT_MAX_BYTES {
            numbered.truncate(DEFAULT_MAX_BYTES);
            numbered.push_str("\n… (truncated by byte limit)");
        }
        AgentToolResult::text(spill_if_large(&numbered, DEFAULT_SPILL_THRESHOLD))
    }
}
