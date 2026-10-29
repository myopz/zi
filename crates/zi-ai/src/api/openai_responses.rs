// OpenAI Responses API streaming (aligned with pi openai-responses.ts).

use std::collections::HashMap;

use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::debug;

use crate::auth::require_api_key;
use crate::cost::calculate_cost;
use crate::event_stream::AssistantMessageEventStream;
use crate::sse::{decode_sse, parse_json_data};
use crate::streaming_json::ToolCallArgumentsStream;
use crate::transcript::{declared_tools, openai_responses_input, openai_tools};
use crate::types::{
    AssistantContentBlock, AssistantMessage, DoneStopReason, ErrorStopReason, Model, SimpleStreamOptions,
    StopReason, StreamOptions, Tool, TranscriptContext,
};

struct OutputSlot {
    content_index: usize,
    kind: SlotKind,
}

enum SlotKind {
    Text,
    Thinking,
    ToolCall { arguments: ToolCallArgumentsStream },
}

/// Stream an assistant message via OpenAI Responses SSE.
pub fn stream(
    model: Model,
    context: TranscriptContext,
    options: StreamOptions,
) -> AssistantMessageEventStream {
    let stream = AssistantMessageEventStream::new();
    let worker = stream.clone();

    tokio::spawn(async move {
        let mut output = crate::types::new_pending_assistant(&model);
        let tools = declared_tools(&context.messages);
        if let Err(err) = run_openai(model, context, options, tools, worker.clone(), &mut output).await {
            output.stop_reason = StopReason::Error;
            output.error_message = Some(err.to_string());
            worker.push(crate::types::AssistantMessageEvent::Error {
                reason: ErrorStopReason::Error,
                error: output.clone(),
            });
            worker.end(None);
        }
    });

    stream
}

