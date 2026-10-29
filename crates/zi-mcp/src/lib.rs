//! MCP client for zi: JSON-RPC protocol, stdio and streamable HTTP transports, tool calls.

mod client;
mod error;
mod protocol;
mod transport;

#[cfg(feature = "oauth")]
pub mod oauth;

pub use client::{ConnectionState, McpClient, McpClientOptions};
pub use error::{json_rpc_error_codes, McpError};
pub use protocol::{
    CallToolResult, ClientCapabilities, ContentBlock, Implementation, InitializeParams,
    InitializeResult, JsonRpcErrorObject, JsonRpcId, JsonRpcMessage, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, LlmContent, ServerCapabilities, Tool, to_llm_content,
    internal_error, json_rpc_error_from_object, json_rpc_error_name, response_id,
    LATEST_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
pub use transport::{
    StdioTransport, StdioTransportOptions, StreamableHttpTransport, StreamableHttpTransportOptions,
    Transport, TransportHandle, DEFAULT_MAX_MESSAGE_BYTES,
};
pub use reqwest::Url;
