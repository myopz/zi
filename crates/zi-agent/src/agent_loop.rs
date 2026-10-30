// Agent loop: prompt, stream assistant, execute tools (aligned with pi agent-loop.ts).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use zi_ai::{
    AssistantContentBlock, AssistantMessage, AssistantMessageEvent, Context, Message, StopReason,
    SystemContent, SystemMessage, SystemRole, Tool, ToolResultMessage, ToolResultRole,
};
use zi_ai::transcript::normalize_context;
use serde_json::Value;
use tracing::debug;

use crate::context_budget::{
    compact_token_limit, estimate_messages_tokens, fold_old_tool_results, should_compact,
};
use crate::stream_fn::StreamFn;
use crate::summarize_compact::{summarize_compact, SummarizeCompactOptions};
use crate::types::{
    AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AgentTool, AgentToolCallOutcome,
    AgentToolResult, ToolExecutionMode,
};

/// Sink for agent lifecycle events.
pub type AgentEventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

/// Run the agent loop from new user (or system) prompts until the model stops or the run aborts.
pub async fn run_agent_loop(
    prompts: Vec<AgentMessage>,
    context: &mut AgentContext,
    config: AgentLoopConfig,
    emit: AgentEventSink,
    abort: Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
) -> Result<Vec<AgentMessage>, crate::error::AgentError> {
    let mut new_messages = Vec::new();

    emit(AgentEvent::AgentStart);

    for message in prompts {
        if matches!(message, Message::User(_)) {
            context.explore_calls_since_user = 0;
        }
        emit(AgentEvent::MessageStart {
            message: message.clone(),
        });
        emit(AgentEvent::MessageEnd {
            message: message.clone(),
        });
        context.messages.push(message.clone());
        new_messages.push(message);
    }

    run_inner_loop(
        context,
        &mut new_messages,
        &config,
        &emit,
        &abort,
        stream_fn,
    )
    .await?;

    emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    });
    Ok(new_messages)
}

/// Max auto-continues after a text-only turn that still looks unfinished (local-model steer).
const MAX_TEXT_TOOL_CONTINUES: usize = 4;

async fn run_inner_loop(
    context: &mut AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
) -> Result<(), crate::error::AgentError> {
    let mut cumulative_input = 0u64;
    let mut cumulative_output = 0u64;
    let mut turn_count = 0usize;
    let mut text_continues_used = 0usize;

    loop {
        if abort.load(Ordering::Relaxed) {
            return Ok(());
        }

        if let Some(max) = config.max_turns {
            if turn_count >= max {
                tracing::info!(max, "agent loop hit max_turns; stopping");
                return Ok(());
            }
        }
        turn_count += 1;

        emit(AgentEvent::TurnStart);

        maybe_auto_compact(context, config, emit, stream_fn.clone(), &abort).await;

        let mut assistant = stream_assistant_response(
            context,
            config,
            emit,
            abort,
            stream_fn.clone(),
            cumulative_input,
            cumulative_output,
        )
        .await?;

        cumulative_input = cumulative_input.saturating_add(assistant.usage.input);
        cumulative_output = cumulative_output.saturating_add(assistant.usage.output);

        // Local models (esp. Ollama) often emit tool calls as plain JSON / <tool_call>
        // text, or narrate "请使用 `read` 工具" without structured tool_calls.
        if recover_text_emitted_tool_calls(&mut assistant, &known_tool_names(context)) {
            if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                *last = assistant.clone();
            }
            tracing::info!("recovered tool call(s) from assistant text content");
        }

        new_messages.push(Message::Assistant(assistant.clone()));

        if matches!(
            assistant.stop_reason,
            StopReason::Error | StopReason::Aborted
        ) {
            let tool_results = Vec::new();
            emit(AgentEvent::TurnEnd {
                message: Message::Assistant(assistant),
                tool_results: tool_results.clone(),
            });
            return Ok(());
        }

        let tool_calls = tool_calls_from_assistant(&assistant);
        if tool_calls.is_empty() {
            emit(AgentEvent::TurnEnd {
                message: Message::Assistant(assistant.clone()),
                tool_results: Vec::new(),
            });

            let text = assistant_text_content(&assistant);
            if text_continues_used < MAX_TEXT_TOOL_CONTINUES
                && looks_unfinished_agent_turn(&text)
            {
                text_continues_used += 1;
                let nudge = continue_nudge_message();
                emit(AgentEvent::MessageStart {
                    message: nudge.clone(),
                });
                emit(AgentEvent::MessageEnd {
                    message: nudge.clone(),
                });
                context.messages.push(nudge.clone());
                new_messages.push(nudge);
                tracing::info!(
                    text_continues_used,
                    "auto-continuing after unfinished text-only turn"
                );
                continue;
            }
            return Ok(());
        }

        text_continues_used = 0;
        promote_deferred_tools(context, config, &tool_calls);

        let tool_results = if assistant.stop_reason == StopReason::Length {
            fail_truncated_tool_calls(&tool_calls, emit).await
        } else {
            execute_tool_calls(context, &assistant, &tool_calls, config, emit, abort).await
        };

        for result in &tool_results {
            context.messages.push(Message::ToolResult(result.clone()));
            new_messages.push(Message::ToolResult(result.clone()));
        }

        emit(AgentEvent::TurnEnd {
            message: Message::Assistant(assistant),
            tool_results: tool_results.clone(),
        });

        if abort.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
}

