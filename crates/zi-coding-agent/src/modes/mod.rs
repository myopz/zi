//! CLI output modes (print, json, rpc, interactive, web).

mod json;
pub mod interactive;
mod print;
mod rpc;
mod web;

pub use json::run_json_mode;
pub use print::run_print_mode;
pub use rpc::run_rpc_mode;
pub use web::run_web_mode;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use zi_agent::{
    Agent, AgentEvent, AgentMessage, AgentOptions, AgentPromptInput, DefaultStreamFn,
};
use zi_ai::{
    claude_sonnet_4, get_env_api_key, Message, Model, SimpleStreamOptions, StreamOptions,
    SystemContent, SystemMessage, SystemRole,
};
use tokio::sync::Mutex;

use crate::dev_ext::{discover_enabled_dev_ext, seed_default_builtin_manifests, PackHost};
use crate::extensions::{ExtensionHook, ExtensionHost};
use crate::keybindings::{load_keybindings, Keybindings};
use crate::mcp_config::{connect_mcp_servers, McpRuntime};
use crate::model_registry::ModelRegistry;
use crate::session_store::{resolve_agent_dir, SessionStore};
use crate::system_prompt::coding_agent_system_prompt;
use crate::themes::{load_themes, resolve_theme, Theme};
use crate::tools::default_tools;
use crate::trust::is_project_trusted;

/// Shared agent + session state for a CLI run.
pub struct CliRuntime {
    pub agent: Agent,
    pub session: Arc<Mutex<SessionStore>>,
    pub cwd: PathBuf,
    pub registry: ModelRegistry,
    pub mcp: McpRuntime,
    /// Extension host retained so lifecycle hooks can be dispatched.
    pub extensions: Arc<ExtensionHost>,
    pub theme: Theme,
    pub keybindings: Keybindings,
    /// Shared TUI full-redraw counter (updated by the interactive loop).
    pub tui_redraws: Arc<AtomicU64>,
    /// Host extension packs (reloadable via `/reload`).
    pub packs: Arc<std::sync::RwLock<PackHost>>,
}

/// Resolve workspace cwd (absolute when possible).
pub fn resolve_cwd(explicit: Option<PathBuf>) -> Result<PathBuf> {
    let cwd =
        explicit.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(std::fs::canonicalize(&cwd).unwrap_or(cwd))
}

/// Load composed catalog + `models.json` from the agent dir.
pub fn load_model_registry() -> Result<ModelRegistry> {
    ModelRegistry::load(&resolve_agent_dir())
}

/// Pick model: `--model` → saved `defaultModel` → first with a key → Claude.
pub fn resolve_model(model_id: Option<&str>, registry: &ModelRegistry) -> Result<Model> {
    if let Some(id) = model_id {
        return registry
            .find(id)
            .cloned()
            .with_context(|| format!("unknown model id `{id}` (check catalog and ~/.zi/agent/models.json)"));
    }
    let remembered = crate::settings::load_settings().default_model;
    if let Some(id) = remembered.as_deref() {
        if let Some(model) = registry.find(id).cloned() {
            return Ok(model);
        }
        tracing::warn!(
            model = id,
            "saved defaultModel not in registry; falling back"
        );
    }
    for model in &registry.models {
        if registry.api_key_for(&model.provider).is_some()
            || get_env_api_key(&model.provider, None).is_some()
        {
            return Ok(model.clone());
        }
    }
    Ok(claude_sonnet_4())
}

