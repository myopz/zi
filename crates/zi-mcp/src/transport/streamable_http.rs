//! Streamable HTTP MCP transport (POST JSON-RPC; optional SSE).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client, Url};
use tokio::sync::Mutex;

use crate::error::McpError;
use crate::protocol::{parse_json_rpc_message, JsonRpcId, JsonRpcMessage, JsonRpcResponse};
use crate::transport::transport::{Transport, TransportHandle, DEFAULT_MAX_MESSAGE_BYTES};

const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct StreamableHttpTransportOptions {
    pub url: Url,
    pub headers: HashMap<String, String>,
    pub open_get_stream: bool,
    pub max_message_bytes: usize,
}

impl StreamableHttpTransportOptions {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            headers: HashMap::new(),
            open_get_stream: true,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

pub struct StreamableHttpTransport {
    options: StreamableHttpTransportOptions,
}

impl StreamableHttpTransport {
    pub fn new(options: StreamableHttpTransportOptions) -> Self {
        Self { options }
    }
}

#[async_trait]
impl Transport for StreamableHttpTransport {
    async fn open(self) -> Result<Box<dyn TransportHandle>, McpError> {
        let client = Client::builder().build()?;
        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel(64);
        let closed = Arc::new(AtomicBool::new(false));

        let handle = StreamableHttpTransportHandle {
            client,
            url: self.options.url,
            extra_headers: self.options.headers,
            open_get_stream: self.options.open_get_stream,
            max_message_bytes: self.options.max_message_bytes,
            session_id: Arc::new(StdMutex::new(None)),
            protocol_version: Arc::new(StdMutex::new(None)),
            inbound: Mutex::new(inbound_rx),
            inbound_tx,
            closed,
            get_stream_started: AtomicBool::new(false),
        };

        Ok(Box::new(handle))
    }
}

struct StreamableHttpTransportHandle {
    client: Client,
    url: Url,
    extra_headers: HashMap<String, String>,
    open_get_stream: bool,
    max_message_bytes: usize,
    session_id: Arc<StdMutex<Option<String>>>,
    protocol_version: Arc<StdMutex<Option<String>>>,
    inbound: Mutex<tokio::sync::mpsc::Receiver<Result<JsonRpcMessage, McpError>>>,
    inbound_tx: tokio::sync::mpsc::Sender<Result<JsonRpcMessage, McpError>>,
    closed: Arc<AtomicBool>,
    get_stream_started: AtomicBool,
}

#[async_trait]
impl TransportHandle for StreamableHttpTransportHandle {
    async fn send(&self, message: &JsonRpcMessage) -> Result<(), McpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(McpError::ConnectionClosed);
        }

        let body = serde_json::to_string(message)?;
        let response = self
            .post_with_headers(body, accept_json_and_sse(), self.build_headers(None))
            .await?;

        self.capture_session(&response);
        if !response.status().is_success() {
            let has_session = self
                .session_id
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
                .is_some();
            return Err(http_error_from_response(response, has_session).await);
        }

        let is_request = matches!(message, JsonRpcMessage::Request(_));
        let request_id = match message {
            JsonRpcMessage::Request(r) => Some(r.id.clone()),
            _ => None,
        };

        let status = response.status().as_u16();
        if is_request && (status == 202 || status == 204) {
            return Err(McpError::Http {
                status,
                message: "server accepted request without a response".into(),
                body: String::new(),
            });
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase());

        match content_type.as_deref() {
            Some("application/json") => {
                let value: serde_json::Value = response.json().await?;
                self.dispatch_json_value(value).await?;
            }
            Some("text/event-stream") => {
                let stream = response.bytes_stream();
                let tx = self.inbound_tx.clone();
                let max_bytes = self.max_message_bytes;
                let closed = self.closed.clone();
                let expected_id = request_id;
                tokio::spawn(async move {
                    if let Err(err) =
                        consume_sse_stream(stream, max_bytes, tx.clone(), expected_id, closed).await
                    {
                        let _ = tx.send(Err(err)).await;
                    }
                });
            }
            _ if !is_request => {
                // Notifications/responses may return 202 with empty body.
            }
            other => {
                let body = truncate_body(response.text().await.unwrap_or_default());
                return Err(McpError::Http {
                    status,
                    message: format!("unsupported response content type: {}", other.unwrap_or("missing")),
                    body,
                });
            }
        }

