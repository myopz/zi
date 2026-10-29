//! RPC mode: minimal JSONL command protocol on stdin/stdout.

use std::io::{self, BufRead, Write};

use anyhow::Result;
use zi_agent::{AgentEvent, AgentPromptInput};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{handle_event, prepare_runtime, CliRuntime};
use crate::cli::Cli;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RpcCommand {
    Prompt {
        id: Option<String>,
        message: String,
    },
    Abort {
        id: Option<String>,
    },
}

#[derive(Serialize)]
struct RpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    kind: &'static str,
    command: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Headless JSONL RPC loop (`prompt` and `abort` commands).
pub async fn run_rpc_mode(cli: &Cli) -> Result<i32> {
    let (runtime, _) = prepare_runtime(cli).await?;

    if let Some(header) = runtime.session.lock().await.header() {
        emit_line(header)?;
    }

    let runtime = Mutex::new(runtime);
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    while let Some(line) = lines.next() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let command: RpcCommand = match serde_json::from_str(trimmed) {
            Ok(c) => c,
            Err(err) => {
                emit_line(&RpcResponse {
                    id: None,
                    kind: "response",
                    command: "unknown".into(),
                    success: false,
                    error: Some(format!("invalid command JSON: {err}")),
                })?;
                continue;
            }
        };

        match command {
            RpcCommand::Prompt { id, message } => {
                let rt = runtime.lock().await;
                let result = run_rpc_prompt(&rt, &message).await;
                drop(rt);
                match result {
                    Ok(()) => {
                        emit_line(&RpcResponse {
                            id,
                            kind: "response",
                            command: "prompt".into(),
                            success: true,
                            error: None,
                        })?;
                    }
                    Err(err) => {
                        emit_line(&RpcResponse {
                            id,
                            kind: "response",
                            command: "prompt".into(),
                            success: false,
                            error: Some(err.to_string()),
                        })?;
                    }
                }
            }
            RpcCommand::Abort { id } => {
                let rt = runtime.lock().await;
                rt.agent.abort();
                rt.agent.wait_for_idle().await;
                emit_line(&RpcResponse {
                    id,
                    kind: "response",
                    command: "abort".into(),
                    success: true,
                    error: None,
                })?;
            }
        }
    }

    Ok(0)
}

async fn run_rpc_prompt(runtime: &CliRuntime, message: &str) -> Result<()> {
    let mut rx = runtime.agent.subscribe();
    let session = runtime.session.clone();

    let forward = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Err(err) = handle_event(&session, &event).await {
                tracing::warn!("session persist failed: {err:#}");
            }
            if let Err(err) = emit_rpc_event(&event) {
                tracing::warn!("rpc event emit failed: {err:#}");
            }
            if matches!(event, AgentEvent::AgentEnd { .. }) {
                break;
            }
        }
    });

    runtime
        .agent
        .prompt(AgentPromptInput::Text(message))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    runtime.agent.wait_for_idle().await;
    super::join_or_abort_listener(forward).await;
    Ok(())
}

fn emit_rpc_event(event: &AgentEvent) -> Result<()> {
    emit_line(event)
}

fn emit_line(value: impl Serialize) -> Result<()> {
    let line = serde_json::to_string(&value)?;
    writeln!(io::stdout(), "{line}")?;
    io::stdout().flush()?;
    Ok(())
}
