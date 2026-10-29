//! Directory-based Agent Skills discovery (`SKILL.md` under `~/.zi/agent/skills` and `.zi/skills`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Project config directory name (analogous to `.pi/` in the source repo).
pub const ZI_CONFIG_DIR: &str = ".zi";

const SKILL_FILE: &str = "SKILL.md";
const MAX_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 1024;

/// Parsed YAML frontmatter fields we care about for skills.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    disable_model_invocation: Option<bool>,
}

/// One discovered skill on disk.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: PathBuf,
    pub base_dir: PathBuf,
    pub body: String,
    pub disable_model_invocation: bool,
    pub source: SkillSource,
}

/// Where a skill was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    User,
    Project,
    Path,
    /// `~/.agents/skills` or `.agents/skills` (Agent Skills spec locations).
    Agents,
}

/// Non-fatal issues while scanning or parsing skills.
#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

/// Result of loading skills from configured locations.
#[derive(Debug, Default)]
pub struct LoadSkillsResult {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

/// Options for [`load_skills`].
#[derive(Debug, Clone)]
pub struct LoadSkillsOptions<'a> {
    pub cwd: &'a Path,
    pub agent_dir: &'a Path,
    pub include_defaults: bool,
    /// When false, skip project `.zi/skills` (untrusted project).
    pub include_project: bool,
    pub extra_paths: &'a [PathBuf],
}

/// Load skills from default dirs and optional extra paths.
pub fn load_skills(options: LoadSkillsOptions<'_>) -> LoadSkillsResult {
    let mut by_name: HashMap<String, Skill> = HashMap::new();
    let mut diagnostics = Vec::new();

    if options.include_defaults {
        let user_dir = options.agent_dir.join("skills");
        merge_dir(&user_dir, SkillSource::User, &mut by_name, &mut diagnostics);

        if options.include_project {
            let project_dir = options.cwd.join(ZI_CONFIG_DIR).join("skills");
            merge_dir(
                &project_dir,
                SkillSource::Project,
                &mut by_name,
                &mut diagnostics,
            );
        }
    }

    for path in options.extra_paths {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            options.cwd.join(path)
        };
        if !resolved.exists() {
            diagnostics.push(SkillDiagnostic {
                path: resolved,
                message: "skill path does not exist".into(),
            });
            continue;
        }
        let meta = match fs::metadata(&resolved) {
            Ok(m) => m,
            Err(err) => {
                diagnostics.push(SkillDiagnostic {
                    path: resolved,
                    message: err.to_string(),
                });
                continue;
            }
        };
        if meta.is_dir() {
            merge_dir(&resolved, SkillSource::Path, &mut by_name, &mut diagnostics);
        } else if resolved.extension().and_then(|e| e.to_str()) == Some("md") {
            merge_file(&resolved, SkillSource::Path, &mut by_name, &mut diagnostics);
        } else {
            diagnostics.push(SkillDiagnostic {
                path: resolved,
                message: "skill path is not a markdown file or directory".into(),
            });
        }
    }

    let mut skills: Vec<Skill> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    LoadSkillsResult {
        skills,
        diagnostics,
    }
}

/// Load default skill directories for `cwd` and the agent dir from [`crate::session_store::resolve_agent_dir`].
pub fn load_default_skills(cwd: &Path) -> LoadSkillsResult {
    let agent_dir = crate::session_store::resolve_agent_dir();
    let trusted = crate::trust::is_project_trusted(cwd);
    let mut extra = crate::packages::skill_paths_for(cwd, &agent_dir, trusted);
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        extra.push(PathBuf::from(home).join(".agents").join("skills"));
    }
    if trusted {
        extra.push(cwd.join(".agents").join("skills"));
    }
    load_skills(LoadSkillsOptions {
        cwd,
        agent_dir: &agent_dir,
        include_defaults: true,
        include_project: trusted,
        extra_paths: &extra,
    })
}

/// Find a skill by name (case-sensitive).
pub fn find_skill<'a>(skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    skills.iter().find(|s| s.name == name)
}

/// Format skills for inclusion in the system prompt (Agent Skills XML block).
pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        String::new(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".into(),
        "Use the read tool to load a skill's file when the task matches its description.".into(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md) and use that absolute path in tool commands.".into(),
        String::new(),
        "<available_skills>".into(),
    ];

    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path.display().to_string())
        ));
        lines.push(format!(
            "    <base_dir>{}</base_dir>",
            escape_xml(&skill.base_dir.display().to_string())
        ));
        lines.push(format!(
            "    <source>{}</source>",
            match skill.source {
                SkillSource::User => "user",
                SkillSource::Project => "project",
                SkillSource::Path => "path",
                SkillSource::Agents => "agents",
            }
        ));
        if !skill.body.is_empty() {
            lines.push(format!(
                "    <body_chars>{}</body_chars>",
                skill.body.chars().count()
            ));
        }
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn merge_dir(
    dir: &Path,
    source: SkillSource,
    by_name: &mut HashMap<String, Skill>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    let result = load_skills_from_dir(dir, source);
    diagnostics.extend(result.diagnostics);
    for skill in result.skills {
        insert_skill(skill, by_name, diagnostics);
    }
}

