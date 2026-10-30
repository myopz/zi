//! Load AGENTS.md / CLAUDE.md context and SYSTEM.md / APPEND_SYSTEM.md overrides.

use std::fs;
use std::path::{Path, PathBuf};

use crate::skills::ZI_CONFIG_DIR;

const CONTEXT_CANDIDATES: &[&str] = &[
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// Aggregated context files for the system prompt.
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    /// Ancestor + agent-dir AGENTS/CLAUDE bodies (nearest first after global).
    pub context_files: Vec<(PathBuf, String)>,
    /// Full system prompt replacement (project beats user).
    pub system_md: Option<(PathBuf, String)>,
    /// Appended after the base system prompt.
    pub append_system_md: Option<(PathBuf, String)>,
}

/// Load context for `cwd` and `agent_dir`.
///
/// Project `SYSTEM.md` / `APPEND_SYSTEM.md` are only loaded when `include_project` is true.
pub fn load_agent_context(cwd: &Path, agent_dir: &Path, include_project: bool) -> AgentContext {
    let mut ctx = AgentContext::default();

    if let Some(global) = load_context_file_from_dir(agent_dir) {
        ctx.context_files.push(global);
    }

    // Walk from cwd up to filesystem root; collect unique context files.
    let mut current = cwd.to_path_buf();
    let mut seen = std::collections::HashSet::new();
    loop {
        if let Some((path, content)) = load_context_file_from_dir(&current) {
            let key = path.to_string_lossy().to_string();
            if seen.insert(key) {
                ctx.context_files.push((path, content));
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }

    // SYSTEM.md: project wins over user (not merged).
    if include_project {
        let project_system = cwd.join(ZI_CONFIG_DIR).join("SYSTEM.md");
        if let Some(v) = read_optional_md(&project_system) {
            ctx.system_md = Some(v);
        }
    }
    if ctx.system_md.is_none() {
        let user_system = agent_dir.join("SYSTEM.md");
        if let Some(v) = read_optional_md(&user_system) {
            ctx.system_md = Some(v);
        }
    }

    // APPEND_SYSTEM.md: project wins over user (not merged).
    if include_project {
        let project_append = cwd.join(ZI_CONFIG_DIR).join("APPEND_SYSTEM.md");
        if let Some(v) = read_optional_md(&project_append) {
            ctx.append_system_md = Some(v);
        }
    }
    if ctx.append_system_md.is_none() {
        let user_append = agent_dir.join("APPEND_SYSTEM.md");
        if let Some(v) = read_optional_md(&user_append) {
            ctx.append_system_md = Some(v);
        }
    }

    ctx
}

fn load_context_file_from_dir(dir: &Path) -> Option<(PathBuf, String)> {
    for name in CONTEXT_CANDIDATES {
        let path = dir.join(name);
        if let Some(v) = read_optional_md(&path) {
            return Some(v);
        }
    }
    None
}

fn read_optional_md(path: &Path) -> Option<(PathBuf, String)> {
    if !path.is_file() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some((path.to_path_buf(), trimmed.to_string()))
}

/// Per-file soft cap for AGENTS/CLAUDE injection (~500 tokens at 4 chars/tok).
pub const CONTEXT_FILE_MAX_CHARS: usize = 2_048;
/// Total soft cap across all injected context files.
pub const CONTEXT_BLOCK_MAX_CHARS: usize = 4_096;

/// Format context files into a prompt section (truncated with read pointers).
pub fn format_context_for_prompt(files: &[(PathBuf, String)]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut parts = vec![
        String::new(),
        "## Project context".into(),
        "Standing instructions (truncated). Use read on the listed path for the full file.".into(),
    ];
    let mut used = 0usize;
    for (path, body) in files {
        if used >= CONTEXT_BLOCK_MAX_CHARS {
            parts.push(String::new());
            parts.push(format!(
                "… further context files omitted; read {} if needed …",
                path.display()
            ));
            break;
        }
        let remaining = CONTEXT_BLOCK_MAX_CHARS.saturating_sub(used);
        let file_cap = CONTEXT_FILE_MAX_CHARS.min(remaining);
        let (body_out, truncated) = if body.len() > file_cap {
            (format!(
                "{}\n… truncated; full path: {} — use read with offset/limit …",
                &body[..file_cap],
                path.display()
            ), true)
        } else {
            (body.clone(), false)
        };
        parts.push(String::new());
        parts.push(format!("### {}", path.display()));
        parts.push(body_out.clone());
        used = used.saturating_add(body_out.len());
        let _ = truncated;
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_agents_and_system() {
        let dir = std::env::temp_dir().join(format!(
            "zi-ctx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = dir.join("proj");
        let agent = dir.join("agent");
        fs::create_dir_all(project.join(".zi")).unwrap();
        fs::create_dir_all(&agent).unwrap();
        fs::write(project.join("AGENTS.md"), "Use tabs.").unwrap();
        fs::write(project.join(".zi").join("SYSTEM.md"), "You are custom.").unwrap();
        fs::write(agent.join("APPEND_SYSTEM.md"), "Be brief.").unwrap();

        let ctx = load_agent_context(&project, &agent, true);
        assert!(ctx
            .context_files
            .iter()
            .any(|(_, c)| c.contains("Use tabs")));
        assert_eq!(
            ctx.system_md.as_ref().map(|(_, c)| c.as_str()),
            Some("You are custom.")
        );
        assert_eq!(
            ctx.append_system_md.as_ref().map(|(_, c)| c.as_str()),
            Some("Be brief.")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
