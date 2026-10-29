//! Session I/O and validation errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("session file is not a valid ri session: {0}")]
    InvalidSessionFile(String),
    #[error("session header missing or invalid")]
    MissingHeader,
}
