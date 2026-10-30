//! Agent event loop for zi (aligned with pi `packages/agent`).

mod agent;
mod agent_loop;
mod context_budget;
mod error;
mod stream_fn;
mod summarize_compact;
mod types;

pub use agent::{Agent, AgentOptions, AgentPromptInput};
pub use agent_loop::{run_agent_loop, AgentEventSink};
pub use context_budget::{
    compact_token_limit, estimate_message_tokens, estimate_messages_tokens, fold_old_tool_results,
    should_compact, structural_compact, StructuralCompaction,
    DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT, DEFAULT_COMPACT_THRESHOLD_RATIO,
    DEFAULT_KEEP_RECENT_MESSAGE_TOKENS, DEFAULT_KEEP_RECENT_TOOL_RESULT_TOKENS,
};
pub use summarize_compact::{serialize_for_summary, summarize_compact, SummarizeCompactOptions};
pub use error::AgentError;
pub use stream_fn::{DefaultStreamFn, StreamFn};
pub use types::{
    AgentContext, AgentEvent, AgentEventReceiver, AgentEventSender, AgentLoopConfig, AgentMessage,
    AgentTool, AgentToolCallOutcome, AgentToolResult, AgentToolUpdateCallback, FnAgentTool,
    ToolExecutionMode,
};
