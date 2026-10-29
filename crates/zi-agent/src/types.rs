// Agent loop types: events, tools, context (aligned with pi packages/agent).

use std::sync::Arc;

use async_trait::async_trait;
use zi_ai::{
    AssistantMessageEvent, Message, Model, SimpleStreamOptions, TextContent, TextContentKind,
    ToolResultMessage, Usage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

/// Conversation message in the agent transcript (LLM messages for Phase 2).
pub type AgentMessage = Message;

/// How multiple tool calls from one assistant message are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ToolExecutionMode {
    #[default]
    Parallel,
    Sequential,
}

/// Snapshot passed into the low-level agent loop.
#[derive(Clone, Default)]
pub struct AgentContext {
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<Arc<dyn AgentTool>>,
}

impl std::fmt::Debug for AgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContext")
            .field("messages", &self.messages)
            .field("tools", &format_args!("{} tool(s)", self.tools.len()))
            .finish()
    }
}

/// Model-facing tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolResult {
    pub content: Vec<zi_ai::TextContent>,
    #[serde(default)]
    pub details: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminate: Option<bool>,
}

impl AgentToolResult {
    /// Build a text result for the model.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![TextContent {
                kind: TextContentKind::Text,
                text: text.into(),
                text_signature: None,
            }],
            details: Value::Null,
            structured_content: None,
            usage: None,
            is_error: None,
            terminate: None,
        }
    }

    /// Build an error result shown to the model.
    pub fn error(text: impl Into<String>) -> Self {
        let mut result = Self::text(text);
        result.is_error = Some(true);
        result
    }
}

/// Partial execution update from a tool.
pub type AgentToolUpdateCallback = Box<dyn Fn(AgentToolResult) + Send + Sync>;

/// Executable tool registered on the agent.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        None
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        on_update: Option<AgentToolUpdateCallback>,
    ) -> AgentToolResult;
}

/// Static tool definition backed by an async handler.
pub struct FnAgentTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub mode: Option<ToolExecutionMode>,
    handler: HandlerFn,
}

type HandlerFn = Arc<
    dyn Fn(
            String,
            Value,
            Option<AgentToolUpdateCallback>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AgentToolResult> + Send>>
        + Send
        + Sync,
>;

impl FnAgentTool {
    /// Register a tool implemented by an async closure.
    pub fn new<F, Fut>(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
        handler: F,
    ) -> Arc<Self>
    where
        F: Fn(String, Value, Option<AgentToolUpdateCallback>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = AgentToolResult> + Send + 'static,
    {
        Arc::new(Self {
            name: name.into(),
            description: description.into(),
            parameters,
            mode: None,
            handler: Arc::new(move |id, args, on_update| Box::pin(handler(id, args, on_update))),
        })
    }
}

#[async_trait]
impl AgentTool for FnAgentTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        self.mode
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        on_update: Option<AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        (self.handler)(tool_call_id.to_string(), args, on_update).await
    }
}

/// Lifecycle events for UI and orchestration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd { messages: Vec<AgentMessage> },
    TurnStart,
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart { message: AgentMessage },
    MessageUpdate {
        message: AgentMessage,
        assistant_message_event: AssistantMessageEvent,
    },
    MessageEnd { message: AgentMessage },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: AgentToolResult,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: AgentToolResult,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
}

/// Configuration for one agent loop run.
#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub stream_options: SimpleStreamOptions,
    pub tool_execution: ToolExecutionMode,
}

impl AgentLoopConfig {
    /// Build config with default stream options for the given model.
    pub fn new(model: Model) -> Self {
        Self {
            model,
            stream_options: SimpleStreamOptions::default(),
            tool_execution: ToolExecutionMode::Parallel,
        }
    }
}

/// Outcome of a finalized tool call.
#[derive(Debug, Clone)]
pub struct AgentToolCallOutcome {
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    pub result: AgentToolResult,
    pub is_error: bool,
    pub duration_ms: u64,
}

/// Broadcast sender used by [`crate::Agent`] for subscriptions.
pub type AgentEventSender = broadcast::Sender<AgentEvent>;

/// Subscribe to agent events (`recv()` until lagged or closed).
pub type AgentEventReceiver = broadcast::Receiver<AgentEvent>;