async fn maybe_auto_compact(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    stream_fn: Arc<dyn StreamFn>,
    abort: &Arc<AtomicBool>,
) {
    if !config.auto_compact {
        return;
    }
    let estimated = estimate_messages_tokens(&context.messages);
    if !should_compact(
        estimated,
        config.model.context_window,
        config.compact_threshold_ratio,
        config.compact_absolute_token_limit,
    ) {
        return;
    }
    let archive_path = config.transcript_archive_dir.as_ref().map(|dir| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        dir.join(format!("compact-{ts}.txt"))
    });
    let Some(compacted) = summarize_compact(
        &context.messages,
        SummarizeCompactOptions {
            keep_recent_tokens: config.keep_recent_message_tokens,
            stream_fn,
            model: config.model.clone(),
            stream_options: config.stream_options.clone(),
            abort: Some(abort),
            archive_path,
        },
    )
    .await
    else {
        return;
    };
    let kept_messages: Vec<_> = compacted
        .messages
        .iter()
        .filter(|m| !matches!(m, Message::System(_)))
        .filter(|m| match m {
            Message::User(u) => match &u.content {
                zi_ai::UserContent::Text(t) => !t.contains("<compaction>"),
                _ => true,
            },
            _ => true,
        })
        .cloned()
        .collect();
    emit(AgentEvent::Compaction {
        summary: compacted.summary.clone(),
        tokens_before: compacted.tokens_before,
        tokens_after: compacted.tokens_after,
        dropped_messages: compacted.dropped_messages,
        kept_messages,
    });
    tracing::info!(
        tokens_before = compacted.tokens_before,
        tokens_after = compacted.tokens_after,
        dropped = compacted.dropped_messages,
        "auto-compacted agent context"
    );
    context.messages = compacted.messages;
}

async fn stream_assistant_response(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
    cumulative_input: u64,
    cumulative_output: u64,
) -> Result<AssistantMessage, crate::error::AgentError> {
    let llm_messages = default_convert_to_llm(&context.messages);
    let (folded_messages, folded_tool_results) =
        fold_old_tool_results(&llm_messages, config.keep_recent_tool_result_tokens);
    let estimated_tokens = estimate_messages_tokens(&folded_messages);
    let compact_limit = compact_token_limit(
        config.model.context_window,
        config.compact_threshold_ratio,
        config.compact_absolute_token_limit,
    );
    let near_limit = should_compact(
        estimated_tokens,
        config.model.context_window,
        config.compact_threshold_ratio,
        config.compact_absolute_token_limit,
    );
    emit(AgentEvent::ContextUsage {
        estimated_tokens,
        context_window: config.model.context_window,
        compact_limit,
        folded_tool_results,
        cumulative_input,
        cumulative_output,
        near_limit,
    });
    if near_limit {
        tracing::warn!(
            estimated_tokens,
            compact_limit,
            context_window = config.model.context_window,
            cumulative_input,
            cumulative_output,
            "context near compact limit; prefer /compact or fewer tool dumps"
        );
    } else {
        tracing::debug!(
            estimated_tokens,
            compact_limit,
            context_window = config.model.context_window,
            folded_tool_results,
            cumulative_input,
            cumulative_output,
            "context usage"
        );
    }

    // Advertise deferred tools in the schema so the model can call them by name
    // (promote_deferred_tools mounts on first use). Omitting them forces the model
    // to invent workarounds (codemode/bash) that then fail.
    let tools = agent_tools_for_llm(&context.tools, &context.deferred_tools);
    let transcript = normalize_context(Context {
        system_prompt: None,
        messages: folded_messages,
        tools,
    });

    let response = stream_fn
        .stream(
            &config.model,
            transcript,
            config.stream_options.clone(),
        )
        .await;

    let mut event_stream = response.events();
    let mut partial_started = false;

    while let Some(event) = event_stream.next().await {
        if abort.load(Ordering::Relaxed) {
            debug!("agent loop abort during assistant stream");
            break;
        }

        match &event {
            AssistantMessageEvent::Start { partial } => {
                context.messages.push(Message::Assistant(partial.clone()));
                partial_started = true;
                emit(AgentEvent::MessageStart {
                    message: Message::Assistant(partial.clone()),
                });
            }
            AssistantMessageEvent::TextStart { partial, .. }
            | AssistantMessageEvent::TextDelta { partial, .. }
            | AssistantMessageEvent::TextEnd { partial, .. }
            | AssistantMessageEvent::ThinkingStart { partial, .. }
            | AssistantMessageEvent::ThinkingDelta { partial, .. }
            | AssistantMessageEvent::ThinkingEnd { partial, .. }
            | AssistantMessageEvent::ToolcallStart { partial, .. }
            | AssistantMessageEvent::ToolcallDelta { partial, .. }
            | AssistantMessageEvent::ToolcallEnd { partial, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = partial.clone();
                    }
                    emit(AgentEvent::MessageUpdate {
                        message: Message::Assistant(partial.clone()),
                        assistant_message_event: event.clone(),
                    });
                }
            }
            AssistantMessageEvent::Done { message, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = message.clone();
                    }
                } else {
                    context.messages.push(Message::Assistant(message.clone()));
                    emit(AgentEvent::MessageStart {
                        message: Message::Assistant(message.clone()),
                    });
                }
                emit(AgentEvent::MessageEnd {
                    message: Message::Assistant(message.clone()),
                });
                return Ok(message.clone());
            }
            AssistantMessageEvent::Error { error, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = error.clone();
                    }
                } else {
                    context.messages.push(Message::Assistant(error.clone()));
                    emit(AgentEvent::MessageStart {
                        message: Message::Assistant(error.clone()),
                    });
                }
                emit(AgentEvent::MessageEnd {
                    message: Message::Assistant(error.clone()),
                });
                return Ok(error.clone());
            }
        }
    }

    let fallback = response.result().await;
    if !partial_started {
        context.messages.push(Message::Assistant(fallback.clone()));
        emit(AgentEvent::MessageStart {
            message: Message::Assistant(fallback.clone()),
        });
    } else if let Some(Message::Assistant(last)) = context.messages.last_mut() {
        *last = fallback.clone();
    }
    emit(AgentEvent::MessageEnd {
        message: Message::Assistant(fallback.clone()),
    });
    Ok(fallback)
}

