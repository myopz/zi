//! Resolve tool paths relative to the agent working directory.

use std::path::{Component, Path, PathBuf};

/// Resolve `path` against `cwd`, rejecting traversal outside `cwd`.
pub fn resolve_to_cwd(cwd: &Path, path: Option<&str>) -> Result<PathBuf, String> {
    let base = if let Some(p) = path.filter(|s| !s.is_empty()) {
        cwd.join(p)
    } else {
        cwd.to_path_buf()
    };
    let normalized = normalize_path(&base);
    let cwd_norm = normalize_path(cwd);
    if normalized.starts_with(&cwd_norm) || normalized == cwd_norm {
        Ok(normalized)
    } else {
        Err(format!(
            "path escapes working directory: {}",
            normalized.display()
        ))
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c),
        }
    }
    out
}
