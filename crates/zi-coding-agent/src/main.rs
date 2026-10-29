//! zi CLI entry point.

mod cli;
mod context;
mod dev_ext;
mod env_file;
mod extensions;
mod keybindings;
mod mcp_bridge;
mod mcp_config;
mod model_registry;
mod modes;
mod packages;
mod prompts;
mod session_store;
mod settings;
mod skills;
mod slash;
mod system_prompt;
mod themes;
mod tools;
mod trust;
mod web;

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;
use zi_agent::AgentEvent;
use zi_ai::{AssistantContentBlock, AssistantMessageEvent, Message};
use zi_telemetry::NOOP_TELEMETRY_CONTEXT;
use zi_tui::{
    InteractiveExit, InteractiveOptions, InteractivePromptError, InteractiveSubmitResult,
    KeybindingOverrides, LiveUpdate, SlashSuggestion, StatusInfo,
};

use cli::{Cli, Command, OutputMode};
use modes::interactive::{boxed_prompt, run as run_interactive_tui};
use modes::{
    prepare_runtime, resolve_cwd, run_json_mode, run_print_mode, run_rpc_mode, run_web_mode,
    CliRuntime,
};
use session_store::resolve_agent_dir;
use slash::{autocomplete_hints, interactive_banner, try_handle_slash, SlashOutcome};

#[tokio::main]
async fn main() {
    let code = match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("zi: {err:#}");
            1
        }
    };
    std::process::exit(code);
}

async fn run() -> Result<i32> {
    let _telemetry = &NOOP_TELEMETRY_CONTEXT;
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cwd = resolve_cwd(cli.cwd.clone())?;
    env_file::load_env_files(&cwd, &resolve_agent_dir())?;

    if let Some(Command::Web(web_cli)) = &cli.command {
        return web::run_web_admin(web_cli);
    }

    let want_interactive = matches!(cli.mode, OutputMode::Interactive)
        || (matches!(cli.mode, OutputMode::Text)
            && cli.prompt_text().is_none()
            && std::io::stdin().is_terminal());

    let code = if want_interactive {
        run_interactive_cli(&cli).await?
    } else {
        match cli.mode {
            OutputMode::Text => run_print_mode(&cli).await?,
            OutputMode::Json => run_json_mode(&cli).await?,
            OutputMode::Rpc => run_rpc_mode(&cli).await?,
            OutputMode::Interactive => run_interactive_cli(&cli).await?,
            OutputMode::Web => run_web_mode(&cli).await?,
        }
    };

    Ok(code)
}

