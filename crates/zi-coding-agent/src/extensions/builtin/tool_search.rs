//! Built-in `tool_search` tool — lists registered tool names (discovery skeleton).

use std::sync::Arc;

use zi_agent::{AgentToolResult, FnAgentTool};

use crate::extensions::{Extension, ExtensionLoadContext, SharedToolList};

/// Registers `tool_search`, which returns names of tools currently in the shared registry.
pub struct ToolSearchBuiltin {
    tool_list: SharedToolList,
}

impl ToolSearchBuiltin {
    pub fn new(tool_list: SharedToolList) -> Self {
        Self { tool_list }
    }
}

impl Extension for ToolSearchBuiltin {
    fn id(&self) -> &str {
        "builtin:tool_search"
    }

    fn load(&self, ctx: &mut ExtensionLoadContext<'_>) {
        let tool_list = Arc::clone(&self.tool_list);
        let tool = FnAgentTool::new(
            "tool_search",
            "List tool names registered in this session (query filters by substring, case-insensitive).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional substring filter."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max names to return.",
                        "minimum": 1
                    }
                },
                "additionalProperties": false
            }),
            move |_id, args, _on_update| {
                let tool_list = Arc::clone(&tool_list);
                async move {
                    let query = args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let limit = args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(64)
                        .min(256) as usize;

                    let guard = tool_list.read().expect("tool list lock");
                    let mut names: Vec<String> = guard
                        .iter()
                        .map(|t| t.name().to_string())
                        .collect();
                    names.sort();
                    names.dedup();

                    if let Some(q) = query {
                        let q_lower = q.to_ascii_lowercase();
                        names.retain(|n| n.to_ascii_lowercase().contains(&q_lower));
                    }
                    names.truncate(limit);

                    if names.is_empty() {
                        return AgentToolResult::text("No matching tools found.");
                    }
                    AgentToolResult::text(names.join("\n"))
                }
            },
        );
        ctx.register_tool(tool);
    }
}
