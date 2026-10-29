// Async client for the zi-env framed protocol.

use crate::error::{EnvError, EnvResult};
use crate::protocol::frame::{decode_frame, Frame, ERROR, EVENT, PING, REQUEST, RESULT};
use crate::protocol::SYNC_PREFIX;
use bytes::BytesMut;
use zi_durable::FileError;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

const REQUEST_TIMEOUT_MS: u64 = 60_000;

#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub protocol: u64,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub home: String,
    pub cwd: String,
    pub features: Vec<String>,
}

pub struct Reply {
    pub json: Value,
    pub payload: Vec<u8>,
}

struct Pending {
    sender: oneshot::Sender<EnvResult<Reply>>,
}

/// Connection to one zi-envd process session.
pub struct Connection {
    _child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<u32, Pending>>>,
    next_id: AtomicU32,
    info: RemoteInfo,
}

impl Connection {
    /// Start zi-envd (or a custom command prefix) and complete `hello`.
    pub async fn start(command: &[String]) -> EnvResult<Self> {
        let token = hex_token();
        let mut argv = command.to_vec();
        argv.push("serve".into());
        argv.push("--token".into());
        argv.push(token.clone());
        let program = argv
            .first()
            .cloned()
            .ok_or_else(|| EnvError::Other("empty command".into()))?;
        let child_argv: Vec<String> = argv.into_iter().skip(1).collect();
        let mut child = Command::new(program)
            .args(child_argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| EnvError::Process(error.to_string()))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| EnvError::Process("no stdout".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EnvError::Process("no stdin".into()))?;
        let remainder = sync_handshake(&mut stdout, &token).await?;
        let pending: Arc<Mutex<HashMap<u32, Pending>>> = Arc::new(Mutex::new(HashMap::new()));
        let stdin_writer = Arc::new(Mutex::new(stdin));
        read_loop(stdout, pending.clone(), remainder);
        let connection = Self {
            _child: Arc::new(Mutex::new(Some(child))),
            stdin: stdin_writer,
            pending,
            next_id: AtomicU32::new(1),
            info: RemoteInfo {
                protocol: 0,
                version: String::new(),
                os: String::new(),
                arch: String::new(),
                home: String::new(),
                cwd: String::new(),
                features: Vec::new(),
            },
        };
        let hello = connection.request_op("hello", json!({ "protocol": 1 }), None).await?;
        let info = parse_hello(&hello.json)?;
        Ok(Self { info, ..connection })
    }

    pub fn info(&self) -> &RemoteInfo {
        &self.info
    }

    pub async fn request_op(
        &self,
        op: &str,
        mut json: Value,
        payload: Option<Vec<u8>>,
    ) -> EnvResult<Reply> {
        if let Some(map) = json.as_object_mut() {
            map.insert("op".into(), Value::String(op.into()));
        }
        self.request(json, payload).await
    }

    pub async fn request(&self, json: Value, payload: Option<Vec<u8>>) -> EnvResult<Reply> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, Pending { sender: tx });
        let frame = Frame::with_payload(REQUEST, id, json, payload.unwrap_or_default());
        self.stdin
            .lock()
            .await
            .write_all(&frame.encode())
            .await?;
        self.stdin.lock().await.flush().await?;
        match tokio::time::timeout(
            std::time::Duration::from_millis(REQUEST_TIMEOUT_MS),
            rx,
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(EnvError::ConnectionLost),
            Err(_) => Err(EnvError::Other("request timeout".into())),
        }
    }
}

fn hex_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032x}")
}

async fn sync_handshake(
    stdout: &mut tokio::process::ChildStdout,
    token: &str,
) -> EnvResult<BytesMut> {
    let expected = format!("{SYNC_PREFIX} {token}\n");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let read = stdout.read(&mut chunk).await?;
        if read == 0 {
            return Err(EnvError::Protocol("daemon exited before sync".into()));
        }
        buf.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buf.windows(expected.len()).position(|window| window == expected.as_bytes()) {
            let remainder = buf.split_off(pos + expected.len());
            return Ok(BytesMut::from(remainder.as_slice()));
        }
        if buf.len() > 1 << 20 {
            return Err(EnvError::Protocol("sync line not found".into()));
        }
    }
}

fn read_loop(
    mut stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<u32, Pending>>>,
    mut buffer: BytesMut,
) {
    tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                Err(_) => break,
            }
            loop {
                match decode_frame(&mut buffer) {
                    Ok(Some(frame)) => handle_frame(frame, &pending).await,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
        let mut guard = pending.lock().await;
        for (_, pending) in guard.drain() {
            let _ = pending.sender.send(Err(EnvError::ConnectionLost));
        }
    });
}

async fn handle_frame(frame: Frame, pending: &Mutex<HashMap<u32, Pending>>) {
    match frame.kind {
        PING | EVENT => {}
        RESULT => {
            if let Some(entry) = pending.lock().await.remove(&frame.id) {
                let _ = entry.sender.send(Ok(Reply {
                    json: frame.json,
                    payload: frame.payload,
                }));
            }
        }
        ERROR => {
            if let Some(entry) = pending.lock().await.remove(&frame.id) {
                let _ = entry.sender.send(Err(remote_error_from_json(frame.json)));
            }
        }
        _ => {}
    }
}

fn remote_error_from_json(json: Value) -> EnvError {
    let code = json
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let message = json
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("remote operation failed")
        .to_string();
    let path = json.get("path").and_then(Value::as_str).map(str::to_string);
    EnvError::Remote(FileError {
        code,
        message,
        path,
    })
}

fn parse_hello(json: &Value) -> EnvResult<RemoteInfo> {
    Ok(RemoteInfo {
        protocol: json.get("protocol").and_then(Value::as_u64).unwrap_or(0),
        version: json
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        os: json.get("os").and_then(Value::as_str).unwrap_or("").to_string(),
        arch: json
            .get("arch")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        home: json
            .get("home")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cwd: json.get("cwd").and_then(Value::as_str).unwrap_or("").to_string(),
        features: json
            .get("features")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}