fn agent_tools_for_llm(
    tools: &[Arc<dyn AgentTool>],
    deferred: &[Arc<dyn AgentTool>],
) -> Option<Vec<Tool>> {
    if tools.is_empty() && deferred.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(tools.len() + deferred.len());
    let mut seen = std::collections::HashSet::new();
    for tool in tools.iter().chain(deferred.iter()) {
        let name = tool.name().to_string();
        if !seen.insert(name.clone()) {
            continue;
        }
        out.push(Tool {
            name,
            description: tool.description().to_string(),
            parameters: tool.parameters_schema(),
        });
    }
    Some(out)
}

fn default_convert_to_llm(messages: &[AgentMessage]) -> Vec<AgentMessage> {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                Message::System(_) | Message::User(_) | Message::Assistant(_) | Message::ToolResult(_)
            )
        })
        .cloned()
        .collect()
}

#[derive(Clone)]
struct ParsedToolCall {
    id: String,
    name: String,
    arguments: Value,
}

fn tool_calls_from_assistant(message: &AssistantMessage) -> Vec<ParsedToolCall> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(ParsedToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn known_tool_names(context: &AgentContext) -> std::collections::HashSet<String> {
    context
        .tools
        .iter()
        .chain(context.deferred_tools.iter())
        .map(|tool| tool.name().to_string())
        .collect()
}

fn assistant_text_content(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn continue_nudge_message() -> Message {
    Message::System(SystemMessage {
        role: SystemRole::System,
        content: SystemContent::Text(
            "Continue the unfinished task. Call tools now — do not narrate tool use \
             or ask the user to run tools. For broad architecture / 'how does this work' \
             questions call explore first, then read Key paths."
                .into(),
        ),
        sections: None,
        tools_added: None,
        tools_removed: None,
        timestamp: wall_now_ms(),
    })
}

fn looks_unfinished_agent_turn(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if trimmed.contains("请使用") && trimmed.contains("工具") {
        return true;
    }
    if trimmed.contains("使用 `") && trimmed.contains("工具") {
        return true;
    }
    if lower.contains("use the `") && lower.contains("tool") {
        return true;
    }
    if lower.contains("use `") && lower.contains(" tool") {
        return true;
    }
    const PLAN_MARKERS: &[&str] = &[
        "接下来",
        "我们先",
        "让我们先",
        "让我们从",
        "下一步",
        "若有必要",
        "i will now",
        "i'll now",
        "next i will",
        "let's start",
        "let us start",
        "first, i need",
        "first i need",
    ];
    PLAN_MARKERS
        .iter()
        .any(|m| trimmed.contains(m) || lower.contains(m))
}

/// Promote text-shaped tool calls into structured [`AssistantContentBlock::ToolCall`]s.
///
/// Returns true when at least one call was recovered.
fn recover_text_emitted_tool_calls(
    message: &mut AssistantMessage,
    known: &std::collections::HashSet<String>,
) -> bool {
    if known.is_empty() {
        return false;
    }
    if message
        .content
        .iter()
        .any(|b| matches!(b, AssistantContentBlock::ToolCall { .. }))
    {
        return false;
    }

    let text = assistant_text_content(message);
    if text.trim().is_empty() {
        return false;
    }

    let (mut calls, cleaned) = extract_text_tool_calls(&text, known);
    if calls.is_empty() {
        calls = extract_narrative_tool_calls(&text, known);
    }
    if calls.is_empty() {
        return false;
    }

    apply_recovered_tool_calls(message, &cleaned, calls);
    true
}

fn apply_recovered_tool_calls(
    message: &mut AssistantMessage,
    cleaned_text: &str,
    calls: Vec<(String, Value)>,
) {
    let mut content = Vec::new();
    let cleaned = cleaned_text.trim();
    if !cleaned.is_empty() {
        content.push(AssistantContentBlock::Text {
            text: cleaned.to_string(),
            text_signature: None,
        });
    }
    for (idx, (name, arguments)) in calls.into_iter().enumerate() {
        content.push(AssistantContentBlock::ToolCall {
            id: format!("text-call-{}-{idx}", wall_now_ms()),
            name,
            arguments,
            thought_signature: None,
            namespace: None,
            arguments_error: None,
        });
    }
    message.content = content;
    if matches!(
        message.stop_reason,
        StopReason::Stop | StopReason::Pending
    ) {
        message.stop_reason = StopReason::ToolUse;
        message.raw_stop_reason = Some("tool_calls".into());
    }
}

/// Recover "请使用 `read` 工具读取 `Cargo.toml`" style narration into tool calls.
fn extract_narrative_tool_calls(
    text: &str,
    known: &std::collections::HashSet<String>,
) -> Vec<(String, Value)> {
    let ticks = find_backtick_spans(text);
    let mut calls = Vec::new();
    for (i, (start, end, inner)) in ticks.iter().enumerate() {
        if !known.contains(inner) {
            continue;
        }
        let before_start = floor_char_boundary(text, start.saturating_sub(24));
        let before = &text[before_start..*start];
        let after_end = ceil_char_boundary(text, (*end + 16).min(text.len()));
        let after = &text[*end..after_end];
        let after_trim = after.trim_start();
        let imperative = before.contains("请使用")
            || before.contains("调用")
            || before.contains("使用")
            || before.to_lowercase().contains("use the")
            || before.to_lowercase().ends_with("use ")
            || after_trim.starts_with("工具")
            || after_trim.to_lowercase().starts_with("tool");
        if !imperative {
            continue;
        }

        // Do not treat `.` inside paths like `Cargo.toml` as a sentence end.
        let sentence_end = narrative_sentence_end(text, *end);
        let window = &text[*end..sentence_end];
        let path = find_backtick_spans(window)
            .into_iter()
            .map(|(_, _, s)| s)
            .find(|s| s != inner && known.get(s).is_none());

        let args = narrative_tool_args(inner, path.as_deref());
        if calls
            .last()
            .is_some_and(|(n, a)| n == inner && a == &args)
        {
            continue;
        }
        // Skip if this tool span is only listed as documentation of available tools.
        let _ = i;
        calls.push((inner.clone(), args));
    }
    calls
}

fn narrative_tool_args(tool: &str, path: Option<&str>) -> Value {
    match tool {
        "read" | "write" | "edit" => {
            let path = path.unwrap_or(".");
            serde_json::json!({ "path": path })
        }
        "ls" => match path {
            Some(p) => serde_json::json!({ "path": p.trim_end_matches('/') }),
            None => serde_json::json!({}),
        },
        "find" => match path {
            Some(p) => serde_json::json!({ "pattern": "**/*", "path": p.trim_end_matches('/') }),
            None => serde_json::json!({ "pattern": "**/*" }),
        },
        "explore" => serde_json::json!({
            "goal": path.unwrap_or("Explore the codebase architecture and key entry points")
        }),
        "grep" => serde_json::json!({
            "pattern": path.unwrap_or("."),
        }),
        _ => serde_json::json!({}),
    }
}

fn narrative_sentence_end(text: &str, from: usize) -> usize {
    let rest = &text[from..];
    let mut best = text.len();
    for delim in ["。", "；", "\n", ". ", ".\n", "; "] {
        if let Some(rel) = rest.find(delim) {
            best = best.min(from + rel);
        }
    }
    floor_char_boundary(text, best)
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn find_backtick_spans(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                i += 1;
            }
            if i < bytes.len() {
                let end = i + 1;
                let inner = text[start + 1..i].trim().to_string();
                if !inner.is_empty() && !inner.contains('\n') {
                    out.push((start, end, inner));
                }
                i = end;
                continue;
            }
            break;
        }
        i += 1;
    }
    out
}

