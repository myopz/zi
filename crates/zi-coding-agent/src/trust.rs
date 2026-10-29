//! Project trust for loading `.zi/` resources (settings, mcp, extensions, skills, prompts, themes).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::session_store::resolve_agent_dir;
use crate::skills::ZI_CONFIG_DIR;

/// Resources under project `.zi/` that require trust before loading.
pub const TRUST_REQUIRING_RESOURCES: &[&str] = &[
    "settings.json",
    "mcp.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

/// Trust store file under the agent dir.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(flatten)]
    entries: HashMap<String, Option<bool>>,
}

fn trust_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("trusted.json")
}

fn normalize_cwd(cwd: &Path) -> PathBuf {
    fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf())
}

/// Load trust decisions from `~/.zi/agent/trusted.json`.
pub fn load_trust_store() -> HashMap<String, Option<bool>> {
    load_trust_store_from(&resolve_agent_dir()).unwrap_or_default()
}

/// Load trust store from an explicit agent directory.
pub fn load_trust_store_from(agent_dir: &Path) -> Result<HashMap<String, Option<bool>>> {
    let path = trust_path(agent_dir);
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let file: TrustFile = serde_json::from_str(&text)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(file.entries)
}

/// Persist trust store.
pub fn save_trust_store(entries: &HashMap<String, Option<bool>>) -> Result<()> {
    save_trust_store_to(&resolve_agent_dir(), entries)
}

/// Persist trust store under `agent_dir`.
pub fn save_trust_store_to(
    agent_dir: &Path,
    entries: &HashMap<String, Option<bool>>,
) -> Result<()> {
    fs::create_dir_all(agent_dir).with_context(|| format!("create {}", agent_dir.display()))?;
    let path = trust_path(agent_dir);
    let file = TrustFile {
        entries: entries.clone(),
    };
    let text = serde_json::to_string_pretty(&file)?;
    fs::write(&path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Nearest ancestor trust decision for `cwd` (`true` / `false`), if any.
pub fn trust_decision_for(cwd: &Path) -> Option<(PathBuf, bool)> {
    let data = load_trust_store();
    let mut current = normalize_cwd(cwd);
    loop {
        let key = current.to_string_lossy().to_string();
        if let Some(Some(decision)) = data.get(&key) {
            return Some((current, *decision));
        }
        let parent = current.parent()?;
        if parent == current {
            return None;
        }
        current = parent.to_path_buf();
    }
}

/// Whether project `.zi` resources may be loaded for `cwd`.
pub fn is_project_trusted(cwd: &Path) -> bool {
    matches!(trust_decision_for(cwd), Some((_, true)))
}

/// Whether the project directory contains trust-gated resources.
pub fn project_has_trust_resources(cwd: &Path) -> bool {
    let zi = cwd.join(ZI_CONFIG_DIR);
    if !zi.exists() {
        return false;
    }
    TRUST_REQUIRING_RESOURCES.iter().any(|name| zi.join(name).exists())
}

/// Record a trust decision for `cwd` (canonical path key).
pub fn set_project_trust(cwd: &Path, trusted: bool) -> Result<PathBuf> {
    let path = normalize_cwd(cwd);
    let key = path.to_string_lossy().to_string();
    let mut entries = load_trust_store();
    entries.insert(key, Some(trusted));
    save_trust_store(&entries)?;
    Ok(path)
}

/// Clear the stored decision for `cwd`.
pub fn clear_project_trust(cwd: &Path) -> Result<()> {
    let path = normalize_cwd(cwd);
    let key = path.to_string_lossy().to_string();
    let mut entries = load_trust_store();
    entries.remove(&key);
    save_trust_store(&entries)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_trust() {
        let dir = std::env::temp_dir().join(format!(
            "zi-trust-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let project = dir.join("proj");
        fs::create_dir_all(&project).unwrap();
        let project = fs::canonicalize(&project).unwrap();

        save_trust_store_to(&dir, &HashMap::from([(
            project.to_string_lossy().to_string(),
            Some(true),
        )]))
        .unwrap();

        // Point resolve via explicit store path by temporarily using agent_dir helpers:
        let loaded = load_trust_store_from(&dir).unwrap();
        assert_eq!(loaded.get(&project.to_string_lossy().to_string()), Some(&Some(true)));
        let _ = fs::remove_dir_all(&dir);
    }
}
