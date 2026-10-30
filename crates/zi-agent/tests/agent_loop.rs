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
    assistant_with_named_tool("echo", json!({ "value": "hi" }))
}

fn assistant_with_named_tool(name: &str, arguments: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContentBlock::ToolCall {
            id: format!("call-{name}"),
            name: name.into(),
            arguments,
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
        deferred_tools: Vec::new(),
        explore_calls_since_user: 0,
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
        deferred_tools: Vec::new(),
        explore_calls_since_user: 0,
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
        deferred_tools: Vec::new(),
        explore_calls_since_user: 0,
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
        deferred_tools: Vec::new(),
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

#[tokio::test]
async fn compact_messages_dedupes_repeated_architecture_edits() {
    use zi_ai::{
        TextContent, TextContentKind, ToolResultContentBlock, ToolResultMessage, ToolResultRole,
    };

    let edit_line =
        "Updated /Users/zhangjianlong/workspace/forkmess/pi/ARCHITECTURE.md (1 edit(s))";
    let mut initial = vec![user_message("write ARCHITECTURE.md")];
    for i in 0..11i64 {
        initial.push(Message::ToolResult(ToolResultMessage {
            role: ToolResultRole::ToolResult,
            tool_call_id: format!("e{i}"),
            tool_name: "edit".into(),
            content: vec![ToolResultContentBlock::Text(TextContent {
                kind: TextContentKind::Text,
                text: edit_line.into(),
                text_signature: None,
            })],
            details: None,
            is_error: false,
            timestamp: i,
        }));
        initial.push(Message::ToolResult(ToolResultMessage {
            role: ToolResultRole::ToolResult,
            tool_call_id: format!("b{i}"),
            tool_name: "bash".into(),
            content: vec![ToolResultContentBlock::Text(TextContent {
                kind: TextContentKind::Text,
                text: format!("noise-{}", "x".repeat(400)),
                text_signature: None,
            })],
            details: None,
            is_error: false,
            timestamp: i,
        }));
    }

    let model = test_model();
    let summary = "## Goal\nwrite ARCHITECTURE.md\n\n## Progress\n### Done\n- [x] Updated ARCHITECTURE.md\n\n## Next Steps\n1. stop\n";
    let agent = Agent::new(AgentOptions {
        model: model.clone(),
        stream_fn: CannedStreamFn::new(model, vec![assistant_text(summary, StopReason::Stop)]),
        deferred_tools: Vec::new(),
        tools: vec![],
        initial_messages: initial,
        stream_options: Default::default(),
        tool_execution: ToolExecutionMode::Parallel,
        event_capacity: 32,
    });
    agent
        .configure_loop(|cfg| {
            cfg.keep_recent_message_tokens = 200;
        })
        .await;

    let compacted = agent.compact_messages().await.expect("should compact");
    assert!(
        compacted.summary.contains("ARCHITECTURE.md") || compacted.summary.contains("## Goal"),
        "summary={}",
        compacted.summary
    );

    let messages = agent.messages().await;
    let edit_count = messages
        .iter()
        .filter(|m| match m {
            Message::User(u) => match &u.content {
                UserContent::Text(t) => {
                    t.contains("<pinned-edit>") && t.contains("ARCHITECTURE.md")
                }
                _ => false,
            },
            _ => false,
        })
        .count();
    assert_eq!(
        edit_count, 1,
        "Agent::compact_messages must leave a single ARCHITECTURE pinned note"
    );
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, Message::ToolResult(tr) if tr.tool_name == "edit")),
        "pinned edits must not remain as orphan toolResults"
    );
}

fn read_tool() -> Arc<dyn AgentTool> {
    FnAgentTool::new(
        "read",
        "Read a file",
        json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        |_id, _args, _on_update| async move { AgentToolResult::text("file body") },
    )
}

