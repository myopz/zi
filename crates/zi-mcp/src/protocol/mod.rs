//! MCP wire protocol types.

pub mod content;
pub mod jsonrpc;
pub mod types;

pub use content::{CallToolResult, ContentBlock, LlmContent, to_llm_content};
pub use jsonrpc::{
    internal_error, json_rpc_error_from_object, json_rpc_error_name, parse_json_rpc_message,
    response_id, JsonRpcErrorObject, JsonRpcId, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse,
};
pub use types::{
    ClientCapabilities, Implementation, InitializeParams, InitializeResult, ListToolsResult,
    ServerCapabilities, Tool, LATEST_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
