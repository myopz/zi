//! Transport trait: outbound send and inbound recv.

use async_trait::async_trait;

use crate::error::McpError;
use crate::protocol::JsonRpcMessage;

pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Bidirectional MCP transport (stdio, streamable HTTP, etc.).
#[async_trait]
pub trait Transport: Send + Sync {
    /// Start I/O and return a handle for send/recv.
    async fn open(self) -> Result<Box<dyn TransportHandle>, McpError>
    where
        Self: Sized;
}

/// Active transport connection.
#[async_trait]
pub trait TransportHandle: Send + Sync {
    async fn send(&self, message: &JsonRpcMessage) -> Result<(), McpError>;

    /// Next inbound message, or `None` when the transport closed.
    async fn recv(&self) -> Result<Option<JsonRpcMessage>, McpError>;

    async fn close(&self) -> Result<(), McpError>;

    fn set_protocol_version(&self, _version: &str) {}
}
