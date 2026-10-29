// Public streaming entry points.

use crate::api::{anthropic, google_generative_ai, openai_completions, openai_responses};
use crate::auth::require_api_key;
use crate::error::ZiAiError;
use crate::event_stream::AssistantMessageEventStream;
use crate::transcript::normalize_context;
use crate::types::{
    Context, ErrorStopReason, KnownApi, Message, Model, SimpleStreamOptions, StopReason, Tool, TranscriptContext,
};

/// Dispatch a streaming request by model API (pi `streamSimple` shape).
pub fn stream_simple(
    model: Model,
    messages: Vec<Message>,
    tools: &[Tool],
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    let context = normalize_context(Context {
        system_prompt: None,
        messages,
        tools: if tools.is_empty() {
            None
        } else {
            Some(tools.to_vec())
        },
    });

    if let Err(err) = require_api_key(&model.provider, options.stream.api_key.as_deref(), None) {
        return error_stream(&model, err);
    }

    match KnownApi::parse(&model.api) {
        Some(KnownApi::AnthropicMessages) => anthropic::stream_simple(model, context, options),
        Some(KnownApi::OpenaiResponses) => openai_responses::stream_simple(model, context, options),
        Some(KnownApi::OpenaiCompletions) => {
            openai_completions::stream_simple(model, context, options)
        }
        Some(KnownApi::GoogleGenerativeAi) => {
            google_generative_ai::stream_simple(model, context, options)
        }
        None => error_stream(
            &model,
            ZiAiError::UnsupportedApi {
                api: model.api.clone(),
            },
        ),
    }
}

/// Stream with an already-normalized transcript.
pub fn stream_transcript(
    model: Model,
    context: TranscriptContext,
    options: SimpleStreamOptions,
) -> AssistantMessageEventStream {
    if let Err(err) = require_api_key(&model.provider, options.stream.api_key.as_deref(), None) {
        return error_stream(&model, err);
    }

    match KnownApi::parse(&model.api) {
        Some(KnownApi::AnthropicMessages) => anthropic::stream_simple(model, context, options),
        Some(KnownApi::OpenaiResponses) => openai_responses::stream_simple(model, context, options),
        Some(KnownApi::OpenaiCompletions) => {
            openai_completions::stream_simple(model, context, options)
        }
        Some(KnownApi::GoogleGenerativeAi) => {
            google_generative_ai::stream_simple(model, context, options)
        }
        None => error_stream(
            &model,
            ZiAiError::UnsupportedApi {
                api: model.api.clone(),
            },
        ),
    }
}

fn error_stream(model: &Model, err: ZiAiError) -> AssistantMessageEventStream {
    let stream = AssistantMessageEventStream::new();
    let mut message = crate::types::new_pending_assistant(model);
    message.stop_reason = StopReason::Error;
    message.error_message = Some(err.to_string());
    stream.push(crate::types::AssistantMessageEvent::Error {
        reason: ErrorStopReason::Error,
        error: message.clone(),
    });
    stream.end(Some(message));
    stream
}