fn extract_text_tool_calls(
    text: &str,
    known: &std::collections::HashSet<String>,
) -> (Vec<(String, Value)>, String) {
    let mut spans: Vec<(usize, usize, String, Value)> = Vec::new();

    for (start, end, raw) in find_tool_call_tags(text) {
        if let Some((name, args)) = parse_tool_call_json(&raw, known) {
            spans.push((start, end, name, args));
        }
    }
    for (start, end, raw) in find_markdown_json_fences(text) {
        if let Some((name, args)) = parse_tool_call_json(&raw, known) {
            spans.push((start, end, name, args));
        }
    }
    if spans.is_empty() {
        for (start, end, value) in find_json_objects(text) {
            if let Some((name, args)) = parse_tool_call_value(&value, known) {
                spans.push((start, end, name, args));
            }
        }
    }

    spans.sort_by_key(|(start, _, _, _)| *start);
    let mut calls = Vec::new();
    let mut cleaned = String::new();
    let mut cursor = 0usize;
    let mut last_end = 0usize;
    for (start, end, name, args) in spans {
        if start < last_end {
            continue; // overlap
        }
        cleaned.push_str(&text[cursor..start]);
        cursor = end;
        last_end = end;
        calls.push((name, args));
    }
    cleaned.push_str(&text[cursor..]);
    // Collapse leftover blank lines from removed fences/tags.
    let cleaned = cleaned
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (calls, cleaned)
}

