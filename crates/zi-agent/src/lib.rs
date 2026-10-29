//! Agent event loop for zi (aligned with pi `packages/agent`).

mod agent;
mod agent_loop;
mod error;
mod stream_fn;
mod types;

pub use agent::{Agent, AgentOptions, AgentPromptInput};
pub use agent_loop::{run_agent_loop, AgentEventSink};
pub use error::AgentError;
pub use stream_fn::{DefaultStreamFn, StreamFn};
pub use types::{
    AgentContext, AgentEvent, AgentEventReceiver, AgentEventSender, AgentLoopConfig, AgentMessage,
    AgentTool, AgentToolCallOutcome, AgentToolResult, AgentToolUpdateCallback, FnAgentTool,
    ToolExecutionMode,
};
