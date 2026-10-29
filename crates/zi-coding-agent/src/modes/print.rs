//! Text print mode: assistant output and tool activity on stdout.

use std::io::Write;

use anyhow::Result;
use zi_agent::AgentEvent;
use zi_ai::{AssistantContentBlock, Message, StopReason, TextContentKind};

use super::{prepare_runtime, run_prompt, CliRuntime};
use crate::cli::Cli;

/// Single-shot run; prints streaming-friendly text to stdout.
pub async fn run_print_mode(cli: &Cli) -> Result<i32> {
    let prompt = cli
        .prompt_text()
        .ok_or_else(|| anyhow::anyhow!("missing prompt: use -p/--print or positional text"))?;
    let (runtime, _) = prepare_runtime(cli).await?;

    let mut rx = runtime.agent.subscribe();
    let print_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            print_text_event(&event);
        }
    });

    let code = run_prompt(&runtime, &prompt).await?;
    print_task.abort();
    let _ = print_task.await;

    print_final_assistant(&runtime).await;
    Ok(code)
}

fn print_text_event(event: &AgentEvent) {
    match event {
        AgentEvent::ToolExecutionStart {
            tool_name,
            args,
            ..
        } => {
            let args_preview = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
            let _ = writeln!(std::io::stdout(), "[tool:start] {tool_name} {args_preview}");
        }
        AgentEvent::ToolExecutionEnd {
            tool_name,
            is_error,
            result,
            duration_ms,
            ..
        } => {
            let preview = result
                .content
                .iter()
                .filter_map(|block| match block {
                    zi_ai::TextContent {
                        kind: TextContentKind::Text,
                        text,
                        ..
                    } => Some(text.as_str()),
                })
                .collect::<Vec<_>>()
                .join("");
            let status = if *is_error { "error" } else { "ok" };
            let ms = duration_ms.map(|d| format!(" {d}ms")).unwrap_or_default();
            let _ = writeln!(
                std::io::stdout(),
                "[tool:{status}] {tool_name}{ms}\n{preview}"
            );
        }
        _ => {}
    }
}

async fn print_final_assistant(runtime: &CliRuntime) {
    let messages = runtime.agent.messages().await;
    let Some(Message::Assistant(assistant)) = messages.last() else {
        return;
    };
    if matches!(
        assistant.stop_reason,
        StopReason::Error | StopReason::Aborted
    ) {
        if let Some(err) = &assistant.error_message {
            let _ = writeln!(std::io::stderr(), "{err}");
        }
        return;
    }
    for block in &assistant.content {
        if let AssistantContentBlock::Text { text, .. } = block {
            if !text.is_empty() {
                let _ = writeln!(std::io::stdout(), "{text}");
            }
        }
    }
}