async fn run_interactive_cli(cli: &Cli) -> Result<i32> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("interactive mode requires a TTY");
    }

    let (runtime, model) = prepare_runtime(cli).await?;
    let banner = interactive_banner(&model, &runtime.registry, &runtime.theme);
    let slash_suggestions = autocomplete_hints(&runtime.cwd)
        .into_iter()
        .map(|(name, summary)| SlashSuggestion { name, summary })
        .collect();
    let cwd_display = runtime.cwd.display().to_string();
    let model_id = model.id.clone();
    let runtime = Arc::new(runtime);
    let abort_runtime = Arc::clone(&runtime);

    let kb = &runtime.keybindings;
    let options = InteractiveOptions {
        banner: Some(banner),
        slash_suggestions,
        cwd: Some(cwd_display.clone()),
        model: Some(model_id.clone()),
        on_abort: Some(Arc::new(move || {
            abort_runtime.agent.abort();
        })),
        keybindings: KeybindingOverrides {
            abort: kb
                .chords_for(crate::keybindings::ACTION_ABORT)
                .first()
                .map(|s| (*s).to_string()),
            quit: kb
                .chords_for(crate::keybindings::ACTION_QUIT)
                .first()
                .map(|s| (*s).to_string()),
            copy: kb
                .chords_for(crate::keybindings::ACTION_COPY)
                .first()
                .map(|s| (*s).to_string()),
        },
        redraw_counter: Some(Arc::clone(&runtime.tui_redraws)),
    };

    let exit = run_interactive_tui(
        options,
        boxed_prompt({
            let runtime = Arc::clone(&runtime);
            let cwd_display = cwd_display.clone();
            move |line, live_tx| {
                let runtime = Arc::clone(&runtime);
                let cwd_display = cwd_display.clone();
                async move {
                    if let Some(outcome) = try_handle_slash(&line, &runtime).await {
                        match outcome {
                            SlashOutcome::Prompt(expanded) => {
                                return run_prompt_streaming(&runtime, &expanded, live_tx)
                                    .await
                                    .map_err(|e| InteractivePromptError::Message(e.to_string()));
                            }
                            other => {
                                let model = runtime.agent.model().await;
                                let _ = live_tx.send(LiveUpdate::Status(StatusInfo {
                                    cwd: cwd_display,
                                    model: model.id,
                                    busy: false,
                                    queue_len: 0,
                                    extra: String::new(),
                                }));
                                return Ok(match other {
                                    SlashOutcome::Notice(text) => {
                                        InteractiveSubmitResult::System(text)
                                    }
                                    SlashOutcome::ResetUi {
                                        notice,
                                        seed,
                                        widget,
                                    } => {
                                        let _ = live_tx.send(LiveUpdate::Widget(widget));
                                        InteractiveSubmitResult::Reset {
                                            notice,
                                            seed: seed
                                                .into_iter()
                                                .map(|s| (s.role, s.content))
                                                .collect(),
                                        }
                                    }
                                    SlashOutcome::Quit => InteractiveSubmitResult::Quit,
                                    SlashOutcome::Prompt(_) => unreachable!(),
                                });
                            }
                        }
                    }

                    run_prompt_streaming(&runtime, &line, live_tx)
                        .await
                        .map_err(|e| InteractivePromptError::Message(e.to_string()))
                }
            }
        }),
    )
    .await?;

    Ok(match exit {
        InteractiveExit::Quit | InteractiveExit::Aborted => 0,
    })
}

