//! Prompt templates from `~/.zi/agent/prompts` and `.zi/prompts` (same-name slash commands).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::skills::ZI_CONFIG_DIR;

/// One markdown prompt template (filename without `.md` = slash name).
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    #[allow(dead_code)]
    pub argument_hint: Option<String>,
    pub content: String,
    #[allow(dead_code)]
    pub file_path: PathBuf,
    #[allow(dead_code)]
    pub source: PromptSource,
}

/// Where a prompt template was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSource {
    User,
    Project,
    Path,
    #[allow(dead_code)]
    Package,
}

/// Non-fatal load issues.
#[derive(Debug, Clone)]
pub struct PromptDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

/// Result of [`load_prompt_templates`].
#[derive(Debug, Default)]
pub struct LoadPromptsResult {
    pub templates: Vec<PromptTemplate>,
    pub diagnostics: Vec<PromptDiagnostic>,
}

/// Options for [`load_prompt_templates`].
#[derive(Debug, Clone)]
pub struct LoadPromptsOptions<'a> {
    pub cwd: &'a Path,
    pub agent_dir: &'a Path,
    pub include_defaults: bool,
    /// When false, skip project `.zi/prompts` (untrusted project).
    pub include_project: bool,
    pub extra_paths: &'a [PathBuf],
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct PromptFrontmatter {
    description: Option<String>,
    argument_hint: Option<String>,
}

/// Load prompt templates (later sources win on name collision).
pub fn load_prompt_templates(options: LoadPromptsOptions<'_>) -> LoadPromptsResult {
    let mut by_name: HashMap<String, PromptTemplate> = HashMap::new();
    let mut diagnostics = Vec::new();

    if options.include_defaults {
        merge_dir(
            &options.agent_dir.join("prompts"),
            PromptSource::User,
            &mut by_name,
            &mut diagnostics,
        );
        if options.include_project {
            merge_dir(
                &options.cwd.join(ZI_CONFIG_DIR).join("prompts"),
                PromptSource::Project,
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
            continue;
        }
        if resolved.is_dir() {
            merge_dir(&resolved, PromptSource::Path, &mut by_name, &mut diagnostics);
        } else if resolved.extension().and_then(|e| e.to_str()) == Some("md") {
            match load_template_file(&resolved, PromptSource::Path) {
                Ok(Some(t)) => {
                    by_name.insert(t.name.clone(), t);
                }
                Ok(None) => {}
                Err(msg) => diagnostics.push(PromptDiagnostic {
                    path: resolved,
                    message: msg,
                }),
            }
        }
    }

    let mut templates: Vec<_> = by_name.into_values().collect();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    LoadPromptsResult {
        templates,
        diagnostics,
    }
}

/// Load defaults for cwd / agent dir, respecting project trust.
pub fn load_default_prompts(cwd: &Path, include_project: bool) -> LoadPromptsResult {
    let agent_dir = crate::session_store::resolve_agent_dir();
    let mut extra = Vec::new();
    if include_project {
        extra.extend(crate::packages::prompt_paths_for(cwd, &agent_dir));
    } else {
        extra.extend(crate::packages::prompt_paths_user_only(&agent_dir));
    }
    load_prompt_templates(LoadPromptsOptions {
        cwd,
        agent_dir: &agent_dir,
        include_defaults: true,
        include_project,
        extra_paths: &extra,
    })
}

/// Parse bash-style quoted command arguments.
pub fn parse_command_args(args_string: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_quote: Option<char> = None;

    for ch in args_string.chars() {
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
            has_token = true;
        } else if ch.is_whitespace() {
            if has_token {
                args.push(std::mem::take(&mut current));
                has_token = false;
            }
        } else {
            current.push(ch);
            has_token = true;
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// Substitute `$1`, `$@`, `${N:-default}`, `${@:N}`, `${@:N:L}` in template content.
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all_args = args.join(" ");
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        let rest = &content[i..];
        if let Some(end) = rest.find('}') {
            if rest.starts_with("${") {
                let inner = &rest[2..end];
                if let Some(replaced) = expand_braced(inner, args, &all_args) {
                    out.push_str(&replaced);
                    i += end + 1;
                    continue;
                }
            }
        }
        if let Some(replaced) = expand_simple(rest, args, &all_args) {
            out.push_str(&replaced.0);
            i += replaced.1;
            continue;
        }
        out.push('$');
        i += 1;
    }
    out
}

fn expand_braced(inner: &str, args: &[String], all_args: &str) -> Option<String> {
    if let Some((target, default)) = inner.split_once(":-") {
        let value = if target == "@" || target == "ARGUMENTS" {
            all_args
        } else if let Ok(n) = target.parse::<usize>() {
            args.get(n.wrapping_sub(1)).map(String::as_str).unwrap_or("")
        } else {
            return None;
        };
        return Some(if value.is_empty() {
            default.to_string()
        } else {
            value.to_string()
        });
    }
    if let Some(slice) = inner.strip_prefix("@:") {
        let (start_s, len_s) = match slice.split_once(':') {
            Some((s, l)) => (s, Some(l)),
            None => (slice, None),
        };
        let mut start = start_s.parse::<isize>().ok()? - 1;
        if start < 0 {
            start = 0;
        }
        let start = start as usize;
        return Some(if let Some(l) = len_s {
            let length = l.parse::<usize>().ok()?;
            args.get(start..start.saturating_add(length).min(args.len()))
                .unwrap_or(&[])
                .join(" ")
        } else {
            args.get(start..).unwrap_or(&[]).join(" ")
        });
    }
    None
}

fn expand_simple(rest: &str, args: &[String], all_args: &str) -> Option<(String, usize)> {
    if rest.starts_with("$ARGUMENTS") {
        return Some((all_args.to_string(), "$ARGUMENTS".len()));
    }
    if rest.starts_with("$@") {
        return Some((all_args.to_string(), 2));
    }
    if rest.len() >= 2 && rest.as_bytes()[1].is_ascii_digit() {
        let mut n = 0usize;
        let mut len = 1;
        for b in rest.as_bytes().iter().skip(1) {
            if b.is_ascii_digit() {
                n = n * 10 + (*b - b'0') as usize;
                len += 1;
            } else {
                break;
            }
        }
        let value = args
            .get(n.wrapping_sub(1))
            .cloned()
            .unwrap_or_default();
        return Some((value, len));
    }
    None
}

/// Expand `/name args` when `name` matches a template; otherwise return `None`.
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let rest = &trimmed[1..];
    let (name, args_str) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a),
        None => (rest, ""),
    };
    let template = templates.iter().find(|t| t.name == name)?;
    let args = parse_command_args(args_str);
    Some(substitute_args(&template.content, &args))
}