fn parse_tool_call_json(
    raw: &str,
    known: &std::collections::HashSet<String>,
) -> Option<(String, Value)> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    parse_tool_call_value(&value, known)
}

fn parse_tool_call_value(
    value: &Value,
    known: &std::collections::HashSet<String>,
) -> Option<(String, Value)> {
    let obj = value.as_object()?;
    let name = obj.get("name")?.as_str()?;
    if !known.contains(name) {
        return None;
    }
    let arguments = obj.get("arguments")?;
    let arguments = match arguments {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| Value::Object(Default::default())),
        other => other.clone(),
    };
    Some((name.to_string(), arguments))
}

fn find_tool_call_tags(text: &str) -> Vec<(usize, usize, String)> {
    let open = "<tool_call>";
    let close = "</tool_call>";
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(open) {
        let start = search_from + rel;
        let inner_start = start + open.len();
        let Some(rel_end) = text[inner_start..].find(close) else {
            break;
        };
        let inner_end = inner_start + rel_end;
        let end = inner_end + close.len();
        out.push((start, end, text[inner_start..inner_end].trim().to_string()));
        search_from = end;
    }
    out
}

fn find_markdown_json_fences(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find("```") {
        let start = search_from + rel;
        let after_ticks = start + 3;
        let rest = &text[after_ticks..];
        let (header_end, body_start) = match rest.find('\n') {
            Some(n) => {
                let header = rest[..n].trim();
                // Accept ```json / ```JSON / bare ```
                if !header.is_empty() && !header.eq_ignore_ascii_case("json") {
                    search_from = after_ticks;
                    continue;
                }
                (after_ticks + n, after_ticks + n + 1)
            }
            None => break,
        };
        let _ = header_end;
        let Some(rel_close) = text[body_start..].find("```") else {
            break;
        };
        let body_end = body_start + rel_close;
        let end = body_end + 3;
        out.push((start, end, text[body_start..body_end].trim().to_string()));
        search_from = end;
    }
    out
}

fn find_json_objects(text: &str) -> Vec<(usize, usize, Value)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some((end, value)) = try_parse_json_object(text, i) {
                out.push((i, end, value));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn try_parse_json_object(text: &str, start: usize) -> Option<(usize, Value)> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    let slice = &text[start..end];
                    let value = serde_json::from_str(slice).ok()?;
                    return Some((end, value));
                }
            }
            _ => {}
        }
    }
    None
}

async fn fail_truncated_tool_calls(
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
) -> Vec<ToolResultMessage> {
    let mut messages = Vec::new();
    for call in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.arguments.clone(),
        });
        let result = AgentToolResult::error(format!(
            "Tool call \"{}\" was not executed: the response hit the output token limit.",
            call.name
        ));
        emit(AgentEvent::ToolExecutionEnd {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            result: result.clone(),
            is_error: true,
            duration_ms: None,
        });
        let tool_result = tool_result_message(&call.id, &call.name, &result, true, 0);
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

async fn execute_tool_calls(
    context: &mut AgentContext,
    assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let sequential = config.tool_execution == ToolExecutionMode::Sequential
        || tool_calls.iter().any(|call| {
            find_tool(context, &call.name)
                .and_then(|tool| tool.execution_mode())
                .map(|mode| mode == ToolExecutionMode::Sequential)
                .unwrap_or(false)
        })
        // Budget accounting needs ordered execution for tree-walk tools.
        || tool_calls.iter().any(|c| is_budgeted_search_tool(&c.name));

    if sequential {
        execute_tool_calls_sequential(context, assistant, tool_calls, config, emit, abort).await
    } else {
        execute_tool_calls_parallel(context, assistant, tool_calls, emit, abort).await
    }
}

/// Tools that burn the parent exploration budget (tree walking / content scan).
/// `read` is excluded so post-explore Key-path verification is not hard-capped.
fn is_budgeted_search_tool(name: &str) -> bool {
    matches!(name, "ls" | "find" | "grep")
}