async fn run_prompt_streaming(
    runtime: &CliRuntime,
    prompt: &str,
    live_tx: mpsc::UnboundedSender<LiveUpdate>,
) -> Result<InteractiveSubmitResult, anyhow::Error> {
    let session_name = runtime.session.lock().await.name().map(str::to_string);
    let before_payload = serde_json::json!({
        "prompt": prompt,
        "session_name": session_name,
    });
    let wasm_effects = runtime.extensions.dispatch_hook(
        crate::extensions::ExtensionHook::BeforeAgentStart,
        &before_payload,
    );
    apply_extension_ui_effects(&live_tx, &wasm_effects);
    if let Some(name) = &wasm_effects.session_name {
        let _ = runtime.session.lock().await.set_name(name.clone());
    }
    let pack_effects = runtime
        .packs
        .read()
        .expect("packs lock")
        .dispatch_hook(
            crate::extensions::ExtensionHook::BeforeAgentStart,
            &before_payload,
        );
    apply_pack_effects_sync(&live_tx, &pack_effects);
    if let Some(name) = &pack_effects.session_name {
        let _ = runtime.session.lock().await.set_name(name.clone());
    }
    if let Some(match_) = pack_effects.prompt_url_match.clone() {
        let cwd = runtime.cwd.clone();
        let live_meta = live_tx.clone();
        let session = runtime.session.clone();
        tokio::spawn(async move {
            let meta = crate::dev_ext::prompt_url::fetch_gh_metadata(
                match_.kind,
                &match_.target,
                &cwd,
            )
            .await;
            let _ = live_meta.send(LiveUpdate::Widget(Some(
                crate::dev_ext::prompt_url::format_widget(&match_, Some(&meta)),
            )));
            let current = session.lock().await.name().map(str::to_string);
            if crate::dev_ext::prompt_url::should_update_session_name(
                current.as_deref(),
                &match_,
                Some(&meta),
            ) {
                let name =
                    crate::dev_ext::prompt_url::desired_session_name(&match_, Some(&meta));
                let _ = session.lock().await.set_name(name);
            }
        });
    }

    let mut rx = runtime.agent.subscribe();
    let session = runtime.session.clone();
    let status_cwd = runtime.cwd.display().to_string();
    let model = runtime.agent.model().await;
    let _ = live_tx.send(LiveUpdate::Status(StatusInfo {
        cwd: status_cwd.clone(),
        model: model.id.clone(),
        busy: true,
        queue_len: 0,
        extra: String::new(),
    }));

    let live_listener = live_tx.clone();
    let packs = Arc::clone(&runtime.packs);
    let extensions = Arc::clone(&runtime.extensions);
    let tps_start = std::sync::Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
    let tps_start_listener = Arc::clone(&tps_start);
    let listener = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Err(err) = modes::handle_event(&session, &event).await {
                tracing::warn!("session persist failed: {err:#}");
            }
            match &event {
                AgentEvent::AgentStart => {
                    if let Ok(mut guard) = tps_start_listener.lock() {
                        *guard = Some(std::time::Instant::now());
                    }
                    let payload = serde_json::json!({});
                    let wasm_effects = extensions.dispatch_hook(
                        crate::extensions::ExtensionHook::AgentStart,
                        &payload,
                    );
                    apply_extension_ui_effects(&live_listener, &wasm_effects);
                    if let Ok(packs) = packs.read() {
                        let effects = packs.dispatch_hook(
                            crate::extensions::ExtensionHook::AgentStart,
                            &payload,
                        );
                        apply_pack_effects_sync(&live_listener, &effects);
                    }
                }
                AgentEvent::AgentEnd { messages } => {
                    let started = tps_start_listener
                        .lock()
                        .ok()
                        .and_then(|mut g| g.take());
                    let elapsed_ms = started
                        .map(|s| s.elapsed().as_millis() as u64)
                        .unwrap_or(0);
                    let usage = crate::dev_ext::tps::sum_turn_usage(messages);
                    let payload = serde_json::json!({
                        "elapsed_ms": elapsed_ms,
                        "input": usage.input,
                        "output": usage.output,
                        "cache_read": usage.cache_read,
                        "cache_write": usage.cache_write,
                        "total_tokens": usage.total_tokens,
                    });
                    let wasm_effects = extensions
                        .dispatch_hook(crate::extensions::ExtensionHook::AgentEnd, &payload);
                    apply_extension_ui_effects(&live_listener, &wasm_effects);
                    if let Ok(packs) = packs.read() {
                        let effects = packs.dispatch_hook(
                            crate::extensions::ExtensionHook::AgentEnd,
                            &payload,
                        );
                        apply_pack_effects_sync(&live_listener, &effects);
                    }
                    // End of this turn — leave so the next prompt does not get a
                    // second subscriber (drop(JoinHandle) does not cancel tasks).
                    break;
                }
                AgentEvent::MessageUpdate {
                    message,
                    assistant_message_event,
                    ..
                } => {
                    if let Message::Assistant(a) = message {
                        push_assistant_partial(&live_listener, a, assistant_message_event);
                    }
                }
                AgentEvent::MessageEnd { message, .. } => {
                    if let Message::Assistant(a) = message {
                        let text = assistant_text(a);
                        if !text.is_empty() {
                            let _ = live_listener.send(LiveUpdate::AssistantText(text));
                        }
                        let thinking = assistant_thinking(a);
                        if !thinking.is_empty() {
                            let _ = live_listener.send(LiveUpdate::Thinking(thinking));
                        }
                    }
                }
                AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name,
                    args,
                } => {
                    let body = format!("running…\n{args}");
                    let _ = live_listener.send(LiveUpdate::Tool {
                        id: tool_call_id.clone(),
                        name: tool_name.clone(),
                        body,
                    });
                }
                AgentEvent::ToolExecutionUpdate {
                    tool_call_id,
                    tool_name,
                    partial_result,
                    ..
                } => {
                    let body = tool_result_text(partial_result);
                    let _ = live_listener.send(LiveUpdate::Tool {
                        id: tool_call_id.clone(),
                        name: tool_name.clone(),
                        body,
                    });
                }
                AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name,
                    result,
                    is_error,
                    ..
                } => {
                    let mut body = tool_result_text(result);
                    if *is_error {
                        body = format!("error\n{body}");
                    }
                    let _ = live_listener.send(LiveUpdate::Tool {
                        id: tool_call_id.clone(),
                        name: tool_name.clone(),
                        body,
                    });
                }
                _ => {}
            }
        }
    });

    let prompt_result = runtime
        .agent
        .prompt(zi_agent::AgentPromptInput::Text(prompt))
        .await;

    // Wait for AgentEnd handling; abort if the loop errored without AgentEnd.
    modes::join_or_abort_listener(listener).await;

    let model = runtime.agent.model().await;
    let _ = live_tx.send(LiveUpdate::Status(StatusInfo {
        cwd: status_cwd,
        model: model.id,
        busy: false,
        queue_len: 0,
        extra: String::new(),
    }));

    if let Err(err) = prompt_result {
        let _ = live_tx.send(LiveUpdate::Error(err.to_string()));
        return Ok(InteractiveSubmitResult::System(format!("error: {err}")));
    }

    match latest_turn_display(runtime).await {
        TurnDisplay::Text(text) => {
            let _ = live_tx.send(LiveUpdate::Done);
            Ok(InteractiveSubmitResult::Assistant(text))
        }
        TurnDisplay::Error(err) => {
            let _ = live_tx.send(LiveUpdate::Error(err.clone()));
            Ok(InteractiveSubmitResult::System(format!("error: {err}")))
        }
        TurnDisplay::Empty => {
            let _ = live_tx.send(LiveUpdate::Done);
            Ok(InteractiveSubmitResult::Assistant(String::new()))
        }
    }
}