        if let JsonRpcMessage::Notification(n) = message {
            if n.method == "notifications/initialized" && self.open_get_stream {
                self.maybe_start_get_stream();
            }
        }

        Ok(())
    }

    async fn recv(&self) -> Result<Option<JsonRpcMessage>, McpError> {
        let mut inbound = self.inbound.lock().await;
        loop {
            match inbound.recv().await {
                Some(Ok(message)) => return Ok(Some(message)),
                Some(Err(err)) => return Err(err),
                None => return Ok(None),
            }
        }
    }

    async fn close(&self) -> Result<(), McpError> {
        self.closed.store(true, Ordering::SeqCst);
        let session_id = self
            .session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(session_id) = session_id {
            let mut headers = self.build_headers(None);
            headers.insert(
                HeaderName::from_static("mcp-session-id"),
                HeaderValue::from_str(&session_id).unwrap_or(HeaderValue::from_static("")),
            );
            let _ = self
                .client
                .delete(self.url.clone())
                .headers(headers)
                .send()
                .await;
        }
        Ok(())
    }

    fn set_protocol_version(&self, version: &str) {
        if let Ok(mut guard) = self.protocol_version.lock() {
            *guard = Some(version.to_string());
        }
    }
}

impl StreamableHttpTransportHandle {
    fn build_headers(&self, extra: Option<HeaderMap>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.extra_headers {
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        if let Some(extra) = extra {
            headers.extend(extra);
        }
        if let Ok(guard) = self.session_id.lock() {
            if let Some(session) = guard.clone() {
                if let Ok(v) = HeaderValue::from_str(&session) {
                    headers.insert(HeaderName::from_static("mcp-session-id"), v);
                }
            }
        }
        if let Ok(guard) = self.protocol_version.lock() {
            if let Some(version) = guard.clone() {
                if let Ok(v) = HeaderValue::from_str(&version) {
                    headers.insert(HeaderName::from_static("mcp-protocol-version"), v);
                }
            }
        }
        headers
    }

    async fn post_with_headers(
        &self,
        body: String,
        accept: HeaderMap,
        mut headers: HeaderMap,
    ) -> Result<reqwest::Response, McpError> {
        headers.extend(accept);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(self
            .client
            .post(self.url.clone())
            .headers(headers)
            .body(body)
            .send()
            .await?)
    }

    fn capture_session(&self, response: &reqwest::Response) {
        if let Some(session) = response.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            if let Ok(mut guard) = self.session_id.lock() {
                *guard = Some(session.to_string());
            }
        }
    }

    async fn dispatch_json_value(&self, value: serde_json::Value) -> Result<(), McpError> {
        let items = match value {
            serde_json::Value::Array(arr) => arr,
            one => vec![one],
        };
        for item in items {
            let message = parse_json_rpc_message(item)?;
            let _ = self.inbound_tx.send(Ok(message)).await;
        }
        Ok(())
    }

    fn maybe_start_get_stream(&self) {
        if self.get_stream_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = StreamableHttpGetStream {
            client: self.client.clone(),
            url: self.url.clone(),
            build_headers: self.extra_headers.clone(),
            session_id: Arc::clone(&self.session_id),
            protocol_version: Arc::clone(&self.protocol_version),
            inbound_tx: self.inbound_tx.clone(),
            closed: self.closed.clone(),
            max_message_bytes: self.max_message_bytes,
        };
        tokio::spawn(async move {
            let inbound_tx = this.inbound_tx.clone();
            if let Err(err) = this.run().await {
                let _ = inbound_tx.send(Err(err)).await;
            }
        });
    }
}

struct StreamableHttpGetStream {
    client: Client,
    url: Url,
    build_headers: HashMap<String, String>,
    session_id: Arc<StdMutex<Option<String>>>,
    protocol_version: Arc<StdMutex<Option<String>>>,
    inbound_tx: tokio::sync::mpsc::Sender<Result<JsonRpcMessage, McpError>>,
    closed: Arc<AtomicBool>,
    max_message_bytes: usize,
}

