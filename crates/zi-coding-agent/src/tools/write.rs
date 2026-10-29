//! Write tool — create or overwrite files under the workspace.

use std::path::PathBuf;

use async_trait::async_trait;
use zi_agent::{AgentTool, AgentToolResult};
use serde_json::{json, Value};

use super::resolve_in_workspace;

/// Write file contents, creating parent directories as needed.
pub struct WriteTool {
    cwd: PathBuf,
}

impl WriteTool {
    /// Bind writes to `cwd`.
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file and parent directories if needed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
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
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return AgentToolResult::error("missing required parameter: content"),
        };

        let absolute = resolve_in_workspace(&self.cwd, path);
        if let Some(parent) = absolute.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                return AgentToolResult::error(format!(
                    "failed to create directories for {}: {err}",
                    absolute.display()
                ));
            }
        }

        if let Err(err) = tokio::fs::write(&absolute, content).await {
            return AgentToolResult::error(format!(
                "failed to write {}: {err}",
                absolute.display()
            ));
        }

        AgentToolResult::text(format!(
            "Wrote {} ({} bytes)",
            absolute.display(),
            content.len()
        ))
    }
}
