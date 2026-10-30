//! Interactive slash commands (intercepted before the LLM).

use std::path::Path;

use zi_ai::{get_env_api_key, Model, SimpleStreamOptions, StreamOptions};

use crate::model_registry::ModelRegistry;
use crate::modes::{resolve_api_key, CliRuntime};
use crate::prompts::{expand_prompt_template, load_default_prompts};
use crate::skills::find_skill;
use crate::trust::{self, is_project_trusted};

/// One builtin slash command for help / autocomplete.
#[derive(Debug, Clone, Copy)]
pub struct SlashCommandMeta {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
}

/// Builtin commands.
pub const BUILTIN_SLASH_COMMANDS: &[SlashCommandMeta] = &[
    SlashCommandMeta {
        name: "model",
        aliases: &["models"],
        summary: "List or switch model (`/model` or `/model <id>`)",
    },
    SlashCommandMeta {
        name: "hotkeys",
        aliases: &["help"],
        summary: "Show keyboard shortcuts and slash commands",
    },
    SlashCommandMeta {
        name: "copy",
        aliases: &[],
        summary: "Copy last assistant reply to the clipboard",
    },
    SlashCommandMeta {
        name: "new",
        aliases: &[],
        summary: "Start a new session (clears transcript)",
    },
    SlashCommandMeta {
        name: "clear",
        aliases: &[],
        summary: "Clear conversation (keeps session file)",
    },
    SlashCommandMeta {
        name: "session",
        aliases: &[],
        summary: "Show current session path and stats",
    },
    SlashCommandMeta {
        name: "resume",
        aliases: &[],
        summary: "Resume a saved session (`/resume` or `/resume <id>`)",
    },
    SlashCommandMeta {
        name: "name",
        aliases: &[],
        summary: "Set or show the session display name",
    },
    SlashCommandMeta {
        name: "compact",
        aliases: &["summarize"],
        summary: "Compact context now (alias `/summarize`; auto at min(90% window, 48k tokens))",
    },
    SlashCommandMeta {
        name: "tree",
        aliases: &[],
        summary: "Show session entry tree",
    },
    SlashCommandMeta {
        name: "fork",
        aliases: &[],
        summary: "Fork the current session into a new branch",
    },
    SlashCommandMeta {
        name: "mcp",
        aliases: &[],
        summary: "Show MCP server status",
    },
    SlashCommandMeta {
        name: "trust",
        aliases: &[],
        summary: "Trust or untrust this project for `.zi` resources",
    },
    SlashCommandMeta {
        name: "theme",
        aliases: &["themes"],
        summary: "List or switch theme (`/theme` or `/theme <name>`)",
    },
    SlashCommandMeta {
        name: "reload",
        aliases: &[],
        summary: "Reload prompts/skills and host packs (WASM artifact swap needs restart)",
    },
    SlashCommandMeta {
        name: "extensions",
        aliases: &["extension"],
        summary: "List discovered host/WASM extensions and pack status",
    },
    SlashCommandMeta {
        name: "tui",
        aliases: &[],
        summary: "Show TUI full-redraw stats (parity with pi redraws extension)",
    },
    SlashCommandMeta {
        name: "ir",
        aliases: &[],
        summary: "Import a shared session (gist / issue / .jsonl); `/ir!` overwrites",
    },
    SlashCommandMeta {
        name: "quit",
        aliases: &["exit"],
        summary: "Exit interactive mode",
    },
];

/// One seeded transcript line after `/new` / `/clear` / `/resume`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedLine {
    pub role: zi_tui::MessageRole,
    pub content: String,
}

/// Result of handling a slash line locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashOutcome {
    /// Show as a system notice in the transcript.
    Notice(String),
    /// Clear the on-screen transcript, seed optional history, then show a notice.
    ResetUi {
        notice: String,
        seed: Vec<SeedLine>,
        /// Optional sticky prompt-url widget text.
        widget: Option<String>,
    },
    /// Expand a prompt template / skill and send to the agent.
    Prompt(String),
    /// End the interactive session.
    Quit,
}