async fn run_openai(
    model: Model,
    context: TranscriptContext,
    options: StreamOptions,
    tools: Vec<Tool>,
    stream: AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), crate::error::ZiAiError> {
    let api_key = require_api_key(&model.provider, options.api_key.as_deref(), None)?;

    let max_tokens = options
        .max_tokens
        .unwrap_or(model.max_tokens)
        .max(16)
        .min(model.max_tokens);

    let mut body = json!({
        "model": model.id,
        "input": openai_responses_input(&context.messages),
        "stream": true,
        "store": false,
        "max_output_tokens": max_tokens,
    });

    if !tools.is_empty() {
        body["tools"] = json!(openai_tools(&tools));
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }

    let url = format!(
        "{}/responses",
        model.base_url.trim_end_matches('/')
    );

    let client = Client::new();
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(crate::error::ZiAiError::Provider(format!(
            "OpenAI HTTP {status}: {text}"
        )));
    }

    stream.push(crate::types::AssistantMessageEvent::Start {
        partial: output.clone(),
    });

    let byte_stream = response.bytes_stream();
    let events = decode_sse(byte_stream.map(|r| r.map_err(reqwest::Error::from)));
    futures::pin_mut!(events);
    let mut slots: HashMap<u64, OutputSlot> = HashMap::new();

    while let Some(frame) = events.next().await {
        let frame = frame?;
        let Some(value) = parse_json_data(&frame.data) else {
            continue;
        };
        debug!(target: "zi_ai::openai", event = ?frame.event, "sse frame");

        let event_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(frame.event.as_deref().unwrap_or(""));

        match event_type {
            "response.created" => {
                if let Some(id) = value
                    .pointer("/response/id")
                    .and_then(|v| v.as_str())
                {
                    output.response_id = Some(id.to_string());
                }
            }
            "response.output_item.added" => {
                let output_index = value.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                create_slot(output_index, &item, output, &stream, &mut slots);
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                let output_index = value.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let delta = value.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(slot) = slots.get(&output_index) {
                    if let AssistantContentBlock::Text { text, .. } = &mut output.content[slot.content_index] {
                        text.push_str(delta);
                    }
                    stream.push(crate::types::AssistantMessageEvent::TextDelta {
                        content_index: slot.content_index,
                        delta: delta.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta" => {
                let output_index = value.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let delta = value.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(slot) = slots.get(&output_index) {
                    if let AssistantContentBlock::Thinking { thinking, .. } =
                        &mut output.content[slot.content_index]
                    {
                        thinking.push_str(delta);
                    }
                    stream.push(crate::types::AssistantMessageEvent::ThinkingDelta {
                        content_index: slot.content_index,
                        delta: delta.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                let output_index = value.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let delta = value.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(slot) = slots.get_mut(&output_index) {
                    if let SlotKind::ToolCall { arguments } = &mut slot.kind {
                        let parsed = arguments.append(delta);
                        if let AssistantContentBlock::ToolCall { arguments: dest, .. } =
                            &mut output.content[slot.content_index]
                        {
                            *dest = parsed;
                        }
                        stream.push(crate::types::AssistantMessageEvent::ToolcallDelta {
                            content_index: slot.content_index,
                            delta: delta.to_string(),
                            partial: output.clone(),
                        });
                    }
                }
            }
            "response.output_item.done" => {
                let output_index = value.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                if !slots.contains_key(&output_index) {
                    create_slot(output_index, &item, output, &stream, &mut slots);
                }
                if let Some(slot) = slots.remove(&output_index) {
                    finalize_slot(slot, &item, output, &stream);
                }
            }
            "response.completed" | "response.incomplete" => {
                if let Some(response) = value.get("response") {
                    if let Some(id) = response.get("id").and_then(|v| v.as_str()) {
                        output.response_id = Some(id.to_string());
                    }
                    apply_openai_usage(output, response.get("usage"), &model);
                    let status = response.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
                    let incomplete_reason = response
                        .pointer("/incomplete_details/reason")
                        .and_then(|v| v.as_str());
                    output.raw_stop_reason = Some(match incomplete_reason {
                        Some(r) => format!("{status}.{r}"),
                        None => status.to_string(),
                    });
                    output.stop_reason = map_openai_stop(status, incomplete_reason);
                }
            }
            _ => {}
        }
    }

    if output.stop_reason == StopReason::Pending {
        output.stop_reason = StopReason::Stop;
    }

    if output.content.iter().any(|b| matches!(b, AssistantContentBlock::ToolCall { .. }))
        && output.stop_reason == StopReason::Stop
    {
        output.stop_reason = StopReason::ToolUse;
    }

    if matches!(output.stop_reason, StopReason::Error | StopReason::Aborted) {
        return Err(crate::error::ZiAiError::Provider(
            output
                .error_message
                .clone()
                .unwrap_or_else(|| "openai stream failed".into()),
        ));
    }

    let done_reason = match output.stop_reason {
        StopReason::Length => DoneStopReason::Length,
        StopReason::ToolUse => DoneStopReason::ToolUse,
        StopReason::Deferred => DoneStopReason::Deferred,
        _ => DoneStopReason::Stop,
    };

    stream.push(crate::types::AssistantMessageEvent::Done {
        reason: done_reason,
        message: output.clone(),
    });
    stream.end(None);
    Ok(())
}

fn create_slot(
    output_index: u64,
    item: &Value,
    output: &mut AssistantMessage,
    stream: &AssistantMessageEventStream,
    slots: &mut HashMap<u64, OutputSlot>,
) {
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match item_type {
        "message" => {
            output.content.push(AssistantContentBlock::Text {
                text: String::new(),
                text_signature: None,
            });
            let content_index = output.content.len() - 1;
            slots.insert(
                output_index,
                OutputSlot {
                    content_index,
                    kind: SlotKind::Text,
                },
            );
            stream.push(crate::types::AssistantMessageEvent::TextStart {
                content_index,
                partial: output.clone(),
            });
        }
        "reasoning" => {
            output.content.push(AssistantContentBlock::Thinking {
                thinking: String::new(),
                thinking_signature: None,
                redacted: None,
            });
            let content_index = output.content.len() - 1;
            slots.insert(
                output_index,
                OutputSlot {
                    content_index,
                    kind: SlotKind::Thinking,
                },
            );
            stream.push(crate::types::AssistantMessageEvent::ThinkingStart {
                content_index,
                partial: output.clone(),
            });
        }
        "function_call" => {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args_text = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
            let mut arg_stream = ToolCallArgumentsStream::default();
            let arguments = if args_text.is_empty() {
                json!({})
            } else {
                arg_stream.append(args_text)
            };
            output.content.push(AssistantContentBlock::ToolCall {
                id,
                name,
                arguments,
                thought_signature: None,
                namespace: item.get("namespace").and_then(|v| v.as_str()).map(str::to_string),
                arguments_error: None,
            });
            let content_index = output.content.len() - 1;
            slots.insert(
                output_index,
                OutputSlot {
                    content_index,
                    kind: SlotKind::ToolCall {
                        arguments: arg_stream,
                    },
                },
            );
            stream.push(crate::types::AssistantMessageEvent::ToolcallStart {
                content_index,
                partial: output.clone(),
            });
        }
        _ => {}
    }
}

fn finalize_slot(
    slot: OutputSlot,
    item: &Value,
    output: &mut AssistantMessage,
    stream: &AssistantMessageEventStream,
) {
    match slot.kind {
        SlotKind::Text => {
            if let AssistantContentBlock::Text { text, .. } = &mut output.content[slot.content_index] {
                if text.is_empty() {
                    if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                        *text = content
                            .iter()
                            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                    }
                }
            }
            let content = match &output.content[slot.content_index] {
                AssistantContentBlock::Text { text, .. } => text.clone(),
                _ => String::new(),
            };
            stream.push(crate::types::AssistantMessageEvent::TextEnd {
                content_index: slot.content_index,
                content,
                partial: output.clone(),
            });
        }
        SlotKind::Thinking => {
            if let AssistantContentBlock::Thinking {
                thinking,
                thinking_signature,
                ..
            } = &mut output.content[slot.content_index]
            {
                if thinking.is_empty() {
                    *thinking = item
                        .pointer("/summary")
                        .and_then(|v| v.as_array())
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        })
                        .unwrap_or_default();
                }
                *thinking_signature = Some(item.to_string());
            }
            let content = match &output.content[slot.content_index] {
                AssistantContentBlock::Thinking { thinking, .. } => thinking.clone(),
                _ => String::new(),
            };
            stream.push(crate::types::AssistantMessageEvent::ThinkingEnd {
                content_index: slot.content_index,
                content,
                partial: output.clone(),
            });
        }
        SlotKind::ToolCall { arguments } => {
            let (parsed, arguments_error) = arguments.finish();
            if let AssistantContentBlock::ToolCall {
                arguments: dest,
                arguments_error: err_slot,
                ..
            } = &mut output.content[slot.content_index]
            {
                *dest = parsed;
                *err_slot = arguments_error;
            }
            if let Some(tool_call) = output.content[slot.content_index].as_tool_call() {
                stream.push(crate::types::AssistantMessageEvent::ToolcallEnd {
                    content_index: slot.content_index,
                    tool_call,
                    partial: output.clone(),
                });
            }
        }
    }
}

fn apply_openai_usage(output: &mut AssistantMessage, usage: Option<&Value>, model: &Model) {
    let Some(usage) = usage else {
        return;
    };
    let cached = usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_write = usage
        .pointer("/input_tokens_details/cache_write_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let input = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    output.usage.input = input.saturating_sub(cached + cache_write);
    output.usage.output = output_tokens;
    output.usage.cache_read = cached;
    output.usage.cache_write = cache_write;
    output.usage.reasoning = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(|v| v.as_u64());
    output.usage.total_tokens = usage.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or_else(|| {
        output.usage.input + output.usage.output + output.usage.cache_read + output.usage.cache_write
    });
    calculate_cost(model, &mut output.usage);
}

fn map_openai_stop(status: &str, incomplete_reason: Option<&str>) -> StopReason {
    match (status, incomplete_reason) {
        ("incomplete", Some("max_output_tokens")) => StopReason::Length,
        ("incomplete", _) => StopReason::Length,
        ("failed", _) | ("cancelled", _) => StopReason::Error,
        _ => StopReason::Stop,
    }
}

/// Convenience entry matching pi `streamSimple` for OpenAI Responses.
pub fn stream_simple(
    model: Model,
    context: TranscriptContext,
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    stream(model, context, options.stream)
}
