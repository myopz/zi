//! JSON-RPC 2.0 message types for MCP.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{json_rpc_error_codes, McpError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcSuccessResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub error: JsonRpcErrorObject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success(JsonRpcSuccessResponse),
    Error(JsonRpcErrorResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl JsonRpcRequest {
    pub fn new(id: JsonRpcId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// Parse a JSON value into a JSON-RPC message.
pub fn parse_json_rpc_message(value: Value) -> Result<JsonRpcMessage, McpError> {
    if let Ok(request) = serde_json::from_value::<JsonRpcRequest>(value.clone()) {
        if request.jsonrpc == "2.0" {
            return Ok(JsonRpcMessage::Request(request));
        }
    }
    if let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(value.clone()) {
        if notification.jsonrpc == "2.0" {
            return Ok(JsonRpcMessage::Notification(notification));
        }
    }
    if let Ok(success) = serde_json::from_value::<JsonRpcSuccessResponse>(value.clone()) {
        if success.jsonrpc == "2.0" {
            return Ok(JsonRpcMessage::Response(JsonRpcResponse::Success(success)));
        }
    }
    if let Ok(error) = serde_json::from_value::<JsonRpcErrorResponse>(value) {
        if error.jsonrpc == "2.0" {
            return Ok(JsonRpcMessage::Response(JsonRpcResponse::Error(error)));
        }
    }
    let err = internal_error("invalid JSON-RPC message");
    Err(json_rpc_error_from_object(&err))
}

pub fn response_id(message: &JsonRpcResponse) -> &JsonRpcId {
    match message {
        JsonRpcResponse::Success(r) => &r.id,
        JsonRpcResponse::Error(r) => &r.id,
    }
}

pub fn json_rpc_error_from_object(error: &JsonRpcErrorObject) -> McpError {
    let _ = json_rpc_error_name(error.code);
    McpError::JsonRpc {
        code: error.code,
        message: error.message.clone(),
        data: error.data.clone(),
    }
}

pub fn internal_error(message: impl Into<String>) -> JsonRpcErrorObject {
    JsonRpcErrorObject {
        code: json_rpc_error_codes::INTERNAL_ERROR,
        message: message.into(),
        data: None,
    }
}

/// Human-readable name for a standard JSON-RPC error code, when known.
pub fn json_rpc_error_name(code: i64) -> Option<&'static str> {
    match code {
        json_rpc_error_codes::PARSE_ERROR => Some("parse_error"),
        json_rpc_error_codes::INVALID_REQUEST => Some("invalid_request"),
        json_rpc_error_codes::METHOD_NOT_FOUND => Some("method_not_found"),
        json_rpc_error_codes::INVALID_PARAMS => Some("invalid_params"),
        json_rpc_error_codes::INTERNAL_ERROR => Some("internal_error"),
        _ => None,
    }
}
