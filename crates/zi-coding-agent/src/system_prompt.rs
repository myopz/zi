//! Default system prompt for the coding agent CLI.

use std::path::Path;

use crate::context::{format_context_for_prompt, load_agent_context};
use crate::session_store::resolve_agent_dir;
use crate::skills::{format_skills_for_prompt, load_skills, LoadSkillsOptions, SkillSource};
use crate::trust::is_project_trusted;

/// Stable system instructions + variable session context (AGENTS/skills/append).
#[derive(Debug, Clone)]
pub struct CodingAgentPromptParts {
    pub stable_system: String,
    pub variable_context: String,
}

/// Short coding-agent system prompt including workspace cwd, context files, and skills.
pub fn coding_agent_system_prompt(cwd: &Path) -> String {
    let parts = coding_agent_prompt_parts(cwd);
    if parts.variable_context.is_empty() {
        parts.stable_system
    } else {
        format!("{}\n\n{}", parts.stable_system, parts.variable_context)
    }
}

/// Split prompt into a stable system prefix and variable session context.
pub fn coding_agent_prompt_parts(cwd: &Path) -> CodingAgentPromptParts {
    let agent_dir = resolve_agent_dir();
    let trusted = is_project_trusted(cwd);
    coding_agent_prompt_parts_with(&agent_dir, cwd, trusted)
}

/// Build prompt parts with explicit agent dir and trust flag.
pub fn coding_agent_prompt_parts_with(
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> CodingAgentPromptParts {
    let ctx = load_agent_context(cwd, agent_dir, trusted);

    let stable_system = if let Some((_, system)) = &ctx.system_md {
        system.clone()
    } else {
        let cwd_display = cwd.to_string_lossy();
        format!(
            "You are zi, a coding agent helping the user in `{cwd_display}`.\n\
             Always answer the user. For greetings, thanks, and other chitchat, reply briefly\n\
             in plain text without tools — same as a normal conversation.\n\
             Use tools when inspecting or changing files.\n\
             Point lookups: read (one file), grep (content), find (glob), ls (one directory).\n\
             Broad / architecture / 'how does this work' when you do not know which files matter:\n\
             call the explore tool directly first (do not wrap it in codemode; do not use bash ls/find).\n\
             Prefer README and manifests. Do not scan the tree with many consecutive ls/find/grep\n\
             calls; after a handful, use explore or stop and answer. Point read of known paths is fine.\n\
             Architecture answers and written analysis docs must cite concrete paths from tool\n\
             results (file or file:symbol). After explore, read its Key paths before writing.\n\
             Cover Agent vs AgentSession vs Mode, and system/tool state in transcript\n\
             (sections / toolsAdded / toolsRemoved) when those files exist.\n\
             UNKNOWN only after a tool attempt failed or the path is confirmed missing;\n\
             unread is not UNKNOWN — do not invent structure.\n\
             bash is hard-blocked for ls/cat/find/rg/grep/sed/wc/head of source — use the tools\n\
             above instead. bash remains for git/npm/cargo/tests/install only.\n\
             Prefer small, focused edits. After a successful write/edit, do not keep re-editing\n\
             unless the user asks or a check fails. When done, stop. Be concise.\n\
             Paths may be relative to the workspace cwd or absolute.\n\
             Standing project rules may appear in <session-context>; read full AGENTS.md paths if truncated."
        )
    };

    let mut variable = String::new();
    let context_block = format_context_for_prompt(&ctx.context_files);
    if !context_block.is_empty() {
        variable.push_str(&context_block);
    }

    if let Some((_, append)) = &ctx.append_system_md {
        if !variable.is_empty() {
            variable.push_str("\n\n");
        }
        variable.push_str(append);
    }

    let mut extra_paths = crate::packages::skill_paths_for(cwd, agent_dir, trusted);
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        extra_paths.push(Path::new(&home).join(".agents").join("skills"));
    }
    if trusted {
        extra_paths.push(cwd.join(".agents").join("skills"));
    }

    let skills = load_skills(LoadSkillsOptions {
        cwd,
        agent_dir,
        include_defaults: true,
        include_project: trusted,
        extra_paths: &extra_paths,
    });
    for diag in &skills.diagnostics {
        tracing::warn!(path = %diag.path.display(), "{}", diag.message);
    }
    let skills: Vec<_> = skills
        .skills
        .into_iter()
        .map(|mut s| {
            if s.file_path
                .components()
                .any(|c| c.as_os_str() == ".agents")
            {
                s.source = SkillSource::Agents;
            }
            s
        })
        .collect();
    let skills_block = format_skills_for_prompt(&skills);
    if !skills_block.is_empty() {
        if !variable.is_empty() {
            variable.push_str("\n");
        }
        variable.push_str(&skills_block);
    }

    CodingAgentPromptParts {
        stable_system,
        variable_context: variable,
    }
}

/// Build system prompt with explicit agent dir and trust flag (joined for tests/compat).
#[allow(dead_code)]
pub fn coding_agent_system_prompt_with(agent_dir: &Path, cwd: &Path, trusted: bool) -> String {
    let parts = coding_agent_prompt_parts_with(agent_dir, cwd, trusted);
    if parts.variable_context.is_empty() {
        parts.stable_system
    } else {
        format!("{}\n\n{}", parts.stable_system, parts.variable_context)
    }
}