impl StreamableHttpGetStream {
    async fn run(self) -> Result<(), McpError> {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.build_headers {
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        if let Ok(guard) = self.session_id.lock() {
            if let Some(session) = guard.clone() {
                if let Ok(v) = HeaderValue::from_str(&session) {
                    headers.insert(HeaderName::from_static("mcp-session-id"), v);
                }
            }
        }
        if let Ok(guard) = self.protocol_version.lock() {
            if let Some(version) = guard.clone() {
                if let Ok(v) = HeaderValue::from_str(&version) {
                    headers.insert(HeaderName::from_static("mcp-protocol-version"), v);
                }
            }
        }

        let response = self.client.get(self.url.clone()).headers(headers).send().await?;
        if response.status().as_u16() == 405 {
            return Ok(());
        }
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = truncate_body(response.text().await.unwrap_or_default());
            return Err(McpError::Http {
                status,
                message: "MCP GET stream failed".into(),
                body,
            });
        }
        let stream = response.bytes_stream();
        consume_sse_stream(stream, self.max_message_bytes, self.inbound_tx, None, self.closed).await
    }
}

fn accept_json_and_sse() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json, text/event-stream"));
    headers
}

fn truncate_body(body: String) -> String {
    body.chars().take(MAX_ERROR_BODY_BYTES).collect()
}

async fn http_error_from_response(response: reqwest::Response, has_session: bool) -> McpError {
    let status_code = response.status().as_u16();
    let www_authenticate = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = truncate_body(response.text().await.unwrap_or_default());
    if status_code == 401 {
        return McpError::AuthRequired { www_authenticate };
    }
    if status_code == 404 && has_session {
        return McpError::SessionExpired;
    }
    McpError::Http {
        status: status_code,
        message: format!("MCP HTTP request failed with status {status_code}"),
        body,
    }
}

async fn consume_sse_stream(
    stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    max_event_bytes: usize,
    inbound_tx: tokio::sync::mpsc::Sender<Result<JsonRpcMessage, McpError>>,
    expected_response_id: Option<JsonRpcId>,
    closed: Arc<AtomicBool>,
) -> Result<(), McpError> {
    let mut stream = std::pin::pin!(stream);
    let mut buffered = String::new();
    let mut data_lines: Vec<String> = Vec::new();
    let mut data_bytes = 0usize;

    let dispatch = |data_lines: &mut Vec<String>,
                    data_bytes: &mut usize,
                    inbound_tx: &tokio::sync::mpsc::Sender<Result<JsonRpcMessage, McpError>>|
     -> Result<(), McpError> {
        if data_lines.is_empty() {
            return Ok(());
        }
        let data = data_lines.join("\n");
        data_lines.clear();
        *data_bytes = 0;
        if data.trim().is_empty() {
            return Ok(());
        }
        let value: serde_json::Value = serde_json::from_str(&data)?;
        let message = parse_json_rpc_message(value)?;
        if let (Some(expected), JsonRpcMessage::Response(JsonRpcResponse::Success(r))) =
            (&expected_response_id, &message)
        {
            if &r.id != expected {
                // Still forward; client matches pending ids.
            }
        }
        let _ = inbound_tx.try_send(Ok(message));
        Ok(())
    };

    while let Some(chunk) = stream.next().await {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        let chunk = chunk?;
        buffered.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = buffered.find('\n') {
            let line = buffered[..newline].trim_end_matches('\r').to_string();
            buffered = buffered[newline + 1..].to_string();
            if line.is_empty() {
                dispatch(&mut data_lines, &mut data_bytes, &inbound_tx)?;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            if field == "data" {
                data_bytes += value.len() + if data_lines.is_empty() { 0 } else { 1 };
                if data_bytes > max_event_bytes {
                    return Err(McpError::Other(format!(
                        "MCP SSE event exceeds {max_event_bytes} bytes"
                    )));
                }
                data_lines.push(value.to_string());
            }
        }
        if buffered.len() > max_event_bytes {
            return Err(McpError::Other(format!(
                "MCP SSE event exceeds {max_event_bytes} bytes"
            )));
        }
    }
    if !buffered.is_empty() {
        let line = buffered.trim_end_matches('\r');
        if !line.is_empty() && !line.starts_with(':') {
            if let Some((field, value)) = line.split_once(':') {
                if field == "data" {
                    data_lines.push(value.strip_prefix(' ').unwrap_or(value).to_string());
                }
            }
        }
    }
    dispatch(&mut data_lines, &mut data_bytes, &inbound_tx)?;
    Ok(())
}