fn explore_ok_tool() -> Arc<dyn AgentTool> {
    FnAgentTool::new(
        "explore",
        "Explore",
        json!({ "type": "object", "properties": { "goal": { "type": "string" } } }),
        |_id, _args, _on_update| async move {
            AgentToolResult::text("Explore summary\n\nKey paths:\n- a.ts")
        },
    )
}

#[tokio::test]
async fn explore_budget_zero_allows_cheap_search_when_exhausted() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![
            assistant_with_named_tool("read", json!({ "path": "a.ts" })),
            assistant_text("ok", StopReason::Stop),
        ],
    );
    let mut config = AgentLoopConfig::new(model);
    config.explore_budget_block = 0;
    config.explore_budget_warn = 0;
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![read_tool()],
        deferred_tools: Vec::new(),
        explore_calls_since_user: 99,
    };
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(|_| {});
    run_agent_loop(
        vec![user_message("read")],
        &mut context,
        config,
        sink,
        Arc::new(AtomicBool::new(false)),
        stream_fn,
    )
    .await
    .expect("loop");

    let tool_result = context
        .messages
        .iter()
        .find_map(|m| match m {
            Message::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("tool result");
    assert!(
        !tool_result.is_error,
        "budget 0 must not block: {:?}",
        tool_result.content
    );
}

#[tokio::test]
async fn successful_explore_resets_cheap_search_counter() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![
            assistant_with_named_tool("explore", json!({ "goal": "arch" })),
            assistant_with_named_tool("read", json!({ "path": "a.ts" })),
            assistant_text("done", StopReason::Stop),
        ],
    );
    let mut config = AgentLoopConfig::new(model);
    config.explore_budget_block = 12;
    config.tool_execution = ToolExecutionMode::Sequential;
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![explore_ok_tool(), read_tool()],
        deferred_tools: Vec::new(),
        explore_calls_since_user: 12,
    };
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(|_| {});
    run_agent_loop(
        vec![user_message("explore then read")],
        &mut context,
        config,
        sink,
        Arc::new(AtomicBool::new(false)),
        stream_fn,
    )
    .await
    .expect("loop");

    let reads: Vec<_> = context
        .messages
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(tr) if tr.tool_name == "read" => Some(tr),
            _ => None,
        })
        .collect();
    assert_eq!(reads.len(), 1);
    assert!(
        !reads[0].is_error,
        "read after explore must succeed; got {:?}",
        reads[0].content
    );
    assert_eq!(
        context.explore_calls_since_user, 0,
        "explore resets to 0; read is not budgeted so counter stays 0"
    );
}

fn ls_tool() -> Arc<dyn AgentTool> {
    FnAgentTool::new(
        "ls",
        "List",
        json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        |_id, _args, _on_update| async move { AgentToolResult::text("dir/") },
    )
}

#[tokio::test]
async fn read_not_blocked_when_ls_budget_exhausted() {
    let model = test_model();
    let stream_fn = CannedStreamFn::new(
        model.clone(),
        vec![
            assistant_with_named_tool("read", json!({ "path": "a.ts" })),
            assistant_text("ok", StopReason::Stop),
        ],
    );
    let mut config = AgentLoopConfig::new(model);
    config.explore_budget_block = 12;
    let mut context = AgentContext {
        messages: Vec::new(),
        tools: vec![read_tool(), ls_tool()],
        deferred_tools: Vec::new(),
        explore_calls_since_user: 12,
    };
    let sink: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(|_| {});
    run_agent_loop(
        vec![user_message("read known path")],
        &mut context,
        config,
        sink,
        Arc::new(AtomicBool::new(false)),
        stream_fn,
    )
    .await
    .expect("loop");

    let tool_result = context
        .messages
        .iter()
        .find_map(|m| match m {
            Message::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("tool result");
    assert!(
        !tool_result.is_error && tool_result.tool_name == "read",
        "read must not be budget-blocked: {:?}",
        tool_result.content
    );
}
