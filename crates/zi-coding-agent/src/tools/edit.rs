//! Edit tool — exact substring replacements in a file.

use std::path::PathBuf;

use async_trait::async_trait;
use zi_agent::{AgentTool, AgentToolResult};
use serde_json::{json, Value};

use super::resolve_in_workspace;

/// Apply one or more exact `oldText` → `newText` edits against the original file.
pub struct EditTool {
    cwd: PathBuf,
}

impl EditTool {
    /// Bind edits to `cwd`.
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[derive(Debug, serde::Deserialize)]
struct SingleEdit {
    #[serde(rename = "oldText")]
    old_text: String,
    #[serde(rename = "newText")]
    new_text: String,
}

#[derive(Debug, serde::Deserialize)]
struct EditArgs {
    path: String,
    edits: Vec<SingleEdit>,
}

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Make precise file edits with exact text replacement. Each oldText is matched against the original file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": { "type": "string" },
                            "newText": { "type": "string" }
                        },
                        "required": ["oldText", "newText"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        _on_update: Option<zi_agent::AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        let parsed: EditArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(err) => return AgentToolResult::error(format!("invalid arguments: {err}")),
        };
        if parsed.edits.is_empty() {
            return AgentToolResult::error("edits must contain at least one entry");
        }

        let absolute = resolve_in_workspace(&self.cwd, &parsed.path);
        let original = match tokio::fs::read_to_string(&absolute).await {
            Ok(s) => s,
            Err(err) => {
                return AgentToolResult::error(format!(
                    "failed to read {}: {err}",
                    absolute.display()
                ));
            }
        };

        let mut content = original.clone();
        for edit in &parsed.edits {
            let count = content.matches(&edit.old_text).count();
            if count == 0 {
                return AgentToolResult::error(format!(
                    "oldText not found in {} (unique match required)",
                    absolute.display()
                ));
            }
            if count > 1 {
                return AgentToolResult::error(format!(
                    "oldText matches {count} times in {}; must be unique",
                    absolute.display()
                ));
            }
            content = content.replacen(&edit.old_text, &edit.new_text, 1);
        }

        if let Err(err) = tokio::fs::write(&absolute, &content).await {
            return AgentToolResult::error(format!(
                "failed to write {}: {err}",
                absolute.display()
            ));
        }

        AgentToolResult::text(format!(
            "Updated {} ({} edit(s))",
            absolute.display(),
            parsed.edits.len()
        ))
    }
}