/// Hint lines for the TUI autocomplete popup: `(name, summary)`.
pub fn autocomplete_hints(cwd: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for cmd in BUILTIN_SLASH_COMMANDS {
        out.push((cmd.name.to_string(), cmd.summary.to_string()));
        for alias in cmd.aliases {
            out.push(((*alias).to_string(), format!("alias for /{}", cmd.name)));
        }
    }

    let trusted = is_project_trusted(cwd);
    let prompts = load_default_prompts(cwd, trusted);
    for t in &prompts.templates {
        out.push((t.name.clone(), format!("prompt: {}", t.description)));
    }

    let skills = crate::skills::load_default_skills(cwd);
    for s in &skills.skills {
        out.push((
            format!("skill:{}", s.name),
            format!("skill: {}", s.description),
        ));
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn resolve_meta(name: &str) -> Option<&'static SlashCommandMeta> {
    let lower = name.to_ascii_lowercase();
    BUILTIN_SLASH_COMMANDS.iter().find(|cmd| {
        cmd.name == lower || cmd.aliases.iter().any(|a| *a == lower)
    })
}

/// If `line` is a slash command, handle it; otherwise return `None` (send to LLM).
pub async fn try_handle_slash(
    line: &str,
    runtime: &CliRuntime,
) -> Option<SlashOutcome> {
    let trimmed = line.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let rest = trimmed[1..].trim();
    if rest.is_empty() {
        return Some(SlashOutcome::Notice(hotkeys_text(runtime)));
    }

    // `/skill:name [args]` — inject skill body as the user prompt.
    if let Some(skill_rest) = rest.strip_prefix("skill:") {
        return Some(handle_skill(runtime, skill_rest).await);
    }

    // `/ir! <ref>` — force overwrite imported session.
    if let Some(ir_rest) = rest.strip_prefix("ir!") {
        if !pack_provides(runtime, "ir") {
            return Some(SlashOutcome::Notice(
                "`import-repro` extension is disabled (add `.zi/extensions/import-repro/extension.json`)."
                    .into(),
            ));
        }
        return Some(handle_ir(runtime, ir_rest.trim(), true).await);
    }

    let mut parts = rest.split_whitespace();
    let name = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();

    if let Some(meta) = resolve_meta(name) {
        return match meta.name {
            "quit" => Some(SlashOutcome::Quit),
            "hotkeys" => Some(SlashOutcome::Notice(hotkeys_text(runtime))),
            "model" => Some(handle_model(runtime, &args).await),
            "copy" => Some(handle_copy(runtime).await),
            "new" => Some(handle_new(runtime).await),
            "clear" => Some(handle_clear(runtime).await),
            "session" => Some(handle_session(runtime).await),
            "resume" => Some(handle_resume(runtime, &args).await),
            "name" => Some(handle_name(runtime, &args).await),
            "compact" => Some(handle_compact(runtime, &args).await),
            "tree" => Some(handle_tree(runtime).await),
            "fork" => Some(handle_fork(runtime).await),
            "mcp" => Some(SlashOutcome::Notice(runtime.mcp.summary())),
            "trust" => Some(handle_trust(runtime, &args).await),
            "theme" => Some(handle_theme(runtime, &args).await),
            "reload" => Some(handle_reload(runtime).await),
            "extensions" => Some(handle_extensions(runtime)),
            "tui" => Some(if pack_provides(runtime, "tui") {
                handle_tui(runtime).await
            } else {
                SlashOutcome::Notice(
                    "`redraws` extension is disabled (add `.zi/extensions/redraws/extension.json`)."
                        .into(),
                )
            }),
            "ir" => Some(if pack_provides(runtime, "ir") {
                handle_ir(runtime, &args.join(" "), false).await
            } else {
                SlashOutcome::Notice(
                    "`import-repro` extension is disabled (add `.zi/extensions/import-repro/extension.json`)."
                        .into(),
                )
            }),
            _ => Some(SlashOutcome::Notice(format!(
                "command `/{}` is not implemented",
                meta.name
            ))),
        };
    }

    // WASM-registered slash commands.
    if let Some(text) = runtime.extensions.dispatch_command(name, &args.join(" ")) {
        return Some(SlashOutcome::Notice(text));
    }

    // Prompt templates → expand and send to agent.
    let trusted = is_project_trusted(&runtime.cwd);
    let prompts = load_prompts_with_contributes(runtime, trusted);
    if let Some(expanded) = expand_prompt_template(trimmed, &prompts.templates) {
        return Some(SlashOutcome::Prompt(expanded));
    }

    Some(SlashOutcome::Notice(format!(
        "unknown command `/{name}`. Try /hotkeys."
    )))
}

async fn handle_skill(runtime: &CliRuntime, rest: &str) -> SlashOutcome {
    let (name, args_str) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a),
        None => (rest, ""),
    };
    if name.is_empty() {
        return SlashOutcome::Notice("usage: /skill:<name> [args]".into());
    }
    let skills = load_skills_with_contributes(runtime);
    let Some(skill) = find_skill(&skills.skills, name) else {
        return SlashOutcome::Notice(format!(
            "unknown skill `{name}`. Skills: {}",
            skills
                .skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let args = crate::prompts::parse_command_args(args_str);
    let mut body = crate::prompts::substitute_args(&skill.body, &args);
    if body.trim().is_empty() {
        body = format!(
            "Follow the skill `{}` at {}.\n\n{}",
            skill.name,
            skill.file_path.display(),
            skill.description
        );
    } else {
        body = format!(
            "Follow the skill `{}` (base_dir: {}).\n\n{body}",
            skill.name,
            skill.base_dir.display()
        );
    }
    SlashOutcome::Prompt(body)
}

async fn handle_name(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    if args.is_empty() {
        let store = runtime.session.lock().await;
        let name = store.name().unwrap_or("(unnamed)");
        return SlashOutcome::Notice(format!("Session name: {name}"));
    }
    let name = args.join(" ");
    let mut store = runtime.session.lock().await;
    match store.set_name(&name) {
        Ok(()) => SlashOutcome::Notice(format!("Session named `{name}`")),
        Err(err) => SlashOutcome::Notice(format!("failed to set name: {err:#}")),
    }
}

async fn handle_compact(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    let Some(compacted) = runtime.agent.compact_messages().await else {
        return SlashOutcome::Notice(
            "Nothing to compact (transcript already within the keep window).".into(),
        );
    };
    let summary = if args.is_empty() {
        compacted.summary.clone()
    } else {
        format!("{}\n\n{}", args.join(" "), compacted.summary)
    };
    let kept_tail: Vec<_> = compacted
        .messages
        .iter()
        .filter(|m| !matches!(m, zi_ai::Message::System(_)))
        .filter(|m| match m {
            zi_ai::Message::User(u) => match &u.content {
                zi_ai::UserContent::Text(t) => !t.contains("<compaction>"),
                _ => true,
            },
            _ => true,
        })
        .cloned()
        .collect();
    // Slash runs outside the per-prompt event listener, so persist here.
    let mut store = runtime.session.lock().await;
    let first_kept = store.last_entry_id().unwrap_or("root").to_string();
    match store.append_compaction(&summary, first_kept, compacted.tokens_before) {
        Ok(id) => {
            for message in &kept_tail {
                if let Err(err) = store.append_message(message) {
                    return SlashOutcome::Notice(format!(
                        "Compacted (id={id}) but failed re-appending kept messages: {err:#}"
                    ));
                }
            }
            SlashOutcome::Notice(format!(
                "Compacted: dropped {} messages (≈{} → ≈{} tokens). entry={id}\n{summary}",
                compacted.dropped_messages, compacted.tokens_before, compacted.tokens_after
            ))
        }
        Err(err) => SlashOutcome::Notice(format!(
            "Memory compacted but session write failed: {err:#}\n{summary}"
        )),
    }
}

async fn handle_tree(runtime: &CliRuntime) -> SlashOutcome {
    let store = runtime.session.lock().await;
    match store.tree_summary() {
        Ok(text) => SlashOutcome::Notice(text),
        Err(err) => SlashOutcome::Notice(format!("tree failed: {err:#}")),
    }
}

async fn handle_fork(runtime: &CliRuntime) -> SlashOutcome {
    let messages = runtime.agent.messages().await;
    let mut store = runtime.session.lock().await;
    let forked = match store.fork(&runtime.cwd) {
        Ok(s) => s,
        Err(err) => return SlashOutcome::Notice(format!("fork failed: {err:#}")),
    };
    // Re-append current messages into the forked file for continuity.
    let mut forked = forked;
    for message in &messages {
        if let Err(err) = forked.append_message(message) {
            return SlashOutcome::Notice(format!("fork persist failed: {err:#}"));
        }
    }
    *store = forked;
    let path = store
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no path)".into());
    SlashOutcome::Notice(format!("Forked session.\n{path}"))
}

async fn handle_trust(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    let sub = args.first().copied().unwrap_or("status");
    match sub {
        "status" | "show" => {
            let trusted = is_project_trusted(&runtime.cwd);
            let decision = trust::trust_decision_for(&runtime.cwd);
            let detail = match decision {
                Some((path, true)) => format!("trusted via {}", path.display()),
                Some((path, false)) => format!("explicitly untrusted via {}", path.display()),
                None => "no decision stored".into(),
            };
            SlashOutcome::Notice(format!(
                "Project trust: {}\n{detail}\nUse `/trust yes` or `/trust no`. Restart after changing.",
                if trusted { "yes" } else { "no" }
            ))
        }
        "yes" | "true" | "on" | "trust" => match trust::set_project_trust(&runtime.cwd, true) {
            Ok(path) => SlashOutcome::Notice(format!(
                "Trusted {}. Restart zi (or open a new session) to load project `.zi` resources.",
                path.display()
            )),
            Err(err) => SlashOutcome::Notice(format!("trust failed: {err:#}")),
        },
        "no" | "false" | "off" | "untrust" => match trust::set_project_trust(&runtime.cwd, false)
        {
            Ok(path) => SlashOutcome::Notice(format!(
                "Marked {} as untrusted for project `.zi` resources.",
                path.display()
            )),
            Err(err) => SlashOutcome::Notice(format!("untrust failed: {err:#}")),
        },
        "clear" => match trust::clear_project_trust(&runtime.cwd) {
            Ok(()) => SlashOutcome::Notice("Cleared trust decision for this cwd.".into()),
            Err(err) => SlashOutcome::Notice(format!("clear trust failed: {err:#}")),
        },
        other => SlashOutcome::Notice(format!(
            "unknown `/trust {other}`. Use status|yes|no|clear."
        )),
    }
}

async fn handle_theme(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    let agent_dir = crate::session_store::resolve_agent_dir();
    let trusted = is_project_trusted(&runtime.cwd);
    let extra = crate::packages::theme_paths_for(&runtime.cwd, &agent_dir, trusted);
    let themes = crate::themes::load_themes(&runtime.cwd, &agent_dir, trusted, &extra);
    if args.is_empty() {
        let current = &runtime.theme.name;
        let mut lines = vec![format!("Current theme: {current}"), "Available:".into()];
        for t in &themes {
            let mark = if &t.name == current { "*" } else { " " };
            lines.push(format!(" {mark} {}", t.name));
        }
        lines.push("Switch: /theme <name> (saved; takes full effect next restart)".into());
        return SlashOutcome::Notice(lines.join("\n"));
    }
    let name = args[0];
    if !themes.iter().any(|t| t.name == name) {
        return SlashOutcome::Notice(format!("unknown theme `{name}`"));
    }
    match crate::settings::save_theme(name) {
        Ok(()) => SlashOutcome::Notice(format!(
            "Theme `{name}` saved. Restart interactive mode to apply colors fully."
        )),
        Err(err) => SlashOutcome::Notice(format!("save theme failed: {err:#}")),
    }
}

async fn handle_tui(runtime: &CliRuntime) -> SlashOutcome {
    let n = runtime
        .tui_redraws
        .load(std::sync::atomic::Ordering::Relaxed);
    SlashOutcome::Notice(format!("TUI full redraws: {n}"))
}

async fn handle_ir(runtime: &CliRuntime, args: &str, overwrite: bool) -> SlashOutcome {
    let ref_str = args.trim();
    if ref_str.is_empty() {
        return SlashOutcome::Notice(
            "Usage: /ir <gist-id | gist-url | pi.dev/session URL | issue URL | .jsonl URL>\n\
             Overwrite: /ir! <ref>"
                .into(),
        );
    }
    let parsed = match crate::dev_ext::import_repro::parse_ref(ref_str, &runtime.cwd) {
        Ok(p) => p,
        Err(err) => return SlashOutcome::Notice(format!("ir: {err:#}")),
    };
    let session_dir = crate::dev_ext::import_repro::session_dir_for_cwd(&runtime.cwd);
    let imported = match crate::dev_ext::import_repro::import_session(
        parsed,
        &runtime.cwd,
        &session_dir,
        overwrite,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("{err:#}");
            if msg.contains("already imported") {
                return SlashOutcome::Notice(format!(
                    "ir: {msg}\nRe-run as `/ir! {ref_str}` to overwrite."
                ));
            }
            return SlashOutcome::Notice(format!("ir: {msg}"));
        }
    };

    let (store, messages) =
        match crate::session_store::SessionStore::open_existing(&runtime.cwd, &imported.destination)
        {
            Ok(v) => v,
            Err(err) => {
                return SlashOutcome::Notice(format!(
                    "Imported {} but failed to open: {err:#}",
                    imported.destination.display()
                ));
            }
        };
    let mut loaded = messages;
    if loaded
        .first()
        .is_none_or(|m| !matches!(m, zi_ai::Message::System(_)))
    {
        loaded.insert(0, fresh_system_message(&runtime.cwd));
    }
    let seed = seed_from_messages(&loaded);
    maybe_apply_prompt_url_name(runtime, &loaded).await;
    let widget = prompt_url_widget_for_messages(runtime, &loaded);
    runtime.agent.replace_messages(loaded).await;
    *runtime.session.lock().await = store;

    let mut notice = format!(
        "Imported session {} (cwd {} -> {})\n{}",
        imported.header_id,
        imported.source_cwd,
        runtime.cwd.display(),
        imported.destination.display()
    );
    if let Some(platform) = imported.platform_notice {
        notice.push('\n');
        notice.push_str(&platform);
    }
    SlashOutcome::ResetUi {
        notice,
        seed,
        widget,
    }
}

