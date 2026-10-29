//! MCP and JSON-RPC error types.

use serde_json::Value;

/// Standard JSON-RPC 2.0 error codes used by MCP.
pub mod json_rpc_error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("JSON-RPC {code}: {message}")]
    JsonRpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("MCP connection closed")]
    ConnectionClosed,
    #[error("MCP request timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("MCP request aborted")]
    Aborted,
    #[error("invalid MCP message: {0}")]
    InvalidMessage(String),
    #[error("HTTP {status}: {message}")]
    Http {
        status: u16,
        message: String,
        body: String,
    },
    #[error("MCP server requires authentication")]
    AuthRequired { www_authenticate: Option<String> },
    #[error("MCP session expired")]
    SessionExpired,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error("{0}")]
    Other(String),
}

impl Clone for McpError {
    fn clone(&self) -> Self {
        match self {
            Self::JsonRpc { code, message, data } => Self::JsonRpc {
                code: *code,
                message: message.clone(),
                data: data.clone(),
            },
            Self::ConnectionClosed => Self::ConnectionClosed,
            Self::Timeout { timeout_ms } => Self::Timeout {
                timeout_ms: *timeout_ms,
            },
            Self::Aborted => Self::Aborted,
            Self::InvalidMessage(message) => Self::InvalidMessage(message.clone()),
            Self::Http {
                status,
                message,
                body,
            } => Self::Http {
                status: *status,
                message: message.clone(),
                body: body.clone(),
            },
            Self::AuthRequired { www_authenticate } => Self::AuthRequired {
                www_authenticate: www_authenticate.clone(),
            },
            Self::SessionExpired => Self::SessionExpired,
            Self::Io(err) => Self::Other(err.to_string()),
            Self::Json(err) => Self::Other(err.to_string()),
            Self::Reqwest(err) => Self::Other(err.to_string()),
            Self::Other(message) => Self::Other(message.clone()),
        }
    }
}

impl McpError {
    pub fn json_rpc(code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self::JsonRpc {
            code,
            message: message.into(),
            data,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::json_rpc(json_rpc_error_codes::INVALID_REQUEST, message, None)
    }
}
