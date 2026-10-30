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
    /// Tools available to mount on demand (name must appear in config.deferred_tool_names).
    pub deferred_tools: Vec<Arc<dyn AgentTool>>,
    /// Parent-session cheap-search tool calls since the last real user message.
    pub explore_calls_since_user: usize,
}

impl std::fmt::Debug for AgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContext")
            .field("messages", &self.messages)
            .field("tools", &format_args!("{} tool(s)", self.tools.len()))
            .field(
                "deferred_tools",
                &format_args!("{} deferred", self.deferred_tools.len()),
            )
            .field("explore_calls_since_user", &self.explore_calls_since_user)
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
    /// Estimated context size before a model request (after folding).
    ContextUsage {
        estimated_tokens: u64,
        context_window: u32,
        /// Effective auto-compact trigger (`min(window×ratio, absolute)`).
        compact_limit: u64,
        folded_tool_results: usize,
        cumulative_input: u64,
        cumulative_output: u64,
        /// True when estimated tokens crossed the effective compact limit.
        near_limit: bool,
    },
    /// In-loop structural compaction truncated the in-memory transcript.
    Compaction {
        summary: String,
        tokens_before: u64,
        tokens_after: u64,
        dropped_messages: usize,
        /// Non-system messages kept after the summary (re-append to session after the marker).
        #[serde(default)]
        kept_messages: Vec<AgentMessage>,
    },
}

/// Configuration for one agent loop run.
#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub stream_options: SimpleStreamOptions,
    pub tool_execution: ToolExecutionMode,
    /// Token budget for recent tool results kept in full when building the LLM request.
    pub keep_recent_tool_result_tokens: u64,
    /// Fraction of `model.context_window` that contributes to the compact limit.
    pub compact_threshold_ratio: f32,
    /// Absolute token ceiling for auto-compact (0 disables the absolute cap).
    pub compact_absolute_token_limit: u64,
    /// When true, LLM/structural compact the in-memory transcript near the limit.
    pub auto_compact: bool,
    /// Token budget for trailing messages kept after auto/manual compaction.
    pub keep_recent_message_tokens: u64,
    /// Optional hard cap on assistant turns (used by explore/subagent loops).
    pub max_turns: Option<usize>,
    /// Directory for transcript archives written on compact (None disables).
    pub transcript_archive_dir: Option<std::path::PathBuf>,
    /// Tools held back until first requested (e.g. explore); merged into `tools` on demand.
    pub deferred_tool_names: Vec<String>,
    /// Soft warn after this many parent ls/find/grep calls since last user message.
    /// `0` disables the warn hint. (`read` is not counted.)
    pub explore_budget_warn: usize,
    /// Hard-block parent ls/find/grep after this many calls since last user message.
    /// `0` disables the hard block (used by explore subagents). (`read` is not counted.)
    pub explore_budget_block: usize,
}

impl AgentLoopConfig {
    /// Build config with default stream options for the given model.
    pub fn new(model: Model) -> Self {
        Self {
            model,
            stream_options: SimpleStreamOptions::default(),
            tool_execution: ToolExecutionMode::Parallel,
            keep_recent_tool_result_tokens:
                crate::context_budget::DEFAULT_KEEP_RECENT_TOOL_RESULT_TOKENS,
            compact_threshold_ratio: crate::context_budget::DEFAULT_COMPACT_THRESHOLD_RATIO,
            compact_absolute_token_limit: crate::context_budget::DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT,
            auto_compact: true,
            keep_recent_message_tokens: crate::context_budget::DEFAULT_KEEP_RECENT_MESSAGE_TOKENS,
            max_turns: None,
            transcript_archive_dir: None,
            deferred_tool_names: Vec::new(),
            explore_budget_warn: 10,
            explore_budget_block: 16,
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