fn merge_dir(
    dir: &Path,
    source: PromptSource,
    by_name: &mut HashMap<String, PromptTemplate>,
    diagnostics: &mut Vec<PromptDiagnostic>,
) {
    if !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            diagnostics.push(PromptDiagnostic {
                path: dir.to_path_buf(),
                message: err.to_string(),
            });
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") || !path.is_file() {
            continue;
        }
        match load_template_file(&path, source) {
            Ok(Some(t)) => {
                by_name.insert(t.name.clone(), t);
            }
            Ok(None) => {}
            Err(msg) => diagnostics.push(PromptDiagnostic {
                path,
                message: msg,
            }),
        }
    }
}

fn load_template_file(
    path: &Path,
    source: PromptSource,
) -> Result<Option<PromptTemplate>, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (frontmatter, body) = parse_frontmatter(&raw);
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "prompt".into());
    let description = frontmatter
        .description
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| {
            body.lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| {
                    let t = l.trim();
                    if t.chars().count() > 60 {
                        format!("{}...", t.chars().take(60).collect::<String>())
                    } else {
                        t.to_string()
                    }
                })
                .unwrap_or_default()
        });
    Ok(Some(PromptTemplate {
        name,
        description,
        argument_hint: frontmatter.argument_hint,
        content: body,
        file_path: path.to_path_buf(),
        source,
    }))
}

fn parse_frontmatter(content: &str) -> (PromptFrontmatter, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return (PromptFrontmatter::default(), trimmed.to_string());
    }
    let Some(end) = trimmed[3..].find("\n---") else {
        return (PromptFrontmatter::default(), trimmed.to_string());
    };
    let yaml = &trimmed[4..4 + end];
    let body_start = 4 + end + 4;
    let body = trimmed.get(body_start..).unwrap_or("").trim().to_string();
    let frontmatter: PromptFrontmatter = serde_yaml::from_str(yaml).unwrap_or_default();
    (frontmatter, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_positional_and_all() {
        let args = vec!["a".into(), "b".into()];
        assert_eq!(substitute_args("x $1 y", &args), "x a y");
        assert_eq!(substitute_args("all:$@", &args), "all:a b");
        assert_eq!(
            substitute_args("${1:-fallback} ${3:-z}", &args),
            "a z"
        );
    }

    #[test]
    fn parse_quoted_args() {
        assert_eq!(
            parse_command_args(r#"one "two three" 'four'"#),
            vec!["one", "two three", "four"]
        );
    }

    #[test]
    fn load_from_dir() {
        let dir = std::env::temp_dir().join(format!(
            "zi-prompts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("demo.md"),
            "---\ndescription: Demo prompt\n---\nHello $1\n",
        )
        .unwrap();
        let mut by_name = HashMap::new();
        let mut diagnostics = Vec::new();
        merge_dir(&dir, PromptSource::User, &mut by_name, &mut diagnostics);
        assert!(diagnostics.is_empty());
        let t = by_name.get("demo").unwrap();
        assert_eq!(t.description, "Demo prompt");
        assert_eq!(
            expand_prompt_template("/demo world", &[t.clone()]).as_deref(),
            Some("Hello world")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
