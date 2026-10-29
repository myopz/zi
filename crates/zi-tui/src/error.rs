// Error types for zi-tui.

use thiserror::Error;

/// Terminal UI failure.
#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("clipboard: {0}")]
    Clipboard(String),
}
