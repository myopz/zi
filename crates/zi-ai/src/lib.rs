//! LLM provider layer for zi (aligned with pi `packages/ai`).

pub mod api;
pub mod auth;
pub mod catalog;
pub mod cost;
pub mod error;
pub mod event_stream;
pub mod sse;
pub mod stream;
pub mod streaming_json;
pub mod transcript;
pub mod types;

pub use auth::{
    find_env_keys, get_env_api_key, require_api_key, ProviderEnv, ANTHROPIC_API_KEY_ENV,
    DEEPSEEK_API_KEY_ENV, GEMINI_API_KEY_ENV, GLM_API_KEY_ENV, GOOGLE_API_KEY_ENV,
    OPENAI_API_KEY_ENV, OPENROUTER_API_KEY_ENV, ZAI_API_KEY_ENV,
};
pub use catalog::{
    all_models, claude_sonnet_4, deepseek_flash, find_model, glm_4_7_flash, glm_4v_flash, gpt_4_1,
    openrouter_auto,
};
pub use error::ZiAiError;
pub use event_stream::{
    create_assistant_message_event_stream, AssistantMessageEventStream, EventStream,
};
pub use stream::{stream_simple, stream_transcript};
pub use types::{
    new_pending_assistant, AssistantContentBlock, AssistantMessage, AssistantMessageEvent,
    AssistantRole, Context, DoneStopReason, ErrorStopReason, ImageContent, KnownApi, Message, Model,
    ModelCost, SimpleStreamOptions, StopReason, StreamOptions, SystemContent, SystemMessage,
    SystemRole, TextContent, TextContentKind, ThinkingContent, Tool, ToolCall, ToolChoice,
    ToolReference, ToolResultContentBlock, ToolResultMessage, ToolResultRole, TranscriptContext,
    Usage, UserContent, UserContentBlock, UserMessage, UserRole,
};
