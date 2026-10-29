//! MCP client: initialize, list tools, call tools.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::error::{json_rpc_error_codes, McpError};
use crate::protocol::{
    json_rpc_error_from_object, response_id, CallToolResult, ClientCapabilities, Implementation,
    InitializeParams, InitializeResult, JsonRpcId, JsonRpcMessage, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, ListToolsResult, Tool, LATEST_PROTOCOL_VERSION,
    SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::transport::TransportHandle;

const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30_000;
const MAX_LIST_PAGES: usize = 1_000;

#[derive(Debug, Clone)]
pub struct McpClientOptions {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
    pub capabilities: ClientCapabilities,
    pub protocol_version: String,
    pub request_timeout_ms: u64,
}

impl McpClientOptions {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            title: None,
            capabilities: ClientCapabilities::default(),
            protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
        }
    }

    fn client_info(&self) -> Implementation {
        Implementation {
            name: self.name.clone(),
            version: self.version.clone(),
            title: self.title.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Closed,
}

pub struct McpClient {
    options: McpClientOptions,
    state: Mutex<ConnectionState>,
    transport: Mutex<Option<Arc<dyn TransportHandle>>>,
    next_request_id: AtomicU64,
    server_info: Mutex<Option<Implementation>>,
    server_capabilities: Mutex<Option<crate::protocol::ServerCapabilities>>,
    instructions: Mutex<Option<String>>,
    protocol_version: Mutex<Option<String>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, McpError>>>>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
}

impl McpClient {
    pub fn new(options: McpClientOptions) -> Self {
        Self {
            options,
            state: Mutex::new(ConnectionState::Idle),
            transport: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            server_info: Mutex::new(None),
            server_capabilities: Mutex::new(None),
            instructions: Mutex::new(None),
            protocol_version: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            reader_task: Mutex::new(None),
        }
    }

    pub async fn connection_state(&self) -> ConnectionState {
        *self.state.lock().await
    }

    pub async fn server_info(&self) -> Option<Implementation> {
        self.server_info.lock().await.clone()
    }

    pub async fn instructions(&self) -> Option<String> {
        self.instructions.lock().await.clone()
    }

    pub async fn protocol_version(&self) -> Option<String> {
        self.protocol_version.lock().await.clone()
    }

    /// Open a transport and connect.
    pub async fn connect_with<T: crate::transport::Transport>(
        self: &Arc<Self>,
        transport: T,
    ) -> Result<InitializeResult, McpError> {
        let handle = transport.open().await?;
        self.connect(Arc::from(handle)).await
    }

    /// Connect over an opened transport handle (after `Transport::open`).
    pub async fn connect(self: &Arc<Self>, transport: Arc<dyn TransportHandle>) -> Result<InitializeResult, McpError> {
        {
            let mut state = self.state.lock().await;
            if *state != ConnectionState::Idle {
                return Err(McpError::Other(format!("cannot connect in {state:?} state")));
            }
            *state = ConnectionState::Connecting;
        }

        *self.transport.lock().await = Some(transport.clone());

        let client = Arc::clone(self);
        let reader = tokio::spawn(async move {
            loop {
                match transport.recv().await {
                    Ok(Some(message)) => client.dispatch_message(message).await,
                    Ok(None) => break,
                    Err(err) => {
                        client.fail_pending(err.clone()).await;
                        break;
                    }
                }
            }
            client.mark_closed().await;
        });
        *self.reader_task.lock().await = Some(reader);

        let init_result = match self.initialize_session().await {
            Ok(result) => result,
            Err(err) => {
                let _ = self.close().await;
                return Err(err);
            }
        };

        *self.state.lock().await = ConnectionState::Connected;
        Ok(init_result)
    }

    async fn initialize_session(&self) -> Result<InitializeResult, McpError> {
        let params = InitializeParams {
            protocol_version: self.options.protocol_version.clone(),
            capabilities: self.options.capabilities.clone(),
            client_info: self.options.client_info(),
        };
        let params = serde_json::to_value(params).map_err(McpError::from)?;
        let result = self
            .request("initialize", Some(params), self.options.request_timeout_ms)
            .await?;
        let init: InitializeResult = serde_json::from_value(result).map_err(|e| {
            McpError::invalid_request(format!("invalid MCP initialize result: {e}"))
        })?;

        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&init.protocol_version.as_str()) {
            return Err(McpError::Other(format!(
                "MCP server selected unsupported protocol version {}",
                init.protocol_version
            )));
        }

        *self.protocol_version.lock().await = Some(init.protocol_version.clone());
        *self.server_info.lock().await = Some(init.server_info.clone());
        *self.server_capabilities.lock().await = Some(init.capabilities.clone());
        *self.instructions.lock().await = init.instructions.clone();

        if let Some(transport) = self.transport.lock().await.as_ref() {
            transport.set_protocol_version(&init.protocol_version);
        }

        self.notify("notifications/initialized", None).await?;
        Ok(init)
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpError> {
        self.list_all("tools/list", "tools").await
    }

    pub async fn call_tool(
        &self,
        name: &str,
        args: Option<Value>,
    ) -> Result<CallToolResult, McpError> {
        let mut params = json!({ "name": name });
        if let Some(arguments) = args {
            params["arguments"] = arguments;
        }
        let result = self
            .request("tools/call", Some(params), self.options.request_timeout_ms)
            .await?;
        normalize_call_tool_result(result)
    }

    pub async fn close(&self) -> Result<(), McpError> {
        if *self.state.lock().await == ConnectionState::Closed {
            return Ok(());
        }
        self.mark_closed().await;
        if let Some(transport) = self.transport.lock().await.take() {
            transport.close().await?;
        }
        if let Some(task) = self.reader_task.lock().await.take() {
            task.abort();
        }
        Ok(())
    }

    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout_ms: u64,
    ) -> Result<Value, McpError> {
        let transport = self
            .transport
            .lock()
            .await
            .clone()
            .ok_or(McpError::ConnectionClosed)?;

        let id_num = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let id = JsonRpcId::Number(id_num as i64);
        let request = JsonRpcRequest::new(id.clone(), method, params);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(id_key(&id), tx);

        transport
            .send(&JsonRpcMessage::Request(request))
            .await?;

        let result = tokio::time::timeout(Duration::from_millis(timeout_ms), rx)
            .await
            .map_err(|_| McpError::Timeout { timeout_ms })?;

        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err),
            Err(_) => {
                self.pending.lock().await.remove(&id_key(&id));
                Err(McpError::ConnectionClosed)
            }
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let transport = self
            .transport
            .lock()
            .await
            .clone()
            .ok_or(McpError::ConnectionClosed)?;
        transport
            .send(&JsonRpcMessage::Notification(JsonRpcNotification::new(
                method, params,
            )))
            .await
    }

    async fn list_all(&self, method: &str, _key: &str) -> Result<Vec<Tool>, McpError> {
        let mut items: Vec<Tool> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();

        for _ in 0..MAX_LIST_PAGES {
            let mut params = Map::new();
            if let Some(c) = cursor.clone() {
                params.insert("cursor".into(), Value::String(c));
            }
            let page_value = self
                .request(
                    method,
                    if params.is_empty() {
                        None
                    } else {
                        Some(Value::Object(params))
                    },
                    self.options.request_timeout_ms,
                )
                .await?;
            let page: ListToolsResult = serde_json::from_value(page_value).map_err(|e| {
                McpError::invalid_request(format!("invalid MCP {method} result: {e}"))
            })?;
            items.extend(page.tools);
            match page.next_cursor {
                None => break,
                Some(c) if c.is_empty() => break,
                Some(c) => {
                    if !seen.insert(c.clone()) {
                        return Err(McpError::Other(format!(
                            "MCP {method} returned duplicate cursor: {c}"
                        )));
                    }
                    cursor = Some(c);
                }
            }
        }
        Ok(items)
    }

    async fn dispatch_message(&self, message: JsonRpcMessage) {
        match message {
            JsonRpcMessage::Response(response) => {
                let key = id_key(response_id(&response));
                match response {
                    JsonRpcResponse::Success(success) => {
                        if let Some(tx) = self.pending.lock().await.remove(&key) {
                            let _ = tx.send(Ok(success.result));
                        }
                    }
                    JsonRpcResponse::Error(error) => {
                        if let Some(tx) = self.pending.lock().await.remove(&key) {
                            let _ = tx.send(Err(json_rpc_error_from_object(&error.error)));
                        }
                    }
                }
            }
            JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => {
                // Phase 1: server-initiated requests/notifications are ignored except progress.
            }
        }
    }

    async fn fail_pending(&self, err: McpError) {
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(err.clone()));
        }
    }

    async fn mark_closed(&self) {
        *self.state.lock().await = ConnectionState::Closed;
        self.fail_pending(McpError::ConnectionClosed).await;
    }
}

fn id_key(id: &JsonRpcId) -> String {
    match id {
        JsonRpcId::Number(n) => n.to_string(),
        JsonRpcId::String(s) => s.clone(),
    }
}

fn normalize_call_tool_result(value: Value) -> Result<CallToolResult, McpError> {
    let result: CallToolResult = serde_json::from_value(value).map_err(|e| {
        McpError::json_rpc(
            json_rpc_error_codes::INVALID_REQUEST,
            format!("invalid MCP tools/call result: {e}"),
            None,
        )
    })?;
    if result.content.is_empty() && result.structured_content.is_none() {
        // Allow empty object from minimal servers.
    }
    Ok(result)
}
