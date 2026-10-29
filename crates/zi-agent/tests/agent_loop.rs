// Agent loop integration tests with a canned stream function.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use zi_agent::{
    run_agent_loop, Agent, AgentContext, AgentEvent, AgentLoopConfig, AgentOptions, AgentPromptInput,
    AgentTool, AgentToolResult, FnAgentTool, StreamFn, ToolExecutionMode,
};
use zi_ai::{
    create_assistant_message_event_stream, new_pending_assistant, AssistantContentBlock,
    AssistantMessage, AssistantMessageEvent, AssistantRole, DoneStopReason, Message, Model,
    ModelCost, StopReason,     UserContent, UserMessage, UserRole,
};
use serde_json::json;

fn test_model() -> Model {
    Model {
        id: "test-model".into(),
        name: "Test".into(),
        api: "anthropic-messages".into(),
        provider: "test".into(),
        base_url: String::new(),
        reasoning: false,
        context_window: 8192,
        max_tokens: 1024,
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    }
}

fn user_message(text: &str) -> Message {
    Message::User(UserMessage {
        role: UserRole::User,
        content: UserContent::Text(text.into()),
        timestamp: 1,
    })
}

fn assistant_text(text: &str, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContentBlock::Text {
            text: text.into(),
            text_signature: None,
        }],
        api: "anthropic-messages".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_model: None,
        response_id: None,
        usage: Default::default(),
        stop_reason: stop,
        error_message: None,
        raw_stop_reason: None,
        timestamp: 2,
        duration_ms: None,
    }
}

fn assistant_with_tool_call() -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContentBlock::ToolCall {
            id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({ "value": "hi" }),
            thought_signature: None,
            namespace: None,
            arguments_error: None,
        }],
        api: "anthropic-messages".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_model: None,
        response_id: None,
        usage: Default::default(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        raw_stop_reason: None,
        timestamp: 2,
        duration_ms: None,
    }
}

struct CannedStreamFn {
    queue: Mutex<VecDeque<AssistantMessage>>,
    model: Model,
}

impl CannedStreamFn {
    fn new(model: Model, responses: Vec<AssistantMessage>) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(responses.into()),
            model,
        })
    }

    fn push_done(stream: &zi_ai::AssistantMessageEventStream, message: AssistantMessage) {
        let partial = new_pending_assistant(&test_model());
        stream.push(AssistantMessageEvent::Start { partial });
        let reason = match message.stop_reason {
            StopReason::ToolUse => DoneStopReason::ToolUse,
            StopReason::Length => DoneStopReason::Length,
            _ => DoneStopReason::Stop,
        };
        stream.push(AssistantMessageEvent::Done { reason, message });
    }
}

#[async_trait]
impl StreamFn for CannedStreamFn {
    async fn stream(
        &self,
        model: &Model,
        _context: zi_ai::TranscriptContext,
        _options: zi_ai::SimpleStreamOptions,
    ) -> zi_ai::AssistantMessageEventStream {
        let _ = model;
        let next = self
            .queue
            .lock()
            .expect("canned queue")
            .pop_front()
            .expect("unexpected stream call");
        let stream = create_assistant_message_event_stream();
        Self::push_done(&stream, next);
        stream
    }
}

fn echo_tool() -> Arc<dyn AgentTool> {
    FnAgentTool::new(
        "echo",
        "Echo a string",
        json!({ "type": "object", "properties": { "value": { "type": "string" } } }),
        |_id, args, _on_update| async move {
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            AgentToolResult::text(value)
        },
    )
}

#[tokio::test]
async fn single_turn_without_tools() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![assistant_text("hello", StopReason::Stop)],
    );
    let config = AgentLoopConfig::new(model);
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![echo_tool()],
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> =
        Arc::new(move |event| events_clone.lock().expect("events").push(event));

    let new_messages = run_agent_loop(
        vec![user_message("ping")],
        &mut context,
        config,
        sink,
        Arc::new(AtomicBool::new(false)),
        stream_fn,
    )
    .await
    .expect("loop");

    assert_eq!(new_messages.len(), 2);
    assert!(matches!(
        new_messages.last(),
        Some(Message::Assistant(msg)) if msg.content.iter().any(|b| matches!(b, AssistantContentBlock::Text { text, .. } if text == "hello"))
    ));

    let types: Vec<String> = events
        .lock()
        .expect("events")
        .iter()
        .map(|event| match event {
            AgentEvent::AgentStart => "agent_start".into(),
            AgentEvent::AgentEnd { .. } => "agent_end".into(),
            AgentEvent::TurnStart => "turn_start".into(),
            AgentEvent::TurnEnd { .. } => "turn_end".into(),
            AgentEvent::MessageStart { .. } => "message_start".into(),
            AgentEvent::MessageEnd { .. } => "message_end".into(),
            AgentEvent::ToolExecutionStart { .. } => "tool_execution_start".into(),
            _ => "other".into(),
        })
        .collect();
    assert!(types.contains(&"agent_start".to_string()));
    assert!(types.contains(&"agent_end".to_string()));
}

#[tokio::test]
async fn tool_call_then_final_text() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![
            assistant_with_tool_call(),
            assistant_text("done", StopReason::Stop),
        ],
    );
    let config = AgentLoopConfig::new(model);
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![echo_tool()],
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> =
        Arc::new(move |event| events_clone.lock().expect("events").push(event));

    run_agent_loop(
        vec![user_message("use tool")],
        &mut context,
        config,
        sink,
        Arc::new(AtomicBool::new(false)),
        stream_fn,
    )
    .await
    .expect("loop");

    assert_eq!(context.messages.len(), 4);
    assert!(matches!(context.messages[3], Message::Assistant(_)));

    let tool_starts = events
        .lock()
        .expect("events")
        .iter()
        .filter(|event| matches!(event, AgentEvent::ToolExecutionStart { .. }))
        .count();
    assert_eq!(tool_starts, 1);
}

#[tokio::test]
async fn abort_before_second_stream_call() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![
            assistant_with_tool_call(),
            assistant_text("should-not-run", StopReason::Stop),
        ],
    );
    let config = AgentLoopConfig::new(model);
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![echo_tool()],
    };
    let abort = Arc::new(AtomicBool::new(false));
    let abort_set = abort.clone();
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(move |event| {
        if matches!(event, AgentEvent::ToolExecutionEnd { .. }) {
            abort_set.store(true, Ordering::Relaxed);
        }
    });

    run_agent_loop(
        vec![user_message("abort")],
        &mut context,
        config,
        sink,
        abort,
        stream_fn,
    )
    .await
    .expect("loop");

    assert_eq!(context.messages.len(), 3);
}

#[tokio::test]
async fn agent_prompt_subscribes_and_updates_transcript() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![assistant_text("from agent", StopReason::Stop)],
    );
    let agent = Agent::new(AgentOptions {
        model,
        stream_fn,
        tools: vec![echo_tool()],
        initial_messages: Vec::new(),
        stream_options: Default::default(),
        tool_execution: ToolExecutionMode::Parallel,
        event_capacity: 32,
    });

    let mut rx = agent.subscribe();
    agent
        .prompt(AgentPromptInput::Text("hello"))
        .await
        .expect("prompt");

    let mut saw_end = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, AgentEvent::AgentEnd { .. }) {
            saw_end = true;
        }
    }
    assert!(saw_end);

    let messages = agent.messages().await;
    assert_eq!(messages.len(), 2);
}
