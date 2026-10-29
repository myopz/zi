//! Built-in MCP extension skeleton (full server wiring lives in [`crate::mcp_bridge`]).

use zi_agent::{AgentToolResult, FnAgentTool};

use crate::extensions::{Extension, ExtensionHook, ExtensionLoadContext};

/// Registers MCP-related hooks and a placeholder status tool until interactive `/mcp` lands.
pub struct McpBuiltin;

impl McpBuiltin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for McpBuiltin {
    fn default() -> Self {
        Self::new()
    }
}

impl Extension for McpBuiltin {
    fn id(&self) -> &str {
        "builtin:mcp"
    }

    fn load(&self, ctx: &mut ExtensionLoadContext<'_>) {
        ctx.subscribe_hook(ExtensionHook::SessionStart);
        let tool = FnAgentTool::new(
            "mcp_status",
            "Report MCP integration status for this zi build (servers connect via mcp_bridge when configured).",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            |_id, _args, _on_update| async move {
                AgentToolResult::text(
                    "MCP: configure servers in ~/.zi/agent/mcp.json (or trusted .zi/mcp.json). \
                     Use /mcp in interactive mode to inspect connection status. \
                     Tools are mounted as mcp_<server>_<tool> on startup.",
                )
            },
        );
        ctx.register_tool(tool);
    }
}
