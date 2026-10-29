//! Theme JSON loading (`~/.zi/agent/themes`, `.zi/themes`, packages).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::skills::ZI_CONFIG_DIR;

/// Minimal theme schema (pi theme subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)]
    pub appearance: Option<String>,
    #[serde(default)]
    pub colors: HashMap<String, String>,
    #[serde(default)]
    pub vars: HashMap<String, String>,
}

/// Built-in dark theme (ANSI-friendly names).
pub fn builtin_dark() -> Theme {
    Theme {
        name: "dark".into(),
        appearance: Some("dark".into()),
        colors: HashMap::from([
            ("accent".into(), "magenta".into()),
            ("text".into(), "white".into()),
            ("muted".into(), "brightBlack".into()),
            ("error".into(), "red".into()),
            ("success".into(), "green".into()),
            ("border".into(), "cyan".into()),
        ]),
        vars: HashMap::new(),
    }
}

/// Built-in light theme.
pub fn builtin_light() -> Theme {
    Theme {
        name: "light".into(),
        appearance: Some("light".into()),
        colors: HashMap::from([
            ("accent".into(), "blue".into()),
            ("text".into(), "black".into()),
            ("muted".into(), "brightBlack".into()),
            ("error".into(), "red".into()),
            ("success".into(), "green".into()),
            ("border".into(), "blue".into()),
        ]),
        vars: HashMap::new(),
    }
}

/// Load all themes (builtins + disk). Later sources override same name.
pub fn load_themes(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
    extra_paths: &[PathBuf],
) -> Vec<Theme> {
    let mut by_name: HashMap<String, Theme> = HashMap::new();
    for t in [builtin_dark(), builtin_light()] {
        by_name.insert(t.name.clone(), t);
    }
    merge_theme_dir(&agent_dir.join("themes"), &mut by_name);
    if include_project {
        merge_theme_dir(&cwd.join(ZI_CONFIG_DIR).join("themes"), &mut by_name);
    }
    for path in extra_paths {
        if path.is_dir() {
            merge_theme_dir(path, &mut by_name);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(t) = load_theme_file(path) {
                by_name.insert(t.name.clone(), t);
            }
        }
    }
    let mut themes: Vec<_> = by_name.into_values().collect();
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    themes
}

/// Resolve theme by name (case-sensitive), falling back to dark.
pub fn resolve_theme(themes: &[Theme], name: &str) -> Theme {
    themes
        .iter()
        .find(|t| t.name == name)
        .cloned()
        .unwrap_or_else(builtin_dark)
}

/// ANSI SGR for a named theme color key (best-effort; unknown → reset-friendly dim).
pub fn ansi_for_color(theme: &Theme, key: &str) -> &'static str {
    let raw = theme
        .colors
        .get(key)
        .map(String::as_str)
        .unwrap_or("text");
    match raw.to_ascii_lowercase().as_str() {
        "red" => "\x1b[31m",
        "green" => "\x1b[32m",
        "yellow" => "\x1b[33m",
        "blue" => "\x1b[34m",
        "magenta" | "violet" => "\x1b[35m",
        "cyan" => "\x1b[36m",
        "white" => "\x1b[37m",
        "black" => "\x1b[30m",
        "brightblack" | "muted" | "dim" => "\x1b[90m",
        _ => "\x1b[39m",
    }
}

pub const ANSI_RESET: &str = "\x1b[0m";

fn merge_theme_dir(dir: &Path, by_name: &mut HashMap<String, Theme>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(t) = load_theme_file(&path) {
                by_name.insert(t.name.clone(), t);
            }
        }
    }
}

fn load_theme_file(path: &Path) -> Result<Theme, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut theme: Theme = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if theme.name.is_empty() {
        theme.name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "theme".into());
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_present() {
        let themes = load_themes(Path::new("."), Path::new("/nonexistent"), false, &[]);
        assert!(themes.iter().any(|t| t.name == "dark"));
        assert!(themes.iter().any(|t| t.name == "light"));
    }
}
