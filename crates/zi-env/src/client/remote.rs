// RemoteExecutionEnv stub (SSH deploy and reconnect deferred).

use zi_durable::{
    ExecutionEnv, FileError, FileListing, ShellExecResult,
};
use serde_json::Value;

pub struct RemoteExecutionEnvOptions {
    pub id: String,
    pub cwd: String,
    /// SSH target, e.g. `user@host`.
    pub ssh_target: String,
}

/// Remote environment placeholder; use SSH + zi-envd deployment in a later slice.
pub struct RemoteExecutionEnv {
    id: String,
    cwd: String,
    ssh_target: String,
}

impl RemoteExecutionEnv {
    pub fn new(options: RemoteExecutionEnvOptions) -> Self {
        Self {
            id: options.id,
            cwd: options.cwd,
            ssh_target: options.ssh_target,
        }
    }
}

fn not_implemented() -> FileError {
    FileError::new(
        "not_implemented",
        "RemoteExecutionEnv over SSH is not implemented yet (TODO: deploy zi-envd via ssh)",
    )
}

impl ExecutionEnv for RemoteExecutionEnv {
    fn id(&self) -> &str {
        &self.id
    }

    fn exec(&self, _command: &str, _cwd: Option<&str>) -> zi_durable::EnvResult<ShellExecResult> {
        Err(not_implemented())
    }

    fn read_file(&self, _path: &str) -> zi_durable::EnvResult<Vec<u8>> {
        Err(not_implemented())
    }

    fn write_file(&self, _path: &str, _bytes: &[u8], _append: bool) -> zi_durable::EnvResult<()> {
        Err(not_implemented())
    }

    fn list_files(&self, _path: &str) -> zi_durable::EnvResult<FileListing> {
        Err(not_implemented())
    }

    fn hello(&self) -> zi_durable::EnvResult<Value> {
        Err(FileError::new(
            "not_implemented",
            format!(
                "SSH remote env to {} not wired; cwd={}",
                self.ssh_target, self.cwd
            ),
        ))
    }
}