fn pack_provides(runtime: &CliRuntime, command: &str) -> bool {
    runtime
        .packs
        .read()
        .map(|p| p.provides_command(command))
        .unwrap_or(false)
}

fn handle_extensions(runtime: &CliRuntime) -> SlashOutcome {
    let mut text = runtime
        .packs
        .read()
        .map(|p| p.list_text())
        .unwrap_or_else(|_| "extensions lock poisoned".into());
    let wasm_cmds = &runtime.extensions.registry().commands;
    if !wasm_cmds.is_empty() {
        text.push_str("\nWASM slash commands:");
        for cmd in wasm_cmds {
            text.push_str(&format!(
                "\n  /{} — {} ({})",
                cmd.name, cmd.summary, cmd.extension_id
            ));
        }
    }
    let wasm_exts: Vec<_> = runtime
        .extensions
        .extensions()
        .iter()
        .filter(|e| e.id().starts_with("wasm:"))
        .map(|e| e.id().to_string())
        .collect();
    if !wasm_exts.is_empty() {
        text.push_str(&format!("\nLoaded WASM modules: {}", wasm_exts.join(", ")));
    }
    SlashOutcome::Notice(text)
}

async fn handle_reload(runtime: &CliRuntime) -> SlashOutcome {
    let trusted = is_project_trusted(&runtime.cwd);
    let agent_dir = crate::session_store::resolve_agent_dir();
    let _ = crate::dev_ext::seed_default_builtin_manifests(&runtime.cwd, &agent_dir, trusted);
    let enabled = crate::dev_ext::discover_enabled_dev_ext(&runtime.cwd, &agent_dir, trusted);
    let packs = crate::dev_ext::PackHost::from_enabled(&enabled);
    let pack_summary = packs.summary_line();
    if let Ok(mut guard) = runtime.packs.write() {
        *guard = packs;
    }

    let prompts = load_prompts_with_contributes(runtime, trusted);
    let skills = load_skills_with_contributes(runtime);
    for d in &prompts.diagnostics {
        tracing::warn!(path = %d.path.display(), "prompt: {}", d.message);
    }
    for d in &skills.diagnostics {
        tracing::warn!(path = %d.path.display(), "skill: {}", d.message);
    }
    SlashOutcome::Notice(format!(
        "Reloaded host packs + resource index ({pack_summary}).\n\
         {} prompts, {} skills (project trusted={trusted}).\n\
         MCP reconnect and WASM module hot-swap require a process restart.",
        prompts.templates.len(),
        skills.skills.len()
    ))
}

