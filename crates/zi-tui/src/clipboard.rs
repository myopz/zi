// System clipboard via arboard.

use crate::error::TuiError;

/// Reads plain text from the system clipboard.
pub fn read_text() -> Result<Option<String>, TuiError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| TuiError::Clipboard(e.to_string()))?;
    match clipboard.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(TuiError::Clipboard(e.to_string())),
    }
}

/// Writes plain text to the system clipboard.
pub fn write_text(text: &str) -> Result<(), TuiError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| TuiError::Clipboard(e.to_string()))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| TuiError::Clipboard(e.to_string()))
}
