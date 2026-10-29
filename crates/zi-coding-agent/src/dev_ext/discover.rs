//! Discover host/WASM extensions from `.zi/extensions` manifests.
//!
//! Custom code still loads as `.wasm`. Host features (tps, /tui, prompt-url, /ir) are
//! implemented in Rust but **opted in** by `extension.json` manifests so projects can
//! enable/disable them without recompiling — same discovery roots as WASM guests.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::skills::ZI_CONFIG_DIR;

pub use zi_host_extensions::{
    ALL_BUILTINS, BUILTIN_IMPORT_REPRO, BUILTIN_PROMPT_URL, BUILTIN_REDRAWS, BUILTIN_TPS,
};

/// Extension kind from `extension.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionKind {
    Builtin,
    Wasm,
}

impl ExtensionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Wasm => "wasm",
        }
    }
}

/// Optional resource dirs contributed by an extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionContributes {
    /// Relative dirs (from the extension folder) that contain prompt templates.
    pub prompts: Vec<String>,
    /// Relative dirs that contain skills.
    pub skills: Vec<String>,
}

/// One discovered extension (manifest-backed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredExtension {
    pub id: String,
    pub kind: ExtensionKind,
    pub enabled: bool,
    pub api_version: u32,
    pub description: Option<String>,
    pub manifest_path: PathBuf,
    /// Directory containing the manifest (for resolving contributes / wasm artifact).
    pub root: PathBuf,
    pub contributes: ExtensionContributes,
    pub permissions: Vec<String>,
}

/// Which host extension packs are active for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledDevExt {
    pub tps: bool,
    pub redraws: bool,
    pub prompt_url: bool,
    pub import_repro: bool,
    /// Paths of manifests that contributed (for diagnostics).
    pub sources: Vec<PathBuf>,
    /// Full discovery list (builtins + wasm manifests).
    pub all: Vec<DiscoveredExtension>,
}

impl Default for EnabledDevExt {
    fn default() -> Self {
        // No manifests → keep previous always-on behavior for UX.
        Self {
            tps: true,
            redraws: true,
            prompt_url: true,
            import_repro: true,
            sources: Vec::new(),
            all: Vec::new(),
        }
    }
}

impl EnabledDevExt {
    fn from_ids(
        ids: &HashSet<String>,
        sources: Vec<PathBuf>,
        all: Vec<DiscoveredExtension>,
    ) -> Self {
        Self {
            tps: ids.contains(BUILTIN_TPS),
            redraws: ids.contains(BUILTIN_REDRAWS),
            prompt_url: ids.contains(BUILTIN_PROMPT_URL),
            import_repro: ids.contains(BUILTIN_IMPORT_REPRO),
            sources,
            all,
        }
    }

    /// True when at least one pack is on.
    pub fn any(&self) -> bool {
        self.tps || self.redraws || self.prompt_url || self.import_repro
    }

    /// Enabled extensions that contribute prompt/skill dirs.
    pub fn enabled_contributes(&self) -> impl Iterator<Item = &DiscoveredExtension> {
        self.all.iter().filter(|e| e.enabled)
    }
}

#[derive(Debug, Deserialize)]
struct ExtensionManifest {
    #[serde(default = "default_schema_version", alias = "schemaVersion")]
    #[allow(dead_code)]
    schema_version: u32,
    /// Stable id (`tps`, `redraws`, …) or a custom wasm-facing name.
    id: String,
    /// `builtin` (host pack) or `wasm`.
    #[serde(default = "default_kind")]
    kind: String,
    /// When false, the pack is discovered but not enabled.
    #[serde(default = "default_enabled")]
    enabled: bool,
    /// Guest/host ABI negotiation (WASM).
    #[serde(default = "default_api_version", alias = "apiVersion")]
    api_version: u32,
    description: Option<String>,
    #[serde(default)]
    contributes: ManifestContributes,
    #[serde(default)]
    permissions: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ManifestContributes {
    #[serde(default)]
    prompts: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
}

fn default_schema_version() -> u32 {
    1
}

fn default_kind() -> String {
    "builtin".into()
}

fn default_enabled() -> bool {
    true
}

fn default_api_version() -> u32 {
    1
}

/// Discover all manifests (builtin + wasm) under agent/project extension roots.
pub fn discover_extensions(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> Vec<DiscoveredExtension> {
    let mut dirs = vec![agent_dir.join("extensions")];
    if include_project {
        dirs.push(cwd.join(ZI_CONFIG_DIR).join("extensions"));
    }
    let mut out = Vec::new();
    for dir in dirs {
        scan_extension_dir(&dir, &mut out);
    }
    out
}

/// Default descriptions for seeded builtin manifests (outside the zi source tree).
fn builtin_seed_description(id: &str) -> &'static str {
    match id {
        BUILTIN_TPS => "After each turn, notify tokens/s + usage (port of .pi/extensions/tps.ts)",
        BUILTIN_REDRAWS => "/tui shows TUI full-redraw count (port of .pi/extensions/redraws.ts)",
        BUILTIN_PROMPT_URL => {
            "Sticky PR/issue/advisory widget + session name (port of .pi/extensions/prompt-url-widget.ts)"
        }
        BUILTIN_IMPORT_REPRO => {
            "/ir import gist/share/issue/jsonl sessions (port of .pi/extensions/import-repro.ts)"
        }
        _ => "Host extension pack",
    }
}

fn dir_has_builtin_manifest(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let manifest = path.join("extension.json");
            if manifest.is_file() && is_builtin_manifest_file(&manifest) {
                return true;
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n != "extension.json")
            && is_builtin_manifest_file(&path)
        {
            return true;
        }
    }
    false
}