fn extension_contribute_paths(
    runtime: &CliRuntime,
) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    let mut prompts = Vec::new();
    let mut skills = Vec::new();
    let Ok(packs) = runtime.packs.read() else {
        return (prompts, skills);
    };
    for ext in packs.discovered().iter().filter(|e| e.enabled) {
        for rel in &ext.contributes.prompts {
            let dir = ext.root.join(rel);
            if dir.is_dir() {
                prompts.push(dir);
            }
        }
        for rel in &ext.contributes.skills {
            let dir = ext.root.join(rel);
            if dir.is_dir() {
                skills.push(dir);
            }
        }
    }
    (prompts, skills)
}

fn load_prompts_with_contributes(
    runtime: &CliRuntime,
    trusted: bool,
) -> crate::prompts::LoadPromptsResult {
    let agent_dir = crate::session_store::resolve_agent_dir();
    let mut extra = if trusted {
        crate::packages::prompt_paths_for(&runtime.cwd, &agent_dir)
    } else {
        crate::packages::prompt_paths_user_only(&agent_dir)
    };
    let (contrib_prompts, _) = extension_contribute_paths(runtime);
    extra.extend(contrib_prompts);
    crate::prompts::load_prompt_templates(crate::prompts::LoadPromptsOptions {
        cwd: &runtime.cwd,
        agent_dir: &agent_dir,
        include_defaults: true,
        include_project: trusted,
        extra_paths: &extra,
    })
}

