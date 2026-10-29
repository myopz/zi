// Execution environment trait used by harness and zi-env clients.

use serde_json::Value;

/// Node-style remote error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}

impl FileError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{}: {} ({path})", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for FileError {}

pub type EnvResult<T> = Result<T, FileError>;

#[derive(Debug, Clone)]
pub struct ShellExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub struct FileListing {
    pub entries: Vec<FileListEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct FileListEntry {
    pub path: String,
    pub kind: FileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// Sandbox-facing filesystem and shell surface (pi durable env subset).
pub trait ExecutionEnv: Send + Sync {
    fn id(&self) -> &str;

    fn exec(&self, command: &str, cwd: Option<&str>) -> EnvResult<ShellExecResult>;

    fn read_file(&self, path: &str) -> EnvResult<Vec<u8>>;

    fn write_file(&self, path: &str, bytes: &[u8], append: bool) -> EnvResult<()>;

    fn list_files(&self, path: &str) -> EnvResult<FileListing>;

    /// Optional metadata for harness diagnostics.
    fn hello(&self) -> EnvResult<Value> {
        Ok(Value::Null)
    }
}
