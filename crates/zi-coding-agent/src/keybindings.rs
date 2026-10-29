//! Load `keybindings.json` from the agent dir (pi-compatible subset).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::session_store::resolve_agent_dir;

/// Named actions that the TUI / interactive loop understands.
pub const ACTION_ABORT: &str = "abort";
pub const ACTION_QUIT: &str = "quit";
pub const ACTION_SUBMIT: &str = "submit";
pub const ACTION_NEWLINE: &str = "newline";
pub const ACTION_COPY: &str = "copy";

/// Default key chords → action names.
pub fn default_keybindings() -> HashMap<String, String> {
    HashMap::from([
        ("escape".into(), ACTION_ABORT.into()),
        ("ctrl+c".into(), ACTION_QUIT.into()),
        ("enter".into(), ACTION_SUBMIT.into()),
        ("shift+enter".into(), ACTION_NEWLINE.into()),
        ("ctrl+j".into(), ACTION_NEWLINE.into()),
        ("ctrl+y".into(), ACTION_COPY.into()),
    ])
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct KeybindingsFile {
    bindings: HashMap<String, String>,
}

/// Merged keybindings (defaults overridden by file).
#[derive(Debug, Clone)]
pub struct Keybindings {
    pub map: HashMap<String, String>,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            map: default_keybindings(),
        }
    }
}

impl Keybindings {
    /// Lookup action for a normalized chord (`ctrl+c`, `escape`, …).
    pub fn action_for(&self, chord: &str) -> Option<&str> {
        self.map.get(chord).map(String::as_str)
    }

    /// Inverse: chords that trigger `action`.
    pub fn chords_for(&self, action: &str) -> Vec<&str> {
        self.map
            .iter()
            .filter(|(_, a)| a.as_str() == action)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Human-readable lines for `/hotkeys`.
    pub fn help_lines(&self) -> Vec<String> {
        let mut pairs: Vec<_> = self.map.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        pairs
            .into_iter()
            .map(|(k, v)| format!("  {k:16} {v}"))
            .collect()
    }
}

/// Load `~/.zi/agent/keybindings.json`, merging over defaults.
pub fn load_keybindings() -> Keybindings {
    load_keybindings_from(&resolve_agent_dir())
}

/// Load keybindings from an explicit agent directory.
pub fn load_keybindings_from(agent_dir: &Path) -> Keybindings {
    let mut map = default_keybindings();
    let path = agent_dir.join("keybindings.json");
    if path.is_file() {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(file) = serde_json::from_str::<KeybindingsFile>(&text) {
                for (k, v) in file.bindings {
                    map.insert(normalize_chord(&k), v);
                }
            }
        }
    }
    Keybindings { map }
}

/// Normalize chord spelling for lookups.
pub fn normalize_chord(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(' ', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_abort() {
        let kb = Keybindings::default();
        assert_eq!(kb.action_for("escape"), Some(ACTION_ABORT));
    }

    #[test]
    fn file_overrides() {
        let dir = std::env::temp_dir().join(format!(
            "zi-kb-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("keybindings.json"),
            r#"{ "ctrl+q": "quit" }"#,
        )
        .unwrap();
        let kb = load_keybindings_from(&dir);
        assert_eq!(kb.action_for("ctrl+q"), Some(ACTION_QUIT));
        assert_eq!(kb.action_for("escape"), Some(ACTION_ABORT));
        let _ = fs::remove_dir_all(&dir);
    }
}
