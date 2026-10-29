// zi-env client and wire errors.

use zi_durable::FileError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("remote: {0}")]
    Remote(FileError),
    #[error("connection lost")]
    ConnectionLost,
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("process: {0}")]
    Process(String),
    #[error("{0}")]
    Other(String),
}

pub type EnvResult<T> = Result<T, EnvError>;

impl From<FileError> for EnvError {
    fn from(value: FileError) -> Self {
        Self::Remote(value)
    }
}