fn load_skills_with_contributes(runtime: &CliRuntime) -> crate::skills::LoadSkillsResult {
    let agent_dir = crate::session_store::resolve_agent_dir();
    let trusted = is_project_trusted(&runtime.cwd);
    let mut extra = crate::packages::skill_paths_for(&runtime.cwd, &agent_dir, trusted);
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        extra.push(std::path::PathBuf::from(home).join(".agents").join("skills"));
    }
    if trusted {
        extra.push(runtime.cwd.join(".agents").join("skills"));
    }
    let (_, contrib_skills) = extension_contribute_paths(runtime);
    extra.extend(contrib_skills);
    crate::skills::load_skills(crate::skills::LoadSkillsOptions {
        cwd: &runtime.cwd,
        agent_dir: &agent_dir,
        include_defaults: true,
        include_project: trusted,
        extra_paths: &extra,
    })
}

fn fresh_system_message(cwd: &std::path::Path) -> zi_ai::Message {
    use zi_ai::{Message, SystemContent, SystemMessage, SystemRole};
    Message::System(SystemMessage {
        role: SystemRole::System,
        content: SystemContent::Text(crate::system_prompt::coding_agent_system_prompt(cwd)),
        sections: None,
        tools_added: None,
        tools_removed: None,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    })
}

