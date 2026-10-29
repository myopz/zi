//! Discover zi/pi packages under agent `npm/`/`git/` and settings `packages` paths.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::session_store::resolve_agent_dir;
use crate::settings::load_settings_from;

/// Package resource roots discovered for loading.
#[derive(Debug, Clone, Default)]
pub struct PackageResources {
    pub extensions: Vec<PathBuf>,
    pub skills: Vec<PathBuf>,
    pub prompts: Vec<PathBuf>,
    pub themes: Vec<PathBuf>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[serde(default)]
    zi: Option<PackageManifest>,
    #[serde(default)]
    pi: Option<PackageManifest>,
}

#[derive(Debug, Default, Deserialize)]
struct PackageManifest {
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    prompts: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
}

/// Collect package resource paths for a workspace (user packages always; project when trusted).
pub fn discover_package_resources(cwd: &Path, include_project: bool) -> PackageResources {
    let agent_dir = resolve_agent_dir();
    discover_package_resources_from(cwd, &agent_dir, include_project)
}

/// Same as [`discover_package_resources`] with explicit agent dir.
pub fn discover_package_resources_from(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> PackageResources {
    let mut out = PackageResources::default();
    let mut roots = Vec::new();

    // Installed package trees
    for sub in ["npm", "git"] {
        collect_package_dirs(&agent_dir.join(sub), &mut roots);
        if include_project {
            collect_package_dirs(&cwd.join(".zi").join(sub), &mut roots);
        }
    }

    // settings.json packages: string paths
    if let Ok(settings) = load_settings_from(agent_dir) {
        for path in &settings.packages {
            let resolved = resolve_pkg_path(cwd, agent_dir, path);
            if resolved.exists() {
                roots.push(resolved);
            } else {
                out.diagnostics
                    .push(format!("package path missing: {path}"));
            }
        }
    }

    for root in roots {
        ingest_package_root(&root, &mut out);
    }
    out
}

/// Prompt paths from packages (user + project when include_project).
pub fn prompt_paths_for(cwd: &Path, agent_dir: &Path) -> Vec<PathBuf> {
    discover_package_resources_from(cwd, agent_dir, true).prompts
}

/// Prompt paths from user-scope packages only.
pub fn prompt_paths_user_only(agent_dir: &Path) -> Vec<PathBuf> {
    discover_package_resources_from(Path::new("."), agent_dir, false).prompts
}

/// Skill paths from packages.
pub fn skill_paths_for(cwd: &Path, agent_dir: &Path, include_project: bool) -> Vec<PathBuf> {
    discover_package_resources_from(cwd, agent_dir, include_project).skills
}

/// Theme paths from packages.
pub fn theme_paths_for(cwd: &Path, agent_dir: &Path, include_project: bool) -> Vec<PathBuf> {
    discover_package_resources_from(cwd, agent_dir, include_project).themes
}

/// Extension paths from packages.
pub fn extension_paths_for(cwd: &Path, agent_dir: &Path, include_project: bool) -> Vec<PathBuf> {
    discover_package_resources_from(cwd, agent_dir, include_project).extensions
}

fn collect_package_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // npm scoped packages: @scope/name
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('@'))
            {
                if let Ok(scoped) = fs::read_dir(&path) {
                    for child in scoped.flatten() {
                        let child_path = child.path();
                        if child_path.is_dir() {
                            out.push(child_path);
                        }
                    }
                }
            } else {
                out.push(path);
            }
        }
    }
}

fn resolve_pkg_path(cwd: &Path, agent_dir: &Path, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else if path.starts_with("~/") {
        dirs_home().join(&path[2..])
    } else if path.starts_with("./") || path.starts_with("../") {
        cwd.join(path)
    } else {
        agent_dir.join(path)
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn ingest_package_root(root: &Path, out: &mut PackageResources) {
    let manifest_path = root.join("package.json");
    if manifest_path.is_file() {
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            if let Ok(pkg) = serde_json::from_str::<PackageJson>(&text) {
                let m = pkg.zi.or(pkg.pi).unwrap_or_default();
                push_rel(root, &m.extensions, &mut out.extensions);
                push_rel(root, &m.skills, &mut out.skills);
                push_rel(root, &m.prompts, &mut out.prompts);
                push_rel(root, &m.themes, &mut out.themes);
                return;
            }
        }
    }
    // Conventional layout without package.json
    for (name, dest) in [
        ("extensions", &mut out.extensions),
        ("skills", &mut out.skills),
        ("prompts", &mut out.prompts),
        ("themes", &mut out.themes),
    ] {
        let path = root.join(name);
        if path.exists() {
            dest.push(path);
        }
    }
}

fn push_rel(root: &Path, rels: &[String], dest: &mut Vec<PathBuf>) {
    for rel in rels {
        let path = root.join(rel);
        if path.exists() {
            dest.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_conventional_layout() {
        let dir = std::env::temp_dir().join(format!(
            "zi-pkg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pkg = dir.join("npm").join("demo-pkg");
        fs::create_dir_all(pkg.join("prompts")).unwrap();
        fs::write(pkg.join("prompts").join("hi.md"), "hello").unwrap();
        let resources = discover_package_resources_from(Path::new("."), &dir, false);
        assert!(resources
            .prompts
            .iter()
            .any(|p| p.ends_with("prompts")));
        let _ = fs::remove_dir_all(&dir);
    }
}
