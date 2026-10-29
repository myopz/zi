//! Stdio MCP transport: spawn a subprocess and frame JSON-RPC with newlines.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::McpError;
use crate::protocol::{parse_json_rpc_message, JsonRpcMessage};
use crate::transport::transport::{Transport, TransportHandle, DEFAULT_MAX_MESSAGE_BYTES};

#[derive(Debug, Clone)]
pub struct StdioTransportOptions {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub inherit_env: bool,
    pub max_message_bytes: usize,
}

impl StdioTransportOptions {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            inherit_env: true,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

pub struct StdioTransport {
    options: StdioTransportOptions,
}

impl StdioTransport {
    pub fn new(options: StdioTransportOptions) -> Self {
        Self { options }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn open(self) -> Result<Box<dyn TransportHandle>, McpError> {
        let mut command = Command::new(&self.options.command);
        command
            .args(&self.options.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &self.options.cwd {
            command.current_dir(cwd);
        }
        if self.options.inherit_env {
            command.envs(std::env::vars());
        }
        command.envs(&self.options.env);

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Other("stdio transport: missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Other("stdio transport: missing stdout".into()))?;

        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel(64);
        let closed = Arc::new(AtomicBool::new(false));
        let max_bytes = self.options.max_message_bytes;

        let closed_reader = closed.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\r', '\n']);
                        if trimmed.is_empty() {
                            continue;
                        }
                        if trimmed.len() > max_bytes {
                            let _ = inbound_tx
                                .send(Err(McpError::Other(format!(
                                    "MCP stdio message exceeds {max_bytes} bytes"
                                ))))
                                .await;
                            continue;
                        }
                        match serde_json::from_str::<serde_json::Value>(trimmed)
                            .map_err(McpError::from)
                            .and_then(parse_json_rpc_message)
                        {
                            Ok(message) => {
                                if inbound_tx.send(Ok(message)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = inbound_tx.send(Err(err)).await;
                            }
                        }
                    }
                    Err(err) => {
                        let _ = inbound_tx.send(Err(err.into())).await;
                        break;
                    }
                }
            }
            closed_reader.store(true, Ordering::SeqCst);
        });

        let handle = StdioTransportHandle {
            stdin: Mutex::new(stdin),
            inbound: Mutex::new(inbound_rx),
            child: Mutex::new(Some(child)),
            closed,
        };

        Ok(Box::new(handle))
    }
}

struct StdioTransportHandle {
    stdin: Mutex<tokio::process::ChildStdin>,
    inbound: Mutex<tokio::sync::mpsc::Receiver<Result<JsonRpcMessage, McpError>>>,
    child: Mutex<Option<Child>>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl TransportHandle for StdioTransportHandle {
    async fn send(&self, message: &JsonRpcMessage) -> Result<(), McpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(McpError::ConnectionClosed);
        }
        let payload = serde_json::to_string(message)?;
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
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
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        Ok(())
    }
}