fn promote_deferred_tools(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    tool_calls: &[ParsedToolCall],
) {
    if config.deferred_tool_names.is_empty() || context.deferred_tools.is_empty() {
        return;
    }
    let wanted: Vec<String> = tool_calls
        .iter()
        .filter(|c| config.deferred_tool_names.iter().any(|d| d == &c.name))
        .map(|c| c.name.clone())
        .collect();
    if wanted.is_empty() {
        return;
    }
    let mut remaining = Vec::new();
    for tool in std::mem::take(&mut context.deferred_tools) {
        if wanted.iter().any(|n| n == tool.name()) {
            if !context.tools.iter().any(|t| t.name() == tool.name()) {
                tracing::info!(tool = tool.name(), "mounting deferred tool");
                context.tools.push(tool);
            }
        } else {
            remaining.push(tool);
        }
    }
    context.deferred_tools = remaining;
}

async fn execute_tool_calls_sequential(
    context: &mut AgentContext,
    _assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let mut messages = Vec::new();
    for call in tool_calls {
        if abort.load(Ordering::Relaxed) {
            break;
        }
        if let Some(blocked) = explore_budget_block(context, config, call) {
            emit(AgentEvent::ToolExecutionStart {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
            });
            let result = AgentToolResult::error(blocked);
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                result: result.clone(),
                is_error: true,
                duration_ms: Some(0),
            });
            let tool_result = outcome_to_tool_result(&AgentToolCallOutcome {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                result,
                is_error: true,
                duration_ms: 0,
            });
            emit_tool_result_message(&tool_result, emit);
            messages.push(tool_result);
            continue;
        }
        let outcome = run_one_tool_call(context, call, emit, abort).await;
        let mut tool_result = outcome_to_tool_result(&outcome);
        if call.name == "explore" && !tool_result.is_error {
            // Budget blocks replacing explore with many ls/find/grep; after a successful
            // explore, allow the parent to verify Key paths (reads are not budgeted).
            context.explore_calls_since_user = 0;
        } else if is_budgeted_search_tool(&call.name) {
            context.explore_calls_since_user =
                context.explore_calls_since_user.saturating_add(1);
            if config.explore_budget_warn > 0
                && context.explore_calls_since_user >= config.explore_budget_warn
                && !tool_result.is_error
            {
                append_explore_budget_hint(&mut tool_result, context.explore_calls_since_user);
            }
        }
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

fn explore_budget_block(
    context: &AgentContext,
    config: &AgentLoopConfig,
    call: &ParsedToolCall,
) -> Option<String> {
    if config.explore_budget_block == 0 || !is_budgeted_search_tool(&call.name) {
        return None;
    }
    if context.explore_calls_since_user >= config.explore_budget_block {
        return Some(format!(
            "Exploration budget exceeded ({} ls/find/grep calls since last user message). \
             Use explore for broad search, or read known paths / stop and answer. \
             (read is not budgeted.)",
            context.explore_calls_since_user
        ));
    }
    None
}

fn append_explore_budget_hint(tr: &mut ToolResultMessage, count: usize) {
    let hint = format!(
        "\n\n[explore budget] {count} ls/find/grep calls since last user message. \
         Prefer explore for broad questions, or read known paths / stop and answer."
    );
    if let Some(zi_ai::ToolResultContentBlock::Text(t)) = tr.content.first_mut() {
        t.text.push_str(&hint);
    } else {
        tr.content.push(zi_ai::ToolResultContentBlock::Text(
            zi_ai::TextContent {
                kind: zi_ai::TextContentKind::Text,
                text: hint,
                text_signature: None,
            },
        ));
    }
}

async fn execute_tool_calls_parallel(
    context: &AgentContext,
    _assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    join_prepared_tool_calls(context, tool_calls, emit, abort).await
}

