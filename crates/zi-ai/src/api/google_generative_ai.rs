// Google Generative AI streaming via REST SSE (Phase 4 subset).

use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::debug;

use crate::auth::require_api_key;
use crate::cost::calculate_cost;
use crate::event_stream::AssistantMessageEventStream;
use crate::sse::{decode_sse, parse_json_data};
use crate::transcript::{declared_tools, google_contents, google_tools};
use crate::types::{
    AssistantContentBlock, AssistantMessage, DoneStopReason, ErrorStopReason, Model, SimpleStreamOptions,
    StopReason, StreamOptions, Tool, TranscriptContext,
};

/// Stream an assistant message via Google `streamGenerateContent` SSE.
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
        if let Err(err) = run_google(model, context, options, tools, worker.clone(), &mut output).await
        {
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

async fn run_google(
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

    let (system_instruction, contents) = google_contents(&context.messages);
    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": max_tokens,
        },
    });
    if let Some(system) = system_instruction {
        body["systemInstruction"] = system;
    }
    if !tools.is_empty() {
        body["tools"] = google_tools(&tools);
    }
    if let Some(temp) = options.temperature {
        body["generationConfig"]["temperature"] = json!(temp);
    }

    let base = model.base_url.trim_end_matches('/');
    let url = format!(
        "{base}/v1beta/models/{}:streamGenerateContent?alt=sse",
        model.id
    );

    let client = Client::new();
    let response = client
        .post(url)
        .query(&[("key", api_key.as_str())])
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(crate::error::ZiAiError::Provider(format!(
            "Google HTTP {status}: {text}"
        )));
    }

    stream.push(crate::types::AssistantMessageEvent::Start {
        partial: output.clone(),
    });

    let byte_stream = response.bytes_stream();
    let events = decode_sse(byte_stream.map(|r| r.map_err(reqwest::Error::from)));
    futures::pin_mut!(events);
    let mut text_index: Option<usize> = None;

    while let Some(frame) = events.next().await {
        let frame = frame?;
        let Some(value) = parse_json_data(&frame.data) else {
            continue;
        };
        debug!(target: "zi_ai::google", "sse chunk");

        if let Some(id) = value.get("responseId").and_then(|v| v.as_str()) {
            output.response_id = Some(id.to_string());
        }
        if let Some(usage) = value.get("usageMetadata") {
            apply_google_usage(output, usage, &model);
        }

        let parts = value
            .pointer("/candidates/0/content/parts")
            .and_then(|v| v.as_array());

        let Some(parts) = parts else {
            continue;
        };

        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                let is_thought = part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_thought {
                    continue;
                }
                if text.is_empty() {
                    continue;
                }
                let idx = ensure_text_block(output, &stream, &mut text_index);
                if let AssistantContentBlock::Text { text: dest, .. } = &mut output.content[idx] {
                    dest.push_str(text);
                }
                stream.push(crate::types::AssistantMessageEvent::TextDelta {
                    content_index: idx,
                    delta: text.to_string(),
                    partial: output.clone(),
                });
            }
            if let Some(call) = part.get("functionCall") {
                let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                let content_index = output.content.len();
                output.content.push(AssistantContentBlock::ToolCall {
                    id: format!("{name}_{content_index}"),
                    name: name.to_string(),
                    arguments: args,
                    thought_signature: None,
                    namespace: None,
                    arguments_error: None,
                });
                stream.push(crate::types::AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: output.clone(),
                });
                if let Some(block) = output.content[content_index].as_tool_call() {
                    stream.push(crate::types::AssistantMessageEvent::ToolcallEnd {
                        content_index,
                        tool_call: block,
                        partial: output.clone(),
                    });
                }
            }
        }

        if let Some(finish) = value.pointer("/candidates/0/finishReason").and_then(|v| v.as_str()) {
            output.raw_stop_reason = Some(finish.to_string());
            output.stop_reason = map_google_stop(finish);
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

fn map_google_stop(raw: &str) -> StopReason {
    match raw {
        "MAX_TOKENS" => StopReason::Length,
        "STOP" => StopReason::Stop,
        _ if raw.contains("TOOL") => StopReason::ToolUse,
        _ => StopReason::Stop,
    }
}

fn apply_google_usage(output: &mut AssistantMessage, usage: &Value, model: &Model) {
    output.usage.input = usage
        .get("promptTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    output.usage.output = usage
        .get("candidatesTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    output.usage.total_tokens = usage
        .get("totalTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(output.usage.input + output.usage.output);
    calculate_cost(model, &mut output.usage);
}

/// Convenience entry matching pi `streamSimple` for Google Generative AI.
pub fn stream_simple(
    model: Model,
    context: TranscriptContext,
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    stream(model, context, options.stream)
}
