//! WebSocket: prompt / abort with streaming AgentEvent JSON.

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use zi_agent::{AgentEvent, AgentPromptInput};

use super::auth::{authenticate_token, AuthUser};
use super::state::AppState;
use crate::modes::join_or_abort_listener;

/// Upgrade to a WebSocket; Bearer header or first-frame `{type:"auth",token}` required.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let header_user = bearer_from_headers(&state, &headers);
    ws.on_upgrade(move |socket| handle_socket(socket, state, header_user))
}

fn bearer_from_headers(state: &AppState, headers: &HeaderMap) -> Option<AuthUser> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))?
        .trim();
    if token.is_empty() {
        return None;
    }
    authenticate_token(state, token).ok()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Auth { token: String },
    Prompt {
        #[serde(rename = "sessionId")]
        session_id: String,
        message: String,
        id: Option<String>,
    },
    Abort {
        #[serde(rename = "sessionId")]
        session_id: String,
        id: Option<String>,
    },
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    mut user: Option<AuthUser>,
) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();

    let writer = tokio::spawn(async move {
        while let Some(value) = out_rx.recv().await {
            let Ok(text) = serde_json::to_string(&value) else {
                continue;
            };
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            WsMessage::Close(_) => break,
            WsMessage::Binary(_) => {
                let _ = out_tx.send(json!({
                    "type": "response",
                    "command": "unknown",
                    "success": false,
                    "error": "binary frames not supported",
                }));
                continue;
            }
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: ClientMsg = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(err) => {
                let _ = out_tx.send(json!({
                    "type": "response",
                    "command": "unknown",
                    "success": false,
                    "error": format!("invalid command JSON: {err}"),
                }));
                continue;
            }
        };

        match parsed {
            ClientMsg::Auth { token } => match authenticate_token(&state, token.trim()) {
                Ok(u) => {
                    user = Some(u);
                    let _ = out_tx.send(json!({
                        "type": "response",
                        "command": "auth",
                        "success": true,
                    }));
                }
                Err(StatusCode::UNAUTHORIZED) => {
                    let _ = out_tx.send(json!({
                        "type": "response",
                        "command": "auth",
                        "success": false,
                        "error": "invalid token",
                    }));
                }
                Err(_) => {
                    let _ = out_tx.send(json!({
                        "type": "response",
                        "command": "auth",
                        "success": false,
                        "error": "auth failed",
                    }));
                }
            },
            ClientMsg::Prompt {
                session_id,
                message,
                id,
            } => {
                let Some(ref u) = user else {
                    let _ = out_tx.send(response_err(id, "prompt", "unauthorized"));
                    continue;
                };
                match run_prompt(&state, u, &session_id, &message, &out_tx).await {
                    Ok(()) => {
                        let _ = out_tx.send(response_ok(id, "prompt"));
                    }
                    Err(err) => {
                        let _ = out_tx.send(response_err(id, "prompt", &err));
                    }
                }
            }
            ClientMsg::Abort { session_id, id } => {
                let Some(ref u) = user else {
                    let _ = out_tx.send(response_err(id, "abort", "unauthorized"));
                    continue;
                };
                match run_abort(&state, u, &session_id).await {
                    Ok(()) => {
                        let _ = out_tx.send(response_ok(id, "abort"));
                    }
                    Err(err) => {
                        let _ = out_tx.send(response_err(id, "abort", &err));
                    }
                }
            }
        }
    }

    drop(out_tx);
    let _ = writer.await;
}

fn response_ok(id: Option<String>, command: &str) -> Value {
    json!({
        "type": "response",
        "id": id,
        "command": command,
        "success": true,
    })
}

fn response_err(id: Option<String>, command: &str, error: &str) -> Value {
    json!({
        "type": "response",
        "id": id,
        "command": command,
        "success": false,
        "error": error,
    })
}