async fn join_prepared_tool_calls(
    context: &AgentContext,
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let mut join_set = tokio::task::JoinSet::new();
    let mut immediate = Vec::new();

    for (index, call) in tool_calls.iter().enumerate() {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.arguments.clone(),
        });

        if abort.load(Ordering::Relaxed) {
            let result = AgentToolResult::error("Operation aborted");
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                result: result.clone(),
                is_error: true,
                duration_ms: None,
            });
            immediate.push(outcome_to_tool_result(&AgentToolCallOutcome {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                result,
                is_error: true,
                duration_ms: 0,
            }));
            continue;
        }

        let Some(tool) = find_tool(context, &call.name) else {
            let result = AgentToolResult::error(format!("Tool {} not found", call.name));
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                result: result.clone(),
                is_error: true,
                duration_ms: None,
            });
            immediate.push(outcome_to_tool_result(&AgentToolCallOutcome {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                result,
                is_error: true,
                duration_ms: 0,
            }));
            continue;
        };

        let call_id = call.id.clone();
        let call_name = call.name.clone();
        let args = call.arguments.clone();
        let emit = emit.clone();
        let abort = abort.clone();
        join_set.spawn(async move {
            let started = Instant::now();
            let emit_update = emit.clone();
            let call_id_u = call_id.clone();
            let call_name_u = call_name.clone();
            let args_u = args.clone();
            let on_update: Option<crate::types::AgentToolUpdateCallback> =
                Some(Box::new(move |partial| {
                    emit_update(AgentEvent::ToolExecutionUpdate {
                        tool_call_id: call_id_u.clone(),
                        tool_name: call_name_u.clone(),
                        args: args_u.clone(),
                        partial_result: partial,
                    });
                }));

            let result = if abort.load(Ordering::Relaxed) {
                AgentToolResult::error("Operation aborted")
            } else {
                tool.execute(&call_id, args.clone(), on_update).await
            };
            let is_error = result.is_error.unwrap_or(false);
            let duration_ms = started.elapsed().as_millis() as u64;
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call_id.clone(),
                tool_name: call_name.clone(),
                result: result.clone(),
                is_error,
                duration_ms: Some(duration_ms),
            });
            (
                index,
                AgentToolCallOutcome {
                    tool_call_id: call_id,
                    tool_name: call_name,
                    args,
                    result,
                    is_error,
                    duration_ms,
                },
            )
        });
    }

    let mut spawned = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        if let Ok(outcome) = joined {
            spawned.push(outcome);
        }
    }
    spawned.sort_by_key(|(index, _)| *index);

    let mut messages = immediate;
    for (_, outcome) in spawned {
        let tool_result = outcome_to_tool_result(&outcome);
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

async fn run_one_tool_call(
    context: &AgentContext,
    call: &ParsedToolCall,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> AgentToolCallOutcome {
    emit(AgentEvent::ToolExecutionStart {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.arguments.clone(),
    });

    let started = Instant::now();
    let call_id = call.id.clone();
    let call_name = call.name.clone();
    let args = call.arguments.clone();

    let on_update: Option<crate::types::AgentToolUpdateCallback> = Some(Box::new({
        let emit = emit.clone();
        let call_id = call_id.clone();
        let call_name = call_name.clone();
        let args = args.clone();
        move |partial| {
            emit(AgentEvent::ToolExecutionUpdate {
                tool_call_id: call_id.clone(),
                tool_name: call_name.clone(),
                args: args.clone(),
                partial_result: partial,
            });
        }
    }));

    let (result, is_error) = if abort.load(Ordering::Relaxed) {
        (AgentToolResult::error("Operation aborted"), true)
    } else if let Some(tool) = find_tool(context, &call.name) {
        let result = tool
            .execute(&call.id, call.arguments.clone(), on_update)
            .await;
        let is_error = result.is_error.unwrap_or(false);
        (result, is_error)
    } else {
        (
            AgentToolResult::error(format!("Tool {} not found", call.name)),
            true,
        )
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        result: result.clone(),
        is_error,
        duration_ms: Some(duration_ms),
    });

    AgentToolCallOutcome {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.arguments.clone(),
        result,
        is_error,
        duration_ms,
    }
}

fn find_tool(context: &AgentContext, name: &str) -> Option<Arc<dyn AgentTool>> {
    context
        .tools
        .iter()
        .find(|tool| tool.name() == name)
        .cloned()
}

fn outcome_to_tool_result(outcome: &AgentToolCallOutcome) -> ToolResultMessage {
    tool_result_message(
        &outcome.tool_call_id,
        &outcome.tool_name,
        &outcome.result,
        outcome.is_error,
        outcome.duration_ms,
    )
}

fn tool_result_message(
    tool_call_id: &str,
    tool_name: &str,
    result: &AgentToolResult,
    is_error: bool,
    _duration_ms: u64,
) -> ToolResultMessage {
    ToolResultMessage {
        role: ToolResultRole::ToolResult,
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        content: result
            .content
            .iter()
            .cloned()
            .map(zi_ai::ToolResultContentBlock::Text)
            .collect(),
        details: Some(result.details.clone()),
        is_error,
        timestamp: wall_now_ms(),
    }
}

fn emit_tool_result_message(message: &ToolResultMessage, emit: &AgentEventSink) {
    emit(AgentEvent::MessageStart {
        message: Message::ToolResult(message.clone()),
    });
    emit(AgentEvent::MessageEnd {
        message: Message::ToolResult(message.clone()),
    });
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod text_tool_call_recovery_tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn known(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn recovers_bare_json() {
        let (calls, cleaned) = extract_text_tool_calls(
            r#"{"name": "explore", "arguments": {"goal": "architecture"}}"#,
            &known(&["explore"]),
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "explore");
        assert_eq!(calls[0].1, json!({"goal": "architecture"}));
        assert!(cleaned.is_empty());
    }

    #[test]
    fn recovers_markdown_fence_after_prose() {
        let text = r#"让我们先开始使用 explore 工具。

```json
{
  "name": "explore",
  "arguments": {
    "goal": "Learn about the codebase"
  }
}
```"#;
        let (calls, cleaned) = extract_text_tool_calls(text, &known(&["explore"]));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "explore");
        assert!(cleaned.contains("让我们先开始"));
        assert!(!cleaned.contains("```"));
    }

    #[test]
    fn recovers_tool_call_xml_tags() {
        let text = r#"<tool_call>
{"name": "ls", "arguments": {"path": "."}}
</tool_call>"#;
        let (calls, _) = extract_text_tool_calls(text, &known(&["ls"]));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "ls");
    }

    #[test]
    fn ignores_unknown_tool_names() {
        let (calls, cleaned) = extract_text_tool_calls(
            r#"{"name": "not_a_tool", "arguments": {}}"#,
            &known(&["explore"]),
        );
        assert!(calls.is_empty());
        assert!(cleaned.contains("not_a_tool"));
    }

    #[test]
    fn promote_mutates_assistant_message() {
        let mut msg = AssistantMessage {
            role: zi_ai::AssistantRole::Assistant,
            content: vec![AssistantContentBlock::Text {
                text: r#"{"name":"explore","arguments":{"goal":"x"}}"#.into(),
                text_signature: None,
            }],
            api: "openai-completions".into(),
            provider: "ollama".into(),
            model: "qwen2.5-coder:7b".into(),
            response_model: None,
            response_id: None,
            usage: Default::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            raw_stop_reason: Some("stop".into()),
            timestamp: 1,
            duration_ms: None,
        };
        assert!(recover_text_emitted_tool_calls(&mut msg, &known(&["explore"])));
        assert_eq!(msg.stop_reason, StopReason::ToolUse);
        assert!(msg
            .content
            .iter()
            .any(|b| matches!(b, AssistantContentBlock::ToolCall { name, .. } if name == "explore")));
    }

    #[test]
    fn recovers_chinese_please_use_tool_narration() {
        let text = r#"项目目录结构如下：
我们先从关键文件开始分析。

### 1. `Cargo.toml`
- 请使用 `read` 工具读取 `Cargo.toml` 文件内容。

### 2. `README.md`
- 请使用 `read` 工具读取 `README.md` 文件内容，以便了解项目的用途和主要功能。

### 3. `zi-web/`
- 请使用 `ls` 工具列出 `zi-web/` 目录的内容，以便了解具体结构。

### 4. `crates/`
- 请使用 `ls` 工具列出 `crates/` 目录的内容，以便了解具体结构。

接下来，我们可以从这些文件和目录中获取更多信息。"#;
        let calls = extract_narrative_tool_calls(text, &known(&["read", "ls", "explore"]));
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].0, "read");
        assert_eq!(calls[0].1, json!({"path": "Cargo.toml"}));
        assert_eq!(calls[1].0, "read");
        assert_eq!(calls[1].1, json!({"path": "README.md"}));
        assert_eq!(calls[2].0, "ls");
        assert_eq!(calls[2].1, json!({"path": "zi-web"}));
        assert_eq!(calls[3].0, "ls");
        assert_eq!(calls[3].1, json!({"path": "crates"}));
        assert!(looks_unfinished_agent_turn(text));
    }

    #[test]
    fn recover_prefers_json_then_falls_back_to_narrative() {
        let mut msg = AssistantMessage {
            role: zi_ai::AssistantRole::Assistant,
            content: vec![AssistantContentBlock::Text {
                text: "请使用 `read` 工具读取 `Cargo.toml`。".into(),
                text_signature: None,
            }],
            api: "openai-completions".into(),
            provider: "ollama".into(),
            model: "qwen2.5-coder:7b".into(),
            response_model: None,
            response_id: None,
            usage: Default::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            raw_stop_reason: Some("stop".into()),
            timestamp: 1,
            duration_ms: None,
        };
        assert!(recover_text_emitted_tool_calls(
            &mut msg,
            &known(&["read", "ls"])
        ));
        let tools: Vec<_> = msg
            .content
            .iter()
            .filter_map(|b| match b {
                AssistantContentBlock::ToolCall { name, arguments, .. } => {
                    Some((name.clone(), arguments.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(tools, vec![("read".into(), json!({"path": "Cargo.toml"}))]);
    }
}

#[cfg(test)]
mod explore_budget_tests {
    use super::*;

    fn call(name: &str) -> ParsedToolCall {
        ParsedToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: Value::Object(Default::default()),
        }
    }

    fn ctx(count: usize) -> AgentContext {
        AgentContext {
            messages: Vec::new(),
            tools: Vec::new(),
            deferred_tools: Vec::new(),
            explore_calls_since_user: count,
        }
    }

    fn tiny_model() -> zi_ai::Model {
        zi_ai::Model {
            id: "t".into(),
            name: "t".into(),
            api: "openai-completions".into(),
            provider: "t".into(),
            base_url: String::new(),
            reasoning: false,
            context_window: 8_000,
            max_tokens: 1_024,
            cost: zi_ai::ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        }
    }

    #[test]
    fn budget_zero_disables_block() {
        let mut config = AgentLoopConfig::new(tiny_model());
        config.explore_budget_block = 0;
        assert!(explore_budget_block(&ctx(99), &config, &call("read")).is_none());
    }

    #[test]
    fn budget_blocks_when_exhausted() {
        let mut config = AgentLoopConfig::new(tiny_model());
        config.explore_budget_block = 12;
        assert!(explore_budget_block(&ctx(12), &config, &call("ls")).is_some());
        assert!(explore_budget_block(&ctx(11), &config, &call("grep")).is_none());
        assert!(explore_budget_block(&ctx(12), &config, &call("read")).is_none());
        assert!(explore_budget_block(&ctx(12), &config, &call("explore")).is_none());
    }
}
