//! Built-in `codemode` tool (zi-codemode DSL / optional QuickJS).

use std::collections::HashMap;
use std::sync::Arc;

use zi_agent::{AgentTool, AgentToolResult, FnAgentTool};
use zi_ai::TextContent;
use zi_codemode::{default_sandbox, tool_map_invoker, CodemodeError, ToolInvokerEntry};

use crate::extensions::{Extension, ExtensionLoadContext, SharedToolList};

/// Registers the `codemode` tool backed by [`zi_codemode`].
pub struct CodemodeBuiltin {
    tool_list: SharedToolList,
}

impl CodemodeBuiltin {
    pub fn new(tool_list: SharedToolList) -> Self {
        Self { tool_list }
    }
}

impl Extension for CodemodeBuiltin {
    fn id(&self) -> &str {
        "builtin:codemode"
    }

    fn load(&self, ctx: &mut ExtensionLoadContext<'_>) {
        let tool_list = Arc::clone(&self.tool_list);
        let tool = FnAgentTool::new(
            "codemode",
            "Run a short DSL script that calls registered tools. Each line must be \
             `await tool(\"name\", { ...json... })` (a leading `const x =` is tolerated). \
             Prefer calling explore/read/grep/find/ls directly as tools for exploration — \
             do not wrap a single explore call in codemode.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "One tool call per line, e.g. await tool(\"grep\", {\"pattern\": \"fn main\", \"glob\": \"*.rs\"}). No general JavaScript."
                    }
                },
                "required": ["script"],
                "additionalProperties": false
            }),
            move |_id, args, _on_update| {
                let tool_list = Arc::clone(&tool_list);
                async move {
                    let script = match args.get("script").and_then(|v| v.as_str()) {
                        Some(s) => s.to_string(),
                        None => return AgentToolResult::error("missing script"),
                    };
                    let invoker = build_invoker(&tool_list);
                    let sandbox = default_sandbox(invoker);
                    match sandbox.run(&script).await {
                        Ok(value) => AgentToolResult::text(value.to_string()),
                        Err(CodemodeError::Parse(msg) | CodemodeError::Runtime(msg)) => {
                            AgentToolResult::error(msg)
                        }
                        Err(CodemodeError::Tool { name, message }) => {
                            AgentToolResult::error(format!("tool {name}: {message}"))
                        }
                    }
                }
            },
        );
        ctx.register_tool(tool);
    }
}

fn build_invoker(tool_list: &SharedToolList) -> zi_codemode::ToolInvoker {
    let tools = tool_list.read().expect("tool list lock").clone();
    let mut map: HashMap<String, ToolInvokerEntry> = HashMap::new();
    for tool in tools {
        if tool.name() == "codemode" {
            continue;
        }
        let entry = codemode_entry(tool);
        map.insert(entry.0, entry.1);
    }
    tool_map_invoker(map)
}

fn codemode_entry(tool: Arc<dyn AgentTool>) -> (String, ToolInvokerEntry) {
    let name = tool.name().to_string();
    let handler: ToolInvokerEntry = Arc::new(move |args| {
        let tool = Arc::clone(&tool);
        Box::pin(async move {
            let result = tool.execute("codemode/nested", args, None).await;
            if result.is_error == Some(true) {
                let text = result
                    .content
                    .first()
                    .map(|c: &TextContent| c.text.as_str())
                    .unwrap_or("tool error");
                Err(CodemodeError::Tool {
                    name: tool.name().to_string(),
                    message: text.to_string(),
                })
            } else {
                Ok(result)
            }
        })
    });
    (name, handler)
}
