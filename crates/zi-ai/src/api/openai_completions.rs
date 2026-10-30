// OpenAI Chat Completions API streaming (Phase 4 subset).

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
use crate::transcript::{declared_tools, openai_chat_messages, openai_chat_tools};
use crate::types::{
    AssistantContentBlock, AssistantMessage, DoneStopReason, ErrorStopReason, Model, SimpleStreamOptions,
    StopReason, StreamOptions, Tool, TranscriptContext,
};

struct ToolSlot {
    content_index: usize,
    id: String,
    name: String,
    arguments: ToolCallArgumentsStream,
}

/// Stream an assistant message via OpenAI Chat Completions SSE.
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
        if let Err(err) = run_chat(model, context, options, tools, worker.clone(), &mut output).await {
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

async fn run_chat(
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
        "messages": openai_chat_messages(&context.messages),
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": max_tokens,
    });

    if !tools.is_empty() {
        body["tools"] = json!(openai_chat_tools(&tools));
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }

    let url = format!(
        "{}/chat/completions",
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
            "OpenAI chat HTTP {status}: {text}"
        )));
    }

    stream.push(crate::types::AssistantMessageEvent::Start {
        partial: output.clone(),
    });

    let byte_stream = response.bytes_stream();
    let events = decode_sse(byte_stream.map(|r| r.map_err(reqwest::Error::from)));
    futures::pin_mut!(events);
    let mut text_index: Option<usize> = None;
    let mut tool_slots: HashMap<u64, ToolSlot> = HashMap::new();

    while let Some(frame) = events.next().await {
        let frame = frame?;
        let Some(value) = parse_json_data(&frame.data) else {
            continue;
        };
        debug!(target: "zi_ai::openai_chat", "sse chunk");

        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
            output.response_id = Some(id.to_string());
        }
        if let Some(usage) = value.get("usage") {
            apply_chat_usage(output, usage, &model);
        }

        let choice = value
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first());

        let Some(choice) = choice else {
            continue;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                if !content.is_empty() {
                    let idx = ensure_text_block(output, &stream, &mut text_index);
                    if let AssistantContentBlock::Text { text, .. } = &mut output.content[idx] {
                        text.push_str(content);
                    }
                    stream.push(crate::types::AssistantMessageEvent::TextDelta {
                        content_index: idx,
                        delta: content.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tool_calls {
                    let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    let slot = tool_slots.entry(index).or_insert_with(|| {
                        let content_index = output.content.len();
                        output.content.push(AssistantContentBlock::ToolCall {
                            id: String::new(),
                            name: String::new(),
                            arguments: json!({}),
                            thought_signature: None,
                            namespace: None,
                            arguments_error: None,
                        });
                        stream.push(crate::types::AssistantMessageEvent::ToolcallStart {
                            content_index,
                            partial: output.clone(),
                        });
                        ToolSlot {
                            content_index,
                            id: String::new(),
                            name: String::new(),
                            arguments: ToolCallArgumentsStream::default(),
                        }
                    });
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        slot.id = id.to_string();
                    }
                    if let Some(name) = tc.pointer("/function/name").and_then(|v| v.as_str()) {
                        slot.name = name.to_string();
                    }
                    if let Some(args_delta) = tc.pointer("/function/arguments").and_then(|v| v.as_str())
                    {
                        let parsed = slot.arguments.append(args_delta);
                        if let AssistantContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } = &mut output.content[slot.content_index]
                        {
                            *id = slot.id.clone();
                            *name = slot.name.clone();
                            *arguments = parsed;
                        }
                        stream.push(crate::types::AssistantMessageEvent::ToolcallDelta {
                            content_index: slot.content_index,
                            delta: args_delta.to_string(),
                            partial: output.clone(),
                        });
                    }
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            output.raw_stop_reason = Some(reason.to_string());
            output.stop_reason = map_chat_stop(reason);
        }
    }

    if let Some(idx) = text_index {
        if let AssistantContentBlock::Text { text, .. } = &output.content[idx] {
            stream.push(crate::types::AssistantMessageEvent::TextEnd {
                content_index: idx,
                content: text.clone(),
                partial: output.clone(),
            });
        }
    }
    for (_, slot) in tool_slots {
        let (args, err) = slot.arguments.finish();
        if let AssistantContentBlock::ToolCall {
            id,
            name,
            arguments,
            arguments_error,
            ..
        } = &mut output.content[slot.content_index]
        {
            *id = slot.id;
            *name = slot.name;
            *arguments = args;
            *arguments_error = err;
        }
        if let Some(block) = output.content[slot.content_index].as_tool_call() {
            stream.push(crate::types::AssistantMessageEvent::ToolcallEnd {
                content_index: slot.content_index,
                tool_call: block,
                partial: output.clone(),
            });
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

fn ensure_text_block(
    output: &mut AssistantMessage,
    stream: &AssistantMessageEventStream,
    text_index: &mut Option<usize>,
) -> usize {
    if let Some(idx) = *text_index {
        return idx;
    }
    let idx = output.content.len();
    output.content.push(AssistantContentBlock::Text {
        text: String::new(),
        text_signature: None,
    });
    stream.push(crate::types::AssistantMessageEvent::TextStart {
        content_index: idx,
        partial: output.clone(),
    });
    *text_index = Some(idx);
    idx
}

fn map_chat_stop(raw: &str) -> StopReason {
    match raw {
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "stop" => StopReason::Stop,
        _ => StopReason::Stop,
    }
}

fn apply_chat_usage(output: &mut AssistantMessage, usage: &Value, model: &Model) {
    let prompt = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // DeepSeek (and some gateways): prompt_tokens = hit + miss.
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);
    let cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|v| v.as_u64());
    output.usage.cache_read = cache_hit;
    output.usage.input = cache_miss.unwrap_or_else(|| prompt.saturating_sub(cache_hit));
    output.usage.output = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    output.usage.total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(prompt + output.usage.output);
    calculate_cost(model, &mut output.usage);
}

/// Convenience entry matching pi `streamSimple` for OpenAI Chat Completions.
pub fn stream_simple(
    model: Model,
    context: TranscriptContext,
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    stream(model, context, options.stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AssistantRole, ModelCost, StopReason};

    #[test]
    fn maps_deepseek_prompt_cache_hit_and_miss() {
        let model = Model {
            id: "deepseek-flash".into(),
            name: "DeepSeek Flash".into(),
            api: "openai-completions".into(),
            provider: "deepseek".into(),
            base_url: String::new(),
            reasoning: false,
            context_window: 128_000,
            max_tokens: 4096,
            cost: ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.0,
            },
        };
        let mut output = AssistantMessage {
            role: AssistantRole::Assistant,
            content: vec![],
            api: "openai-completions".into(),
            provider: "deepseek".into(),
            model: "deepseek-flash".into(),
            response_model: None,
            response_id: None,
            usage: Default::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            raw_stop_reason: None,
            timestamp: 0,
            duration_ms: None,
        };
        let usage = serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 50,
            "total_tokens": 1050,
            "prompt_cache_hit_tokens": 700,
            "prompt_cache_miss_tokens": 300,
        });
        apply_chat_usage(&mut output, &usage, &model);
        assert_eq!(output.usage.cache_read, 700);
        assert_eq!(output.usage.input, 300);
        assert_eq!(output.usage.output, 50);
        assert_eq!(output.usage.total_tokens, 1050);
    }
}
