//! JSON mode: JSONL agent events on stdout.

use std::io::Write;

use anyhow::Result;
use zi_agent::AgentEvent;
use serde::Serialize;

use super::{prepare_runtime, run_prompt};
use crate::cli::Cli;

#[derive(Serialize)]
struct JsonWireEvent<'a> {
    #[serde(flatten)]
    event: &'a AgentEvent,
}

/// Single-shot run emitting one JSON object per line.
pub async fn run_json_mode(cli: &Cli) -> Result<i32> {
    let prompt = cli
        .prompt_text()
        .ok_or_else(|| anyhow::anyhow!("missing prompt: use -p/--print or positional text"))?;
    let (runtime, _) = prepare_runtime(cli).await?;

    if let Some(header) = runtime.session.lock().await.header() {
        emit_json_value(header)?;
    }

    let mut rx = runtime.agent.subscribe();
    let json_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Err(err) = emit_json_event(&event) {
                tracing::warn!("json emit failed: {err:#}");
            }
        }
    });

    let code = run_prompt(&runtime, &prompt).await?;
    json_task.abort();
    let _ = json_task.await;
    Ok(code)
}

fn emit_json_event(event: &AgentEvent) -> Result<()> {
    let wire = JsonWireEvent { event };
    emit_json_value(&wire)
}

fn emit_json_value(value: impl Serialize) -> Result<()> {
    let line = serde_json::to_string(&value)?;
    writeln!(std::io::stdout(), "{line}")?;
    Ok(())
}
