//! Spill oversized tool output to disk so the model transcript keeps a short pointer.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::session_store::resolve_agent_dir;

/// Default byte threshold before spilling (aligned with bash model-facing cap).
pub const DEFAULT_SPILL_THRESHOLD: usize = 16 * 1024;

/// How many leading bytes to keep in the transcript when spilling.
const DISPLAY_HEAD: usize = 4 * 1024;

static SPILL_SEQ: AtomicU64 = AtomicU64::new(1);

/// If `text` exceeds `threshold`, write the full body under
/// `resolve_agent_dir()/tool-outputs/` and return a head + path pointer.
pub fn spill_if_large(text: &str, threshold: usize) -> String {
    if text.len() <= threshold {
        return text.to_string();
    }
    match write_spill(text, "tool") {
        Ok(path) => {
            let head_len = DISPLAY_HEAD.min(text.len());
            let head = &text[..head_len];
            format!(
                "{head}\n\n… full output: {} ({} bytes); use read with offset/limit …",
                path.display(),
                text.len()
            )
        }
        Err(err) => {
            // Fall back to head-only if the agent dir is unwritable.
            let head_len = DISPLAY_HEAD.min(text.len());
            format!(
                "{}\n\n… ({} bytes omitted; spill failed: {err}) …",
                &text[..head_len],
                text.len().saturating_sub(head_len)
            )
        }
    }
}

fn write_spill(text: &str, label: &str) -> std::io::Result<PathBuf> {
    let dir = tool_outputs_dir();
    fs::create_dir_all(&dir)?;
    let seq = SPILL_SEQ.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("{label}-{ms}-{seq}.txt");
    let path = dir.join(name);
    fs::write(&path, text)?;
    Ok(path)
}

fn tool_outputs_dir() -> PathBuf {
    resolve_agent_dir().join("tool-outputs")
}

/// Test helper result: display text plus absolute spill path when written.
#[cfg(test)]
pub struct SpillResult {
    pub display: String,
    pub path: Option<PathBuf>,
}

/// Test helper: spill root under a given agent dir (does not use env).
#[cfg(test)]
pub fn spill_if_large_in(
    agent_dir: &std::path::Path,
    text: &str,
    threshold: usize,
) -> SpillResult {
    if text.len() <= threshold {
        return SpillResult {
            display: text.to_string(),
            path: None,
        };
    }
    let dir = agent_dir.join("tool-outputs");
    if let Err(err) = fs::create_dir_all(&dir) {
        let head_len = DISPLAY_HEAD.min(text.len());
        return SpillResult {
            display: format!(
                "{}\n\n… ({} bytes omitted; spill failed: {err}) …",
                &text[..head_len],
                text.len().saturating_sub(head_len)
            ),
            path: None,
        };
    }
    let seq = SPILL_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("tool-test-{seq}.txt"));
    if let Err(err) = fs::write(&path, text) {
        let head_len = DISPLAY_HEAD.min(text.len());
        return SpillResult {
            display: format!(
                "{}\n\n… ({} bytes omitted; spill failed: {err}) …",
                &text[..head_len],
                text.len().saturating_sub(head_len)
            ),
            path: None,
        };
    }
    let head_len = DISPLAY_HEAD.min(text.len());
    SpillResult {
        display: format!(
            "{}\n\n… full output: {} ({} bytes); use read with offset/limit …",
            &text[..head_len],
            path.display(),
            text.len()
        ),
        path: Some(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn below_threshold_returns_full_text() {
        let display = spill_if_large("hello", 100);
        assert_eq!(display, "hello");
    }

    #[test]
    fn above_threshold_writes_file_and_pointer() {
        let dir = env::temp_dir().join(format!("zi-spill-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmpdir");
        let body = "x".repeat(20_000);
        let r = spill_if_large_in(&dir, &body, 1000);
        assert!(r.path.is_some(), "expected spill path");
        let path = r.path.as_ref().unwrap();
        assert!(path.exists());
        assert_eq!(fs::read_to_string(path).unwrap().len(), 20_000);
        assert!(r.display.contains("full output:"));
        assert!(r.display.contains(path.to_string_lossy().as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }
}