fn is_builtin_manifest_file(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_str::<ExtensionManifest>(&text) else {
        return false;
    };
    let kind = manifest.kind.to_ascii_lowercase();
    kind == "builtin" || kind == "host"
}

/// Write default builtin `extension.json` files under `~/.zi/agent/extensions/` when neither
/// the agent nor (if trusted) the project tree already has any builtin manifests.
///
/// Keeps toggles outside the zi source tree so encrypted/hidden checkouts still work.
/// Returns how many manifests were written.
pub fn seed_default_builtin_manifests(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> usize {
    let agent_ext = agent_dir.join("extensions");
    if dir_has_builtin_manifest(&agent_ext) {
        return 0;
    }
    if include_project {
        let project_ext = cwd.join(ZI_CONFIG_DIR).join("extensions");
        if dir_has_builtin_manifest(&project_ext) {
            // Preserve project-only enablement subsets used in development checkouts.
            return 0;
        }
    }

    if let Err(err) = fs::create_dir_all(&agent_ext) {
        tracing::warn!(
            path = %agent_ext.display(),
            "failed to create agent extensions dir for seed: {err}"
        );
        return 0;
    }

    let mut written = 0;
    for id in ALL_BUILTINS {
        let dir = agent_ext.join(id);
        let manifest_path = dir.join("extension.json");
        if manifest_path.is_file() {
            continue;
        }
        if let Err(err) = fs::create_dir_all(&dir) {
            tracing::warn!(path = %dir.display(), "seed extension dir failed: {err}");
            continue;
        }
        let body = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "kind": "builtin",
            "enabled": true,
            "apiVersion": 1,
            "description": builtin_seed_description(id),
        });
        let text = match serde_json::to_string_pretty(&body) {
            Ok(t) => format!("{t}\n"),
            Err(err) => {
                tracing::warn!(id = *id, "seed manifest serialize failed: {err}");
                continue;
            }
        };
        match fs::write(&manifest_path, text) {
            Ok(()) => {
                tracing::info!(path = %manifest_path.display(), "seeded builtin extension manifest");
                written += 1;
            }
            Err(err) => {
                tracing::warn!(path = %manifest_path.display(), "seed manifest write failed: {err}");
            }
        }
    }
    written
}

/// Load enabled host packs from agent + (optional) project extension dirs.
///
/// Discovery roots:
/// - `~/.zi/agent/extensions/`
/// - `<cwd>/.zi/extensions/` when `include_project`
///
/// Manifest paths:
/// - `<dir>/<id>/extension.json`
/// - `<dir>/<id>.json`
///
/// If **no** builtin manifests are found, returns [`EnabledDevExt::default`] (all on).
/// If any builtin manifests exist, only those with `enabled: true` are on.
pub fn discover_enabled_dev_ext(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> EnabledDevExt {
    let all = discover_extensions(cwd, agent_dir, include_project);
    let mut ids = HashSet::new();
    let mut sources = Vec::new();
    let mut saw_builtin_manifest = false;

    for ext in &all {
        if ext.kind != ExtensionKind::Builtin {
            continue;
        }
        saw_builtin_manifest = true;
        sources.push(ext.manifest_path.clone());
        if !ext.enabled {
            continue;
        }
        if ALL_BUILTINS.contains(&ext.id.as_str()) {
            ids.insert(ext.id.clone());
        } else {
            tracing::warn!(
                path = %ext.manifest_path.display(),
                id = %ext.id,
                "unknown builtin extension id; known: {}",
                ALL_BUILTINS.join(", ")
            );
        }
    }

    if !saw_builtin_manifest {
        let mut def = EnabledDevExt::default();
        def.all = all;
        return def;
    }
    EnabledDevExt::from_ids(&ids, sources, all)
}

fn scan_extension_dir(dir: &Path, out: &mut Vec<DiscoveredExtension>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let manifest = path.join("extension.json");
            if manifest.is_file() {
                if let Some(ext) = parse_manifest(&manifest, path.clone()) {
                    out.push(ext);
                }
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n != "extension.json")
        {
            let root = path
                .parent()
                .unwrap_or(dir)
                .to_path_buf();
            if let Some(ext) = parse_manifest(&path, root) {
                out.push(ext);
            }
        }
    }
}