fn apply_pack_effects_sync(
    live: &mpsc::UnboundedSender<LiveUpdate>,
    effects: &crate::dev_ext::PackHookEffects,
) {
    for notice in &effects.notices {
        let _ = live.send(LiveUpdate::System(notice.clone()));
    }
    if let Some(widget) = &effects.widget {
        let _ = live.send(LiveUpdate::Widget(widget.clone()));
    }
}

fn apply_extension_ui_effects(
    live: &mpsc::UnboundedSender<LiveUpdate>,
    effects: &crate::extensions::ExtensionUiEffects,
) {
    for notice in &effects.notices {
        let _ = live.send(LiveUpdate::System(notice.clone()));
    }
    if let Some(widget) = &effects.widget {
        let _ = live.send(LiveUpdate::Widget(widget.clone()));
    }
}

fn push_assistant_partial(
    live: &mpsc::UnboundedSender<LiveUpdate>,
    a: &zi_ai::AssistantMessage,
    event: &AssistantMessageEvent,
) {
    match event {
        AssistantMessageEvent::TextDelta { .. }
        | AssistantMessageEvent::TextStart { .. }
        | AssistantMessageEvent::TextEnd { .. } => {
            let text = assistant_text(a);
            if !text.is_empty() {
                let _ = live.send(LiveUpdate::AssistantText(text));
            }
        }
        AssistantMessageEvent::ThinkingDelta { .. }
        | AssistantMessageEvent::ThinkingStart { .. }
        | AssistantMessageEvent::ThinkingEnd { .. } => {
            let thinking = assistant_thinking(a);
            if !thinking.is_empty() {
                let _ = live.send(LiveUpdate::Thinking(thinking));
            }
        }
        _ => {}
    }
}

fn assistant_text(a: &zi_ai::AssistantMessage) -> String {
    a.content
        .iter()
        .filter_map(|b| match b {
            AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn assistant_thinking(a: &zi_ai::AssistantMessage) -> String {
    a.content
        .iter()
        .filter_map(|b| match b {
            AssistantContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn tool_result_text(result: &zi_agent::AgentToolResult) -> String {
    result
        .content
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

enum TurnDisplay {
    Text(String),
    Error(String),
    Empty,
}

async fn latest_turn_display(runtime: &CliRuntime) -> TurnDisplay {
    use zi_ai::StopReason;
    let messages = runtime.agent.messages().await;
    let Some(Message::Assistant(a)) = messages.last() else {
        return TurnDisplay::Empty;
    };
    let text = assistant_text(a);
    if matches!(a.stop_reason, StopReason::Error | StopReason::Aborted) {
        if let Some(err) = a.error_message.as_deref().filter(|s| !s.is_empty()) {
            return TurnDisplay::Error(err.to_string());
        }
        if !text.is_empty() {
            return TurnDisplay::Error(text);
        }
        return TurnDisplay::Error("agent ended with error or abort".into());
    }
    if text.is_empty() {
        TurnDisplay::Empty
    } else {
        TurnDisplay::Text(text)
    }
}

