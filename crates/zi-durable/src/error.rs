// Durable harness and storage errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DurableError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("session not initialized")]
    NoSession,
    #[error("conversation {0} not found")]
    ConversationNotFound(u64),
    #[error("entry {0} not found")]
    EntryNotFound(u64),
    #[error("compaction not implemented: {0}")]
    CompactionNotImplemented(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type DurableResult<T> = Result<T, DurableError>;
