// Stream function abstraction (aligned with pi agent StreamFn).

use async_trait::async_trait;
use zi_ai::{
    stream_transcript, AssistantMessageEventStream, Model, SimpleStreamOptions, TranscriptContext,
};

/// LLM streaming entry point for the agent loop.
#[async_trait]
pub trait StreamFn: Send + Sync {
    /// Return an assistant message event stream for one provider request.
    async fn stream(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream;
}

/// Default implementation delegating to [`zi_ai::stream_transcript`].
pub struct DefaultStreamFn;

#[async_trait]
impl StreamFn for DefaultStreamFn {
    async fn stream(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        stream_transcript(model.clone(), context, options)
    }
}