async fn handle_new(runtime: &CliRuntime) -> SlashOutcome {
    {
        let mut store = runtime.session.lock().await;
        if let Err(err) = store.restart(&runtime.cwd) {
            return SlashOutcome::Notice(format!("failed to start new session: {err:#}"));
        }
    }
    let messages = vec![fresh_system_message(&runtime.cwd)];
    runtime.agent.replace_messages(messages.clone()).await;
    let widget = dispatch_session_start(runtime, &messages).await;
    let path = runtime
        .session
        .lock()
        .await
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no session file)".into());
    SlashOutcome::ResetUi {
        notice: format!("New session started.\n{path}"),
        seed: Vec::new(),
        widget,
    }
}

async fn handle_clear(runtime: &CliRuntime) -> SlashOutcome {
    let messages = vec![fresh_system_message(&runtime.cwd)];
    runtime.agent.replace_messages(messages.clone()).await;
    let widget = dispatch_session_start(runtime, &messages).await;
    SlashOutcome::ResetUi {
        notice: "Conversation cleared (session file kept).".into(),
        seed: Vec::new(),
        widget,
    }
}

/// Rebuild host-pack session UI (prompt-url widget) after `/new` / `/clear`.
async fn dispatch_session_start(
    runtime: &CliRuntime,
    messages: &[zi_ai::Message],
) -> Option<String> {
    let session_name = runtime.session.lock().await.name().map(str::to_string);
    let payload = serde_json::json!({
        "cwd": runtime.cwd.display().to_string(),
        "session_name": session_name,
        "messages": messages,
    });
    let wasm_effects = runtime
        .extensions
        .dispatch_hook(crate::extensions::ExtensionHook::SessionStart, &payload);
    if let Some(name) = wasm_effects.session_name {
        let _ = runtime.session.lock().await.set_name(name);
    }
    let effects = runtime
        .packs
        .read()
        .expect("packs lock")
        .dispatch_hook(crate::extensions::ExtensionHook::SessionStart, &payload);
    if let Some(name) = effects.session_name {
        let _ = runtime.session.lock().await.set_name(name);
    }
    effects
        .widget
        .flatten()
        .or(wasm_effects.widget.flatten())
}

async fn handle_session(runtime: &CliRuntime) -> SlashOutcome {
    let store = runtime.session.lock().await;
    let path = store
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(disabled)".into());
    let id = store
        .header()
        .map(|h| h.id.clone())
        .unwrap_or_else(|| "-".into());
    let name = store.name().unwrap_or("-").to_string();
    let parent = store
        .parent_session_id()
        .unwrap_or("-")
        .to_string();
    drop(store);
    let n = runtime.agent.messages().await.len();
    let model = runtime.agent.model().await;
    SlashOutcome::Notice(format!(
        "Session\n  id: {id}\n  name: {name}\n  parent: {parent}\n  path: {path}\n  messages: {n}\n  model: {} ({})\n  theme: {}\n  trusted: {}\n  cwd: {}",
        model.id,
        model.provider,
        runtime.theme.name,
        is_project_trusted(&runtime.cwd),
        runtime.cwd.display()
    ))
}

