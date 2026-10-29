// Anthropic Messages API streaming (aligned with pi anthropic-messages.ts).

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
use crate::transcript::{anthropic_messages, anthropic_tools, declared_tools, leading_system_text};
use crate::types::{
    AssistantContentBlock, AssistantMessage, DoneStopReason, ErrorStopReason, Model, SimpleStreamOptions,
    StopReason, StreamOptions, Tool, TranscriptContext,
};

struct OpenBlock {
    content_index: usize,
    arguments: Option<ToolCallArgumentsStream>,
}

/// Stream an assistant message via Anthropic Messages SSE.
pub fn stream(
    model: Model,
    context: TranscriptContext,
    options: StreamOptions,
) -> AssistantMessageEventStream {
    let stream = AssistantMessageEventStream::new();
    let worker_stream = stream.clone();

    tokio::spawn(async move {
        let mut output = crate::types::new_pending_assistant(&model);
        let tools = declared_tools(&context.messages);

        let result = run_anthropic(model, context, options, tools, worker_stream.clone(), &mut output).await;
        if let Err(err) = result {
            output.stop_reason = StopReason::Error;
            output.error_message = Some(err.to_string());
            worker_stream.push(crate::types::AssistantMessageEvent::Error {
                reason: ErrorStopReason::Error,
                error: output.clone(),
            });
            worker_stream.end(None);
        }
    });

    stream
}