/// Prefer models.json key, then process env for the model's provider.
pub fn resolve_api_key(model: &Model, registry: &ModelRegistry) -> Option<String> {
    registry
        .api_key_for(&model.provider)
        .map(str::to_string)
        .or_else(|| get_env_api_key(&model.provider, None))
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fresh system message for a workspace cwd (TUI and web share this).
pub fn system_message(cwd: &Path) -> Message {
    Message::System(SystemMessage {
        role: SystemRole::System,
        content: SystemContent::Text(coding_agent_system_prompt(cwd)),
        sections: None,
        tools_added: None,
        tools_removed: None,
        timestamp: wall_now_ms(),
    })
}

/// Build agent, tools, and optional session store.
pub async fn build_runtime(
    cwd: PathBuf,
    model: Model,
    no_session: bool,
    api_key: Option<String>,
    registry: ModelRegistry,
) -> Result<CliRuntime> {
    let base_tools = default_tools(cwd.clone());
    let agent_dir = resolve_agent_dir();
    let trusted = is_project_trusted(&cwd);
    if crate::trust::project_has_trust_resources(&cwd) && !trusted {
        tracing::warn!(
            "project `.zi` resources present but not trusted; run /trust to enable project skills/prompts/mcp/extensions"
        );
    }

    let mut extension_host = ExtensionHost::new(&cwd);
    extension_host.load_builtins(&base_tools);
    extension_host.load_wasm_extensions(&cwd, &agent_dir, trusted);
    for err in &extension_host.registry().errors {
        tracing::warn!(
            path = %err.path.display(),
            "extension load failed: {}",
            err.message
        );
    }
    for warn in &extension_host.registry().warnings {
        tracing::warn!("extension: {warn}");
    }
    for ext in extension_host.extensions() {
        tracing::debug!(id = ext.id(), "extension registered");
    }

    let (mcp, mcp_tools) = connect_mcp_servers(&cwd, &agent_dir, trusted).await;
    for status in &mcp.statuses {
        if status.ok {
            tracing::info!(
                server = %status.name,
                tools = status.tool_count,
                "MCP server connected"
            );
        } else {
            tracing::warn!(server = %status.name, "MCP connect failed: {}", status.detail);
        }
    }

    let mut tools = extension_host.merge_tools(base_tools);
    let mut seen: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name().to_string()).collect();
    for tool in mcp_tools {
        if seen.insert(tool.name().to_string()) {
            tools.push(tool);
        }
    }

    let settings = crate::settings::load_settings();
    let theme_extra = crate::packages::theme_paths_for(&cwd, &agent_dir, trusted);
    let themes = load_themes(&cwd, &agent_dir, trusted, &theme_extra);
    let theme_name = settings.theme.as_deref().unwrap_or("dark");
    let theme = resolve_theme(&themes, theme_name);
    let keybindings = load_keybindings();
    let seeded = seed_default_builtin_manifests(&cwd, &agent_dir, trusted);
    if seeded > 0 {
        tracing::info!(
            count = seeded,
            path = %agent_dir.join("extensions").display(),
            "seeded default builtin extension manifests outside source tree"
        );
    }
    let enabled = discover_enabled_dev_ext(&cwd, &agent_dir, trusted);
    for src in &enabled.sources {
        tracing::debug!(path = %src.display(), "extension manifest");
    }
    let packs = PackHost::from_enabled(&enabled);
    tracing::info!("{}", packs.summary_line());
    for ext in enabled.enabled_contributes() {
        if !ext.contributes.prompts.is_empty() || !ext.contributes.skills.is_empty() {
            tracing::debug!(
                id = %ext.id,
                prompts = ?ext.contributes.prompts,
                skills = ?ext.contributes.skills,
                "extension contributes"
            );
        }
    }

    let initial_messages: Vec<AgentMessage> = vec![system_message(&cwd)];

    let mut options = AgentOptions::new(model, Arc::new(DefaultStreamFn));
    options.tools = tools;
    options.initial_messages = initial_messages.clone();
    options.stream_options = SimpleStreamOptions {
        stream: StreamOptions {
            api_key,
            ..Default::default()
        },
        ..Default::default()
    };

    let agent = Agent::new(options);
    let session = SessionStore::open(&cwd, no_session).context("open session")?;

    let session_payload = serde_json::json!({
        "cwd": cwd.display().to_string(),
        "trusted": trusted,
        "session_name": session.name(),
        "messages": initial_messages,
    });
    extension_host.dispatch_hook(ExtensionHook::SessionStart, &session_payload);
    let _ = packs.dispatch_hook(ExtensionHook::SessionStart, &session_payload);

    Ok(CliRuntime {
        agent,
        session: Arc::new(Mutex::new(session)),
        cwd,
        registry,
        mcp,
        extensions: Arc::new(extension_host),
        theme,
        keybindings,
        tui_redraws: Arc::new(AtomicU64::new(0)),
        packs: Arc::new(std::sync::RwLock::new(packs)),
    })
}