async fn handle_resume(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    use crate::session_store::SessionStore;
    let files = match SessionStore::list_sessions(&runtime.cwd, 20) {
        Ok(f) => f,
        Err(err) => return SlashOutcome::Notice(format!("list sessions failed: {err:#}")),
    };
    if files.is_empty() {
        return SlashOutcome::Notice("No saved sessions for this cwd.".into());
    }
    let target = if let Some(query) = args.first().copied() {
        files
            .iter()
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(query))
            })
            .cloned()
    } else {
        let current = runtime.session.lock().await.path().map(|p| p.to_path_buf());
        files
            .iter()
            .find(|p| current.as_ref() != Some(*p))
            .cloned()
            .or(current)
    };
    let Some(path) = target else {
        return SlashOutcome::Notice(
            "No matching session. Use /resume <id-substring> or /session.".into(),
        );
    };
    let (store, messages) = match SessionStore::open_existing(&runtime.cwd, &path) {
        Ok(v) => v,
        Err(err) => return SlashOutcome::Notice(format!("resume failed: {err:#}")),
    };
    let mut loaded = messages;
    if loaded
        .first()
        .is_none_or(|m| !matches!(m, zi_ai::Message::System(_)))
    {
        loaded.insert(0, fresh_system_message(&runtime.cwd));
    }
    let seed = seed_from_messages(&loaded);
    let widget = prompt_url_widget_for_messages(runtime, &loaded);
    maybe_apply_prompt_url_name(runtime, &loaded).await;
    runtime.agent.replace_messages(loaded).await;
    *runtime.session.lock().await = store;
    SlashOutcome::ResetUi {
        notice: format!("Resumed {}", path.display()),
        seed,
        widget,
    }
}

fn prompt_url_widget_for_messages(
    runtime: &CliRuntime,
    messages: &[zi_ai::Message],
) -> Option<String> {
    let current = runtime
        .session
        .try_lock()
        .ok()
        .and_then(|s| s.name().map(str::to_string));
    let payload = serde_json::json!({
        "messages": messages,
        "session_name": current,
    });
    let effects = runtime
        .packs
        .read()
        .ok()?
        .dispatch_hook(crate::extensions::ExtensionHook::SessionSwitch, &payload);
    effects.widget.flatten()
}

async fn maybe_apply_prompt_url_name(runtime: &CliRuntime, messages: &[zi_ai::Message]) {
    let current = runtime.session.lock().await.name().map(str::to_string);
    let payload = serde_json::json!({
        "messages": messages,
        "session_name": current,
    });
    let effects = runtime
        .packs
        .read()
        .expect("packs lock")
        .dispatch_hook(crate::extensions::ExtensionHook::SessionSwitch, &payload);
    if let Some(name) = effects.session_name {
        let _ = runtime.session.lock().await.set_name(name);
    }
}

fn seed_from_messages(messages: &[zi_ai::Message]) -> Vec<SeedLine> {
    use zi_ai::{AssistantContentBlock, Message, UserContent};
    use zi_tui::MessageRole;
    let mut seed = Vec::new();
    for message in messages {
        match message {
            Message::User(u) => {
                let text = match &u.content {
                    UserContent::Text(t) => t.clone(),
                    UserContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|b| match b {
                            zi_ai::UserContentBlock::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                };
                if !text.is_empty() {
                    seed.push(SeedLine {
                        role: MessageRole::User,
                        content: text,
                    });
                }
            }
            Message::Assistant(a) => {
                let text = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    seed.push(SeedLine {
                        role: MessageRole::Assistant,
                        content: text,
                    });
                }
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        zi_ai::ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                seed.push(SeedLine {
                    role: MessageRole::Tool,
                    content: format!("{}: {text}", tr.tool_name),
                });
            }
            Message::System(_) => {}
        }
    }
    seed
}

async fn handle_copy(runtime: &CliRuntime) -> SlashOutcome {
    let text = latest_assistant_plain(runtime).await;
    if text.is_empty() {
        return SlashOutcome::Notice("Nothing to copy (no assistant reply yet).".into());
    }
    match zi_tui::clipboard_write(&text) {
        Ok(()) => SlashOutcome::Notice("Copied last reply to clipboard.".into()),
        Err(err) => SlashOutcome::Notice(format!("clipboard error: {err}")),
    }
}

async fn latest_assistant_plain(runtime: &CliRuntime) -> String {
    use zi_ai::{AssistantContentBlock, Message};
    let messages = runtime.agent.messages().await;
    messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant(a) => {
                let t = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
            _ => None,
        })
        .unwrap_or_default()
}

async fn handle_model(runtime: &CliRuntime, args: &[&str]) -> SlashOutcome {
    if let Some(id) = args.first().copied() {
        return switch_model(runtime, id).await;
    }
    SlashOutcome::Notice(list_models(runtime).await)
}

async fn list_models(runtime: &CliRuntime) -> String {
    let current = runtime.agent.model().await;
    let mut lines = vec![
        format!("Current: {} ({})", current.id, current.provider),
        "Available:".into(),
    ];
    for model in &runtime.registry.models {
        let mark = if model.id == current.id { "*" } else { " " };
        let key = if runtime.registry.api_key_for(&model.provider).is_some()
            || get_env_api_key(&model.provider, None).is_some()
        {
            "key"
        } else {
            "no-key"
        };
        lines.push(format!(
            " {mark} {}  [{} / {} / {key}]",
            model.id, model.provider, model.api
        ));
    }
    lines.push("Switch: /model <id>".into());
    lines.join("\n")
}

