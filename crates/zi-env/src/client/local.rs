// LocalExecutionEnv: zi-envd child on the same machine.

use crate::client::connection::Connection;
use crate::error::{EnvError, EnvResult};
use zi_durable::{
    ExecutionEnv, FileError, FileKind, FileListEntry, FileListing, ShellExecResult,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

/// Default argv prefix to spawn the bundled `zi-envd` binary from PATH.
pub fn local_daemon_command() -> Vec<String> {
    vec!["zi-envd".into()]
}

pub struct LocalExecutionEnvOptions {
    pub command: Vec<String>,
    pub id: String,
}

impl Default for LocalExecutionEnvOptions {
    fn default() -> Self {
        Self {
            command: local_daemon_command(),
            id: "zi-env:local".into(),
        }
    }
}

/// Execution environment backed by a local zi-envd process.
pub struct LocalExecutionEnv {
    id: String,
    connection: Arc<Mutex<Connection>>,
    runtime: Handle,
}

impl LocalExecutionEnv {
    pub async fn connect(options: LocalExecutionEnvOptions) -> EnvResult<Self> {
        let connection = Connection::start(&options.command).await?;
        Ok(Self {
            id: options.id,
            connection: Arc::new(Mutex::new(connection)),
            runtime: Handle::current(),
        })
    }

    /// Blocking wrapper for non-async callers.
    pub fn connect_blocking(options: LocalExecutionEnvOptions) -> EnvResult<Self> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| EnvError::Other(error.to_string()))?
            .block_on(Self::connect(options))
    }

    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

impl ExecutionEnv for LocalExecutionEnv {
    fn id(&self) -> &str {
        &self.id
    }

    fn exec(&self, command: &str, cwd: Option<&str>) -> zi_durable::EnvResult<ShellExecResult> {
        self.block_on(async {
            let mut json = json!({ "command": command, "inheritEnv": true });
            if let Some(cwd) = cwd {
                json["cwd"] = Value::from(cwd);
            }
            let reply = self
                .connection
                .lock()
                .await
                .request_op("exec", json, None)
                .await
                .map_err(env_to_file)?;
            let exit_code = reply
                .json
                .get("exitCode")
                .and_then(Value::as_i64)
                .unwrap_or(-1) as i32;
            let combined = String::from_utf8_lossy(&reply.payload).into_owned();
            Ok(ShellExecResult {
                exit_code,
                stdout: combined.clone(),
                stderr: String::new(),
            })
        })
    }

    fn read_file(&self, path: &str) -> zi_durable::EnvResult<Vec<u8>> {
        self.block_on(async {
            let reply = self
                .connection
                .lock()
                .await
                .request_op("read", serde_json::json!({ "path": path }), None)
                .await
                .map_err(env_to_file)?;
            Ok(reply.payload)
        })
    }

    fn write_file(&self, path: &str, bytes: &[u8], append: bool) -> zi_durable::EnvResult<()> {
        self.block_on(async {
            self.connection
                .lock()
                .await
                .request_op(
                    "write",
                    serde_json::json!({ "path": path, "append": append }),
                    Some(bytes.to_vec()),
                )
                .await
                .map_err(env_to_file)?;
            Ok(())
        })
    }

    fn list_files(&self, path: &str) -> zi_durable::EnvResult<FileListing> {
        self.block_on(async {
            let reply = self
                .connection
                .lock()
                .await
                .request_op("listFiles", serde_json::json!({ "path": path }), None)
                .await
                .map_err(env_to_file)?;
            Ok(decode_list_payload(&reply.payload, reply.json.get("truncated") == Some(&Value::Bool(true))))
        })
    }

    fn hello(&self) -> zi_durable::EnvResult<Value> {
        self.block_on(async {
            let info = self.connection.lock().await.info().clone();
            Ok(serde_json::json!({
                "protocol": info.protocol,
                "version": info.version,
                "os": info.os,
                "arch": info.arch,
                "cwd": info.cwd,
                "features": info.features,
            }))
        })
    }
}

fn decode_list_payload(payload: &[u8], truncated: bool) -> FileListing {
    let mut entries = Vec::new();
    let mut start = 0usize;
    while start < payload.len() {
        let Some(end) = payload[start..].iter().position(|byte| *byte == 0) else {
            break;
        };
        let kind = match payload[start] {
            b'f' => FileKind::File,
            b'd' => FileKind::Directory,
            b'l' => FileKind::Symlink,
            _ => FileKind::Other,
        };
        let path = String::from_utf8_lossy(&payload[start + 1..start + end]).into_owned();
        entries.push(FileListEntry { path, kind });
        start += end + 1;
    }
    FileListing { entries, truncated }
}

fn env_to_file(error: EnvError) -> FileError {
    match error {
        EnvError::Remote(file) => file,
        other => FileError::new("unknown", other.to_string()),
    }
}

/// Resolve absolute path using daemon cwd (minimal helper).
pub fn absolute_from_cwd(cwd: &str, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(cwd).join(path)
    }
}