/// Shared setup: cwd, registry, model, api key, runtime.
pub async fn prepare_runtime(cli: &crate::cli::Cli) -> Result<(CliRuntime, Model)> {
    let cwd = resolve_cwd(cli.cwd.clone())?;
    let registry = load_model_registry()?;
    let model = resolve_model(cli.model.as_deref(), &registry)?;
    // Remember the model used for this run (CLI flag or restored default).
    if let Err(err) = crate::settings::save_default_model(&model.id) {
        tracing::warn!("failed to persist defaultModel: {err:#}");
    }
    let api_key = resolve_api_key(&model, &registry);
    let runtime =
        build_runtime(cwd, model.clone(), cli.no_session, api_key, registry).await?;
    Ok((runtime, model))
}

/// Wait for a per-prompt agent event listener to finish; abort if it stalls.
///
/// `drop(JoinHandle)` does **not** cancel the task. Leaked listeners keep
/// receiving the next turn's broadcast events and double-append to the session.
pub async fn join_or_abort_listener(mut handle: tokio::task::JoinHandle<()>) {
    tokio::select! {
        _ = &mut handle => {}
        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
            handle.abort();
            let _ = handle.await;
        }
    }
}

/// Run one user prompt to completion (persists session messages).
pub async fn run_prompt(runtime: &CliRuntime, prompt: &str) -> Result<i32> {
    tracing::debug!(cwd = %runtime.cwd.display(), "running prompt");
    runtime.extensions.dispatch_hook(
        ExtensionHook::BeforeAgentStart,
        &serde_json::json!({ "prompt": prompt }),
    );
    let mut rx = runtime.agent.subscribe();
    let session = runtime.session.clone();
    let agent = &runtime.agent;

    let listener = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Err(err) = handle_event(&session, &event).await {
                tracing::warn!("session persist failed: {err:#}");
            }
            if matches!(event, AgentEvent::AgentEnd { .. }) {
                break;
            }
        }
    });

    agent
        .prompt(AgentPromptInput::Text(prompt))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    agent.wait_for_idle().await;
    join_or_abort_listener(listener).await;

    let messages = agent.messages().await;
    let Some(Message::Assistant(assistant)) = messages.last() else {
        return Ok(0);
    };

    use zi_ai::StopReason;
    if matches!(
        assistant.stop_reason,
        StopReason::Error | StopReason::Aborted
    ) {
        return Ok(1);
    }
    Ok(0)
}

pub async fn handle_event(
    session: &Arc<Mutex<SessionStore>>,
    event: &AgentEvent,
) -> Result<()> {
    if let AgentEvent::MessageEnd { message, .. } = event {
        let mut store = session.lock().await;
        store.append_message(message)?;
    }
    Ok(())
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use zi_ai::{KnownApi, ModelCost};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn registry_with(models: Vec<Model>) -> ModelRegistry {
        ModelRegistry {
            models,
            provider_api_keys: HashMap::new(),
        }
    }

    #[test]
    fn unknown_model_errors() {
        let reg = registry_with(vec![]);
        let err = resolve_model(Some("no-such-model"), &reg).unwrap_err();
        assert!(err.to_string().contains("unknown model id"), "{err:#}");
    }

    #[test]
    fn picks_explicit_model() {
        let model = Model {
            id: "custom-1".into(),
            name: "Custom".into(),
            api: KnownApi::OpenaiCompletions.as_str().into(),
            provider: "custom".into(),
            base_url: "https://example.com/v1".into(),
            reasoning: false,
            context_window: 128_000,
            max_tokens: 8_192,
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        };
        let reg = registry_with(vec![model.clone()]);
        let got = resolve_model(Some("custom-1"), &reg).unwrap();
        assert_eq!(got.id, "custom-1");
    }

    #[tokio::test]
    async fn join_or_abort_stops_leaked_listener() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&finished);
        let handle = tokio::spawn(async move {
            // Simulate the old bug: a listener that never exits on its own.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            flag.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        join_or_abort_listener(handle).await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "listener should be aborted promptly"
        );
        assert!(
            !finished.load(Ordering::SeqCst),
            "aborted task must not run to completion"
        );
    }
}