fn parse_manifest(path: &Path, root: PathBuf) -> Option<DiscoveredExtension> {
    let text = fs::read_to_string(path).ok()?;
    let manifest: ExtensionManifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!(path = %path.display(), "invalid extension.json; skipping: {err}");
            return None;
        }
    };
    let kind = match manifest.kind.to_ascii_lowercase().as_str() {
        "builtin" | "host" => ExtensionKind::Builtin,
        "wasm" => ExtensionKind::Wasm,
        other => {
            tracing::warn!(
                path = %path.display(),
                kind = other,
                "unknown extension kind; skipping"
            );
            return None;
        }
    };
    Some(DiscoveredExtension {
        id: normalize_id(&manifest.id),
        kind,
        enabled: manifest.enabled,
        api_version: manifest.api_version,
        description: manifest.description,
        manifest_path: path.to_path_buf(),
        root,
        contributes: ExtensionContributes {
            prompts: manifest.contributes.prompts,
            skills: manifest.contributes.skills,
        },
        permissions: manifest.permissions,
    })
}

fn normalize_id(id: &str) -> String {
    id.trim().to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_manifests_enables_all() {
        let dir = std::env::temp_dir().join(format!(
            "zi-ext-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let enabled = discover_enabled_dev_ext(Path::new("."), &dir, false);
        assert!(enabled.tps && enabled.redraws && enabled.prompt_url && enabled.import_repro);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifests_select_subset() {
        let dir = std::env::temp_dir().join(format!(
            "zi-ext-sel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ext = dir.join("extensions");
        fs::create_dir_all(ext.join("tps")).unwrap();
        fs::write(
            ext.join("tps").join("extension.json"),
            r#"{"id":"tps","kind":"builtin"}"#,
        )
        .unwrap();
        fs::write(
            ext.join("redraws.json"),
            r#"{"id":"redraws","kind":"builtin","enabled":true}"#,
        )
        .unwrap();
        fs::create_dir_all(ext.join("import-repro")).unwrap();
        fs::write(
            ext.join("import-repro").join("extension.json"),
            r#"{"id":"import-repro","kind":"builtin","enabled":false}"#,
        )
        .unwrap();

        let enabled = discover_enabled_dev_ext(Path::new("."), &dir, false);
        assert!(enabled.tps);
        assert!(enabled.redraws);
        assert!(!enabled.prompt_url);
        assert!(!enabled.import_repro);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wasm_manifest_with_contributes() {
        let dir = std::env::temp_dir().join(format!(
            "zi-ext-wasm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ext = dir.join("extensions").join("hello");
        fs::create_dir_all(ext.join("prompts")).unwrap();
        fs::write(
            ext.join("extension.json"),
            r#"{"schemaVersion":1,"id":"hello","kind":"wasm","apiVersion":1,"enabled":true,"contributes":{"prompts":["prompts"],"skills":["skills"]}}"#,
        )
        .unwrap();
        let all = discover_extensions(Path::new("."), &dir, false);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind, ExtensionKind::Wasm);
        assert_eq!(all[0].contributes.prompts, vec!["prompts".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn seeds_agent_builtins_when_empty() {
        let dir = std::env::temp_dir().join(format!(
            "zi-ext-seed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let written = seed_default_builtin_manifests(Path::new("."), &dir, false);
        assert_eq!(written, ALL_BUILTINS.len());
        let again = seed_default_builtin_manifests(Path::new("."), &dir, false);
        assert_eq!(again, 0);
        let enabled = discover_enabled_dev_ext(Path::new("."), &dir, false);
        assert!(enabled.tps && enabled.redraws && enabled.prompt_url && enabled.import_repro);
        assert!(dir.join("extensions/tps/extension.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn seed_skips_when_project_has_builtins() {
        let agent = std::env::temp_dir().join(format!(
            "zi-ext-seed-agent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cwd = std::env::temp_dir().join(format!(
            "zi-ext-seed-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&agent).unwrap();
        let proj_ext = cwd.join(ZI_CONFIG_DIR).join("extensions").join("tps");
        fs::create_dir_all(&proj_ext).unwrap();
        fs::write(
            proj_ext.join("extension.json"),
            r#"{"id":"tps","kind":"builtin","enabled":true}"#,
        )
        .unwrap();
        let written = seed_default_builtin_manifests(&cwd, &agent, true);
        assert_eq!(written, 0);
        assert!(!agent.join("extensions/tps/extension.json").is_file());
        let _ = fs::remove_dir_all(&agent);
        let _ = fs::remove_dir_all(&cwd);
    }
}