fn merge_file(
    path: &Path,
    source: SkillSource,
    by_name: &mut HashMap<String, Skill>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    match parse_skill_file(path, source) {
        Ok(Some(skill)) => insert_skill(skill, by_name, diagnostics),
        Ok(None) => {}
        Err(msg) => diagnostics.push(SkillDiagnostic {
            path: path.to_path_buf(),
            message: msg,
        }),
    }
}

fn insert_skill(
    skill: Skill,
    by_name: &mut HashMap<String, Skill>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    if let Some(existing) = by_name.get(&skill.name) {
        diagnostics.push(SkillDiagnostic {
            path: skill.file_path.clone(),
            message: format!(
                "name \"{}\" collision (already loaded from {})",
                skill.name,
                existing.file_path.display()
            ),
        });
        return;
    }
    by_name.insert(skill.name.clone(), skill);
}

fn load_skills_from_dir(dir: &Path, source: SkillSource) -> LoadSkillsResult {
    let mut skills = Vec::new();
    let mut diagnostics = Vec::new();
    if !dir.is_dir() {
        return LoadSkillsResult {
            skills,
            diagnostics,
        };
    }
    scan_skills_dir(dir, source, true, &mut skills, &mut diagnostics);
    LoadSkillsResult {
        skills,
        diagnostics,
    }
}

fn scan_skills_dir(
    dir: &Path,
    source: SkillSource,
    include_root_md: bool,
    skills: &mut Vec<Skill>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            diagnostics.push(SkillDiagnostic {
                path: dir.to_path_buf(),
                message: err.to_string(),
            });
            return;
        }
    };

    let mut children: Vec<_> = entries.filter_map(|e| e.ok()).collect();

    for entry in &children {
        if entry.file_name().to_string_lossy() == SKILL_FILE {
            let path = entry.path();
            if path.is_file() {
                match parse_skill_file(&path, source) {
                    Ok(Some(skill)) => skills.push(skill),
                    Ok(None) => {}
                    Err(msg) => diagnostics.push(SkillDiagnostic {
                        path,
                        message: msg,
                    }),
                }
            }
            return;
        }
    }

    children.sort_by_key(|e| e.file_name());

    for entry in children {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type().ok();
        let is_dir = file_type.as_ref().is_some_and(|t| t.is_dir());
        let is_file = file_type.as_ref().is_some_and(|t| t.is_file());

        if is_dir {
            scan_skills_dir(&path, source, false, skills, diagnostics);
        } else if is_file && include_root_md && name_str.ends_with(".md") {
            match parse_skill_file(&path, source) {
                Ok(Some(skill)) => skills.push(skill),
                Ok(None) => {}
                Err(msg) => diagnostics.push(SkillDiagnostic {
                    path,
                    message: msg,
                }),
            }
        }
    }
}

fn parse_skill_file(path: &Path, source: SkillSource) -> Result<Option<Skill>, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (frontmatter, body) = parse_frontmatter(&raw);
    let is_declared = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy() == SKILL_FILE);

    let description = frontmatter
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());

    let Some(description) = description else {
        if is_declared {
            return Err("description is required".into());
        }
        return Ok(None);
    };

    if description.len() > MAX_DESCRIPTION_LEN {
        tracing::warn!(
            path = %path.display(),
            "skill description exceeds {} characters",
            MAX_DESCRIPTION_LEN
        );
    }

    let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let parent_name = base_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".into());

    let name = frontmatter
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(parent_name.as_str())
        .to_string();

    if name.len() > MAX_NAME_LEN {
        tracing::warn!(
            path = %path.display(),
            "skill name exceeds {} characters",
            MAX_NAME_LEN
        );
    }
    validate_skill_name(&name, path);

    Ok(Some(Skill {
        name,
        description: description.to_string(),
        file_path: path.to_path_buf(),
        base_dir,
        body,
        disable_model_invocation: frontmatter.disable_model_invocation == Some(true),
        source,
    }))
}

fn validate_skill_name(name: &str, path: &Path) {
    let mut problems = Vec::new();
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        problems.push("name contains invalid characters (lowercase a-z, 0-9, hyphens only)");
    }
    if name.starts_with('-') || name.ends_with('-') {
        problems.push("name must not start or end with a hyphen");
    }
    if name.contains("--") {
        problems.push("name must not contain consecutive hyphens");
    }
    for msg in problems {
        tracing::warn!(path = %path.display(), "{msg}");
    }
}

fn parse_frontmatter(content: &str) -> (SkillFrontmatter, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return (SkillFrontmatter::default(), trimmed.to_string());
    }
    let Some(end) = trimmed[3..].find("\n---") else {
        return (SkillFrontmatter::default(), trimmed.to_string());
    };
    let yaml = &trimmed[4..4 + end];
    let body_start = 4 + end + 4;
    let body = trimmed.get(body_start..).unwrap_or("").trim().to_string();
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap_or_default();
    (frontmatter, body)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
