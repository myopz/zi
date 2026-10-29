//! Default system prompt for the coding agent CLI.

use std::path::Path;

use crate::context::{format_context_for_prompt, load_agent_context};
use crate::session_store::resolve_agent_dir;
use crate::skills::{format_skills_for_prompt, load_skills, LoadSkillsOptions, SkillSource};
use crate::trust::is_project_trusted;

/// Short coding-agent system prompt including workspace cwd, context files, and skills.
pub fn coding_agent_system_prompt(cwd: &Path) -> String {
    let agent_dir = resolve_agent_dir();
    let trusted = is_project_trusted(cwd);
    coding_agent_system_prompt_with(&agent_dir, cwd, trusted)
}

/// Build system prompt with explicit agent dir and trust flag.
pub fn coding_agent_system_prompt_with(agent_dir: &Path, cwd: &Path, trusted: bool) -> String {
    let ctx = load_agent_context(cwd, agent_dir, trusted);

    let mut base = if let Some((_, system)) = &ctx.system_md {
        system.clone()
    } else {
        let cwd_display = cwd.to_string_lossy();
        format!(
            "You are zi, a coding agent. Work in the user's project at `{cwd_display}`.\n\
             Use the provided tools (read, bash, edit, write) to inspect and change files.\n\
             Prefer small, focused edits. Run shell commands via bash when needed.\n\
             Paths may be relative to the workspace cwd or absolute."
        )
    };

    let context_block = format_context_for_prompt(&ctx.context_files);
    if !context_block.is_empty() {
        base.push_str(&context_block);
    }

    if let Some((_, append)) = &ctx.append_system_md {
        base.push_str("\n\n");
        base.push_str(append);
    }

    let mut extra_paths = crate::packages::skill_paths_for(cwd, agent_dir, trusted);
    // Always scan Agent Skills spec locations (user home + project when trusted).
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
    // Remap Path source under .agents to Agents for prompt labels.
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
        base.push_str(&skills_block);
    }
    base
}
