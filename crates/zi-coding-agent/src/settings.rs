//! Persist agent preferences under `~/.zi/agent/settings.json`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::session_store::resolve_agent_dir;

/// User preferences (pi `settings.json` subset).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    /// Last selected / preferred model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Active theme name (`dark`, `light`, or a custom JSON theme).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Package paths / sources to load (local dirs relative to agent dir or absolute).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<String>,
}

fn settings_path(agent_dir: &Path) -> std::path::PathBuf {
    agent_dir.join("settings.json")
}

/// Load settings from the agent dir; missing file → empty defaults.
pub fn load_settings() -> AgentSettings {
    load_settings_from(&resolve_agent_dir()).unwrap_or_default()
}

/// Load settings from an explicit agent directory.
pub fn load_settings_from(agent_dir: &Path) -> Result<AgentSettings> {
    let path = settings_path(agent_dir);
    if !path.is_file() {
        return Ok(AgentSettings::default());
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let settings: AgentSettings = serde_json::from_str(&text)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(settings)
}

/// Persist `defaultModel` (merges with existing settings.json when present).
pub fn save_default_model(model_id: &str) -> Result<()> {
    save_default_model_to(&resolve_agent_dir(), model_id)
}

/// Persist `defaultModel` under `agent_dir`.
pub fn save_default_model_to(agent_dir: &Path, model_id: &str) -> Result<()> {
    let mut settings = load_settings_from(agent_dir).unwrap_or_default();
    settings.default_model = Some(model_id.to_string());
    save_settings_to(agent_dir, &settings)
}

/// Persist `theme` under `agent_dir`.
pub fn save_theme(theme: &str) -> Result<()> {
    save_theme_to(&resolve_agent_dir(), theme)
}

/// Persist `theme` under `agent_dir`.
pub fn save_theme_to(agent_dir: &Path, theme: &str) -> Result<()> {
    let mut settings = load_settings_from(agent_dir).unwrap_or_default();
    settings.theme = Some(theme.to_string());
    save_settings_to(agent_dir, &settings)
}

/// Write full settings JSON.
pub fn save_settings_to(agent_dir: &Path, settings: &AgentSettings) -> Result<()> {
    fs::create_dir_all(agent_dir)
        .with_context(|| format!("create {}", agent_dir.display()))?;
    let path = settings_path(agent_dir);
    let text = serde_json::to_string_pretty(settings)?;
    fs::write(&path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_default_model() {
        let dir = std::env::temp_dir().join(format!(
            "zi-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        save_default_model_to(&dir, "glm-4.7-flash").unwrap();
        let loaded = load_settings_from(&dir).unwrap();
        assert_eq!(loaded.default_model.as_deref(), Some("glm-4.7-flash"));
        let raw = fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(raw.contains("defaultModel"));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!(
            "zi-settings-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let loaded = load_settings_from(&dir).unwrap();
        assert!(loaded.default_model.is_none());
    }
}