async fn run_anthropic(
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
        .min(model.max_tokens);

    let mut body = json!({
        "model": model.id,
        "max_tokens": max_tokens,
        "messages": anthropic_messages(&context.messages),
        "stream": true,
    });

    if let Some(system) = leading_system_text(&context.messages) {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = json!(anthropic_tools(&tools));
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }

    let url = format!(
        "{}/v1/messages",
        model.base_url.trim_end_matches('/')
    );

    let client = Client::new();
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(crate::error::ZiAiError::Provider(format!(
            "Anthropic HTTP {status}: {text}"
        )));
    }

    stream.push(crate::types::AssistantMessageEvent::Start {
        partial: output.clone(),
    });

    let byte_stream = response.bytes_stream();
    let events = decode_sse(byte_stream.map(|r| r.map_err(reqwest::Error::from)));
    futures::pin_mut!(events);

    let mut open_blocks: HashMap<u64, OpenBlock> = HashMap::new();

    while let Some(frame) = events.next().await {
        let frame = frame?;
        let Some(value) = parse_json_data(&frame.data) else {
            continue;
        };
        debug!(target: "zi_ai::anthropic", event = ?frame.event, "sse frame");

        let event_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(frame.event.as_deref().unwrap_or(""));

        match event_type {
            "message_start" => {
                if let Some(message) = value.get("message") {
                    if let Some(id) = message.get("id").and_then(|v| v.as_str()) {
                        output.response_id = Some(id.to_string());
                    }
                    if let Some(usage) = message.get("usage") {
                        apply_anthropic_usage(output, usage, &model);
                    }
                }
            }
            "content_block_start" => {
                let index = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let block = value.get("content_block").cloned().unwrap_or(Value::Null);
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        output.content.push(AssistantContentBlock::Text {
                            text: text.to_string(),
                            text_signature: None,
                        });
                        let content_index = output.content.len() - 1;
                        open_blocks.insert(index, OpenBlock {
                            content_index,
                            arguments: None,
                        });
                        stream.push(crate::types::AssistantMessageEvent::TextStart {
                            content_index,
                            partial: output.clone(),
                        });
                    }
                    "thinking" | "redacted_thinking" => {
                        let (thinking, signature, redacted) = if block_type == "redacted_thinking" {
                            (
                                "[Reasoning redacted]".to_string(),
                                block.get("data").and_then(|v| v.as_str()).map(str::to_string),
                                Some(true),
                            )
                        } else {
                            (
                                block
                                    .get("thinking")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                block.get("signature").and_then(|v| v.as_str()).map(str::to_string),
                                None,
                            )
                        };
                        output.content.push(AssistantContentBlock::Thinking {
                            thinking,
                            thinking_signature: signature,
                            redacted,
                        });
                        let content_index = output.content.len() - 1;
                        open_blocks.insert(index, OpenBlock {
                            content_index,
                            arguments: None,
                        });
                        stream.push(crate::types::AssistantMessageEvent::ThinkingStart {
                            content_index,
                            partial: output.clone(),
                        });
                    }
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or(json!({}));
                        output.content.push(AssistantContentBlock::ToolCall {
                            id,
                            name,
                            arguments: input,
                            thought_signature: None,
                            namespace: None,
                            arguments_error: None,
                        });
                        let content_index = output.content.len() - 1;
                        open_blocks.insert(index, OpenBlock {
                            content_index,
                            arguments: Some(ToolCallArgumentsStream::default()),
                        });
                        stream.push(crate::types::AssistantMessageEvent::ToolcallStart {
                            content_index,
                            partial: output.clone(),
                        });
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let Some(open) = open_blocks.get_mut(&index) else {
                    continue;
                };
                let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        let piece = delta.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if let AssistantContentBlock::Text { text, .. } =
                            &mut output.content[open.content_index]
                        {
                            text.push_str(piece);
                        }
                        stream.push(crate::types::AssistantMessageEvent::TextDelta {
                            content_index: open.content_index,
                            delta: piece.to_string(),
                            partial: output.clone(),
                        });
                    }
                    "thinking_delta" => {
                        let piece = delta.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                        if let AssistantContentBlock::Thinking { thinking, .. } =
                            &mut output.content[open.content_index]
                        {
                            thinking.push_str(piece);
                        }
                        stream.push(crate::types::AssistantMessageEvent::ThinkingDelta {
                            content_index: open.content_index,
                            delta: piece.to_string(),
                            partial: output.clone(),
                        });
                    }
                    "input_json_delta" => {
                        let piece = delta.get("partial_json").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(args) = open.arguments.as_mut() {
                            let parsed = args.append(piece);
                            if let AssistantContentBlock::ToolCall { arguments, .. } =
                                &mut output.content[open.content_index]
                            {
                                *arguments = parsed;
                            }
                            stream.push(crate::types::AssistantMessageEvent::ToolcallDelta {
                                content_index: open.content_index,
                                delta: piece.to_string(),
                                partial: output.clone(),
                            });
                        }
                    }
                    "signature_delta" => {
                        let piece = delta.get("signature").and_then(|v| v.as_str()).unwrap_or("");
                        if let AssistantContentBlock::Thinking {
                            thinking_signature,
                            ..
                        } = &mut output.content[open.content_index]
                        {
                            if thinking_signature.is_none() {
                                *thinking_signature = Some(String::new());
                            }
                            if let Some(sig) = thinking_signature {
                                sig.push_str(piece);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let Some(open) = open_blocks.remove(&index) else {
                    continue;
                };
                match &output.content[open.content_index] {
                    AssistantContentBlock::Text { text, .. } => {
                        stream.push(crate::types::AssistantMessageEvent::TextEnd {
                            content_index: open.content_index,
                            content: text.clone(),
                            partial: output.clone(),
                        });
                    }
                    AssistantContentBlock::Thinking { thinking, .. } => {
                        stream.push(crate::types::AssistantMessageEvent::ThinkingEnd {
                            content_index: open.content_index,
                            content: thinking.clone(),
                            partial: output.clone(),
                        });
                    }
                    AssistantContentBlock::ToolCall { .. } => {
                        if let Some(args) = open.arguments {
                            let (arguments, arguments_error) = args.finish();
                            if let AssistantContentBlock::ToolCall {
                                arguments: dest,
                                arguments_error: err_slot,
                                ..
                            } = &mut output.content[open.content_index]
                            {
                                *dest = arguments;
                                *err_slot = arguments_error;
                            }
                        }
                        if let Some(tool_call) = output.content[open.content_index].as_tool_call() {
                            stream.push(crate::types::AssistantMessageEvent::ToolcallEnd {
                                content_index: open.content_index,
                                tool_call,
                                partial: output.clone(),
                            });
                        }
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = value.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|v| v.as_str()) {
                        output.raw_stop_reason = Some(reason.to_string());
                        output.stop_reason = map_anthropic_stop(reason);
                    }
                }
                if let Some(usage) = value.get("usage") {
                    apply_anthropic_usage(output, usage, &model);
                }
            }
            "message_stop" => {}
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
                .unwrap_or_else(|| "anthropic stream failed".into()),
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

fn apply_anthropic_usage(output: &mut AssistantMessage, usage: &Value, model: &Model) {
    if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
        output.usage.input = v;
    }
    if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
        output.usage.output = v;
    }
    if let Some(v) = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
        output.usage.cache_read = v;
    }
    if let Some(v) = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()) {
        output.usage.cache_write = v;
    }
    output.usage.total_tokens = output.usage.input
        + output.usage.output
        + output.usage.cache_read
        + output.usage.cache_write;
    calculate_cost(model, &mut output.usage);
}

fn map_anthropic_stop(raw: &str) -> StopReason {
    match raw {
        "end_turn" => StopReason::Stop,
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        "pause_turn" | "stop_sequence" => StopReason::Stop,
        _ => StopReason::Stop,
    }
}

/// Convenience entry matching pi `streamSimple` for Anthropic.
pub fn stream_simple(
    model: Model,
    context: TranscriptContext,
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    stream(model, context, options.stream)
}