async fn run_prompt(
    state: &AppState,
    user: &AuthUser,
    session_id: &str,
    message: &str,
    out_tx: &mpsc::UnboundedSender<Value>,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("message required".into());
    }
    state
        .db
        .get_session(&user.id, session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "session not found".to_string())?;

    // First real prompt: replace "New session" with a short title from the message.
    if !message.trim().starts_with('/') {
        if let Ok(Some(updated)) = state
            .db
            .maybe_autotitle_session(&user.id, session_id, message)
        {
            let _ = out_tx.send(json!({
                "type": "session_meta",
                "sessionId": session_id,
                "revision": updated.revision,
                "title": updated.title,
                "model": updated.model.clone(),
            }));
        }
    }

    let handle = state
        .runtimes
        .get_or_create(state, &user.id, session_id)
        .await
        .map_err(|e| e.to_string())?;

    let rt = handle.runtime.lock().await;
    if rt.agent.is_running() {
        return Err("agent already running".into());
    }

    // Same as TUI: `/…` is handled locally and must not hit the LLM.
    let mut prompt_text = message.to_string();
    if message.trim().starts_with('/') {
        match crate::slash::try_handle_slash(message, &rt).await {
            Some(crate::slash::SlashOutcome::Prompt(expanded)) => {
                prompt_text = expanded;
            }
            Some(crate::slash::SlashOutcome::Notice(text)) => {
                emit_slash_notice(state, user, session_id, &rt, out_tx, &text).await?;
                return Ok(());
            }
            Some(crate::slash::SlashOutcome::ResetUi {
                notice,
                seed,
                widget: _,
            }) => {
                let revision = state
                    .db
                    .clear_messages(&user.id, session_id)
                    .map_err(|e| e.to_string())?;
                let _ = out_tx.send(json!({
                    "type": "slash_result",
                    "sessionId": session_id,
                    "kind": "reset",
                    "text": notice,
                    "seed": seed
                        .iter()
                        .map(|s| json!({
                            "role": match s.role {
                                zi_tui::MessageRole::User => "user",
                                zi_tui::MessageRole::Assistant => "assistant",
                                zi_tui::MessageRole::System => "system",
                                zi_tui::MessageRole::Tool => "tool",
                                zi_tui::MessageRole::Thinking => "thinking",
                            },
                            "content": s.content,
                        }))
                        .collect::<Vec<_>>(),
                    "revision": revision,
                }));
                return Ok(());
            }
            Some(crate::slash::SlashOutcome::Quit) => {
                let _ = out_tx.send(json!({
                    "type": "slash_result",
                    "sessionId": session_id,
                    "kind": "notice",
                    "text": "`/quit` is only available in the TUI. Close the browser tab or log out instead.",
                }));
                return Ok(());
            }
            None => {}
        }
    }

    let mut rx = rt.agent.subscribe();
    let db = ArcDb(state.db.clone());
    let user_id = user.id.clone();
    let sid = session_id.to_string();
    let out = out_tx.clone();

    let forward = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(value) = serde_json::to_value(&event) {
                let _ = out.send(value);
            }
            if let AgentEvent::MessageEnd { message, .. } = &event {
                match db.0.append_message(&user_id, &sid, message) {
                    Ok(revision) => {
                        let _ = out.send(json!({
                            "type": "session_meta",
                            "sessionId": sid,
                            "revision": revision,
                        }));
                    }
                    Err(err) => {
                        tracing::warn!("web message persist failed: {err:#}");
                    }
                }
            }
            if matches!(event, AgentEvent::AgentEnd { .. }) {
                break;
            }
        }
    });

    let result = rt
        .agent
        .prompt(AgentPromptInput::Text(&prompt_text))
        .await
        .map_err(|e| e.to_string());
    rt.agent.wait_for_idle().await;
    drop(rt);
    join_or_abort_listener(forward).await;
    result
}

async fn emit_slash_notice(
    state: &AppState,
    user: &AuthUser,
    session_id: &str,
    rt: &crate::modes::CliRuntime,
    out_tx: &mpsc::UnboundedSender<Value>,
    text: &str,
) -> Result<(), String> {
    // `/model` may have switched the live agent — mirror into SQLite + UI.
    let model = rt.agent.model().await;
    if let Ok(Some(updated)) = state
        .db
        .set_session_model(&user.id, session_id, &model.id)
    {
        let _ = out_tx.send(json!({
            "type": "session_meta",
            "sessionId": session_id,
            "revision": updated.revision,
            "model": model.id,
            "title": updated.title,
        }));
    }
    let _ = out_tx.send(json!({
        "type": "slash_result",
        "sessionId": session_id,
        "kind": "notice",
        "text": text,
        "model": model.id,
    }));
    Ok(())
}

async fn run_abort(
    state: &AppState,
    user: &AuthUser,
    session_id: &str,
) -> Result<(), String> {
    state
        .db
        .get_session(&user.id, session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "session not found".to_string())?;

    let Some(handle) = state.runtimes.get(&user.id, session_id).await else {
        return Ok(());
    };
    let rt = handle.runtime.lock().await;
    rt.agent.abort();
    rt.agent.wait_for_idle().await;
    Ok(())
}

/// Thin newtype so the forward task can hold `Arc<WebDb>` without importing in spawn.
struct ArcDb(std::sync::Arc<super::db::WebDb>);
