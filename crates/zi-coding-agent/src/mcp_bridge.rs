//! Mount MCP server tools as [`zi_agent::AgentTool`] implementations.

use std::sync::Arc;

use async_trait::async_trait;
use zi_agent::{AgentTool, AgentToolResult, AgentToolUpdateCallback};
use zi_ai::Tool as LlmToolDescriptor;
use zi_mcp::{to_llm_content, CallToolResult, McpClient, McpError, Tool as McpTool};
use serde_json::Value;

/// Serde-friendly LLM tool descriptor (same shape as [`zi_ai::Tool`]).
pub type AgentToolDescriptor = LlmToolDescriptor;

/// Options when mounting MCP tools onto an agent.
#[derive(Debug, Clone)]
pub struct McpMountOptions {
    /// Prefix every mounted tool name (e.g. `mcp_myserver_`).
    pub name_prefix: String,
    /// Optional namespace stored on tool calls for disambiguation.
    pub namespace: Option<String>,
}

impl Default for McpMountOptions {
    fn default() -> Self {
        Self {
            name_prefix: "mcp_".into(),
            namespace: None,
        }
    }
}

/// One MCP tool wrapped for agent execution plus its LLM descriptor.
#[derive(Debug, Clone)]
pub struct McpToolMount {
    pub descriptor: AgentToolDescriptor,
    pub agent_name: String,
    pub mcp_name: String,
}

/// List MCP tools and map them to LLM-facing JSON descriptors.
pub async fn list_mcp_tool_descriptors(
    client: &McpClient,
    options: &McpMountOptions,
) -> Result<Vec<McpToolMount>, McpError> {
    let tools = client.list_tools().await?;
    Ok(tools
        .into_iter()
        .map(|tool| mcp_tool_to_mount(&tool, options))
        .collect())
}

/// List tools and build executable [`AgentTool`] handles.
pub async fn mount_mcp_tools(
    client: Arc<McpClient>,
    options: McpMountOptions,
) -> Result<Vec<Arc<dyn AgentTool>>, McpError> {
    let mounts = list_mcp_tool_descriptors(client.as_ref(), &options).await?;
    Ok(mounts
        .into_iter()
        .map(|mount| {
            Arc::new(McpAgentTool {
                client: Arc::clone(&client),
                mcp_name: mount.mcp_name,
                agent_name: mount.agent_name,
                description: mount.descriptor.description.clone(),
                parameters: mount.descriptor.parameters.clone(),
            }) as Arc<dyn AgentTool>
        })
        .collect())
}

fn mcp_tool_to_mount(tool: &McpTool, options: &McpMountOptions) -> McpToolMount {
    let agent_name = format!("{}{}", options.name_prefix, tool.name);
    let description = tool
        .description
        .clone()
        .or(tool.title.clone())
        .unwrap_or_else(|| format!("MCP tool {}", tool.name));
    let mcp_name = tool.name.clone();
    McpToolMount {
        descriptor: LlmToolDescriptor {
            name: agent_name.clone(),
            description,
            parameters: tool.input_schema.clone(),
        },
        agent_name,
        mcp_name,
    }
}

struct McpAgentTool {
    client: Arc<McpClient>,
    mcp_name: String,
    agent_name: String,
    description: String,
    parameters: Value,
}

#[async_trait]
impl AgentTool for McpAgentTool {
    fn name(&self) -> &str {
        &self.agent_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        _on_update: Option<AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        let arguments = if args.is_null() {
            None
        } else {
            Some(args)
        };
        match self.client.call_tool(&self.mcp_name, arguments).await {
            Ok(result) => mcp_result_to_agent(result),
            Err(err) => AgentToolResult::error(format!("MCP tool {} failed: {err}", self.mcp_name)),
        }
    }
}

fn mcp_result_to_agent(result: CallToolResult) -> AgentToolResult {
    let is_error = result.is_error.unwrap_or(false);
    let llm = to_llm_content(&result);
    let text = llm
        .into_iter()
        .map(|block| match block {
            zi_mcp::LlmContent::Text { text } => text,
            zi_mcp::LlmContent::Image { mime_type, .. } => format!("[image {mime_type}]"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = if is_error {
        AgentToolResult::error(text)
    } else {
        AgentToolResult::text(text)
    };
    if let Some(structured) = result.structured_content {
        out.structured_content = Some(structured);
    }
    out.details = serde_json::to_value(&result.content).unwrap_or(Value::Null);
    out
}
