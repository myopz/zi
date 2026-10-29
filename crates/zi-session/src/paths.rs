//! Default session directory layout under `~/.zi/agent/`.

use std::path::{Path, PathBuf};

/// Default zi agent config root (`~/.zi/agent`).
pub fn agent_dir() -> PathBuf {
    home_dir()
        .map(|home| home.join(".zi").join("agent"))
        .unwrap_or_else(|| PathBuf::from(".zi/agent"))
}

/// All session project directories (`~/.zi/agent/sessions`).
pub fn sessions_root() -> PathBuf {
    agent_dir().join("sessions")
}

/// Encode a cwd into the sessions subdirectory name (matches pi session-manager).
pub fn encode_cwd_for_sessions(cwd: &Path) -> String {
    let resolved = cwd.to_string_lossy();
    let trimmed = resolved.trim_start_matches(['/', '\\']);
    format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
}

/// Default session directory for a working directory.
pub fn default_session_dir(cwd: &Path) -> PathBuf {
    sessions_root().join(encode_cwd_for_sessions(cwd))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