async fn switch_model(runtime: &CliRuntime, id: &str) -> SlashOutcome {
    let Some(model) = runtime.registry.find(id).cloned() else {
        return SlashOutcome::Notice(format!(
            "unknown model `{id}`. Use /model to list ids."
        ));
    };
    let api_key = resolve_api_key(&model, &runtime.registry);
    let stream_options = SimpleStreamOptions {
        stream: StreamOptions {
            api_key,
            ..Default::default()
        },
        ..Default::default()
    };
    let label = format!("{} ({})", model.id, model.provider);
    let id = model.id.clone();
    runtime.agent.set_model(model, stream_options).await;
    if let Err(err) = crate::settings::save_default_model(&id) {
        return SlashOutcome::Notice(format!(
            "Switched to {label} (warning: could not save default: {err:#})"
        ));
    }
    SlashOutcome::Notice(format!("Switched to {label} (saved as default)"))
}

fn hotkeys_text(runtime: &CliRuntime) -> String {
    let mut lines = vec![
        "Hotkeys:".into(),
        "  Enter         submit (queues while busy)".into(),
        "  Shift+Enter   newline (also Ctrl+J)".into(),
        "  Esc           abort running turn".into(),
        "  Ctrl+C        abort turn / quit when idle".into(),
        "  Ctrl+V        paste".into(),
        "  Ctrl+Y        copy last reply (also /copy)".into(),
        "  Ctrl+W        delete word backward".into(),
        "  Ctrl+U / K    kill to line start / end".into(),
        "  Ctrl+O        expand/collapse tool output".into(),
        "  Ctrl+T        show/hide thinking".into(),
        "  Wheel         scroll transcript".into(),
        "  PageUp/Down   scroll transcript".into(),
        "  Ctrl+Up/Down  scroll transcript".into(),
        "  (Option+drag to select text; Ctrl+Y copies last reply)".into(),
        "  Up/Down       history or slash suggestions".into(),
        "  Tab           complete /command or @path".into(),
        "".into(),
        "Keybindings (from keybindings.json):".into(),
    ];
    lines.extend(runtime.keybindings.help_lines());
    lines.push("".into());
    lines.push(format!("Theme: {}", runtime.theme.name));
    lines.push("".into());
    lines.push("Slash commands:".into());
    for cmd in BUILTIN_SLASH_COMMANDS {
        let aliases = if cmd.aliases.is_empty() {
            String::new()
        } else {
            format!(" (aliases: {})", cmd.aliases.join(", "))
        };
        lines.push(format!("  /{}{}  — {}", cmd.name, aliases, cmd.summary));
    }
    lines.push("  /skill:<name>  — run a skill as the user prompt".into());
    lines.push("  /<prompt>      — expand a prompt template".into());
    lines.join("\n")
}

/// Startup banner for interactive mode.
pub fn interactive_banner(
    model: &Model,
    registry: &ModelRegistry,
    theme: &crate::themes::Theme,
) -> String {
    let _ = registry;
    let accent = crate::themes::ansi_for_color(theme, "accent");
    let reset = crate::themes::ANSI_RESET;
    format!(
        "{accent}zi interactive{reset} · model {} ({}) · theme {}\nType / for commands · /model · /hotkeys · /trust · /quit",
        model.id, model.provider, theme.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autocomplete_includes_aliases() {
        let names: Vec<_> = autocomplete_hints(Path::new("."))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(names.iter().any(|n| n == "model"));
        assert!(names.iter().any(|n| n == "models"));
        assert!(names.iter().any(|n| n == "help"));
        assert!(names.iter().any(|n| n == "quit"));
        assert!(names.iter().any(|n| n == "mcp"));
        assert!(names.iter().any(|n| n == "trust"));
        assert!(names.iter().any(|n| n == "compact"));
        assert!(names.iter().any(|n| n == "summarize"));
    }

    #[test]
    fn resolve_aliases() {
        assert_eq!(resolve_meta("models").unwrap().name, "model");
        assert_eq!(resolve_meta("help").unwrap().name, "hotkeys");
        assert_eq!(resolve_meta("exit").unwrap().name, "quit");
        assert_eq!(resolve_meta("themes").unwrap().name, "theme");
        assert_eq!(resolve_meta("summarize").unwrap().name, "compact");
        assert!(resolve_meta("nope").is_none());
    }
}
