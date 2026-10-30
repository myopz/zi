// Stateful agent wrapper: prompt, abort, event subscription.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use zi_ai::{Message, Model, UserContent, UserContentBlock, UserMessage, UserRole};
use tokio::sync::{broadcast, Mutex, Notify};
use tracing::debug;

use crate::agent_loop::{run_agent_loop, AgentEventSink};
use crate::error::AgentError;
use crate::stream_fn::StreamFn;
use crate::types::{
    AgentContext, AgentEvent, AgentEventReceiver, AgentLoopConfig, AgentMessage, AgentTool,
    ToolExecutionMode,
};

/// Stateful agent that owns the transcript and runs the loop on `prompt`.
pub struct Agent {
    inner: Mutex<AgentInner>,
    stream_fn: Arc<dyn StreamFn>,
    loop_config: Mutex<AgentLoopConfig>,
    event_tx: broadcast::Sender<AgentEvent>,
    abort: Arc<AtomicBool>,
    idle: Notify,
    running: AtomicBool,
}

struct AgentInner {
    messages: Vec<AgentMessage>,
    tools: Vec<Arc<dyn AgentTool>>,
    deferred_tools: Vec<Arc<dyn AgentTool>>,
}

/// Options for constructing an [`Agent`].
pub struct AgentOptions {
    pub model: Model,
    pub stream_fn: Arc<dyn StreamFn>,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub deferred_tools: Vec<Arc<dyn AgentTool>>,
    pub initial_messages: Vec<AgentMessage>,
    pub stream_options: zi_ai::SimpleStreamOptions,
    pub tool_execution: ToolExecutionMode,
    pub event_capacity: usize,
}

impl AgentOptions {
    /// Build options with defaults for stream settings and event buffer size.
    pub fn new(model: Model, stream_fn: Arc<dyn StreamFn>) -> Self {
        Self {
            model: model.clone(),
            stream_fn,
            tools: Vec::new(),
            deferred_tools: Vec::new(),
            initial_messages: Vec::new(),
            stream_options: zi_ai::SimpleStreamOptions::default(),
            tool_execution: ToolExecutionMode::Parallel,
            event_capacity: 256,
        }
    }
}

impl Agent {
    /// Create an agent with the given model and stream function.
    pub fn new(options: AgentOptions) -> Self {
        let mut loop_config = AgentLoopConfig::new(options.model);
        loop_config.stream_options = options.stream_options;
        loop_config.tool_execution = options.tool_execution;
        let (event_tx, _) = broadcast::channel(options.event_capacity);
        Self {
            inner: Mutex::new(AgentInner {
                messages: options.initial_messages,
                tools: options.tools,
                deferred_tools: options.deferred_tools,
            }),
            stream_fn: options.stream_fn,
            loop_config: Mutex::new(loop_config),
            event_tx,
            abort: Arc::new(AtomicBool::new(false)),
            idle: Notify::new(),
            running: AtomicBool::new(false),
        }
    }

    /// Current model used for the next turn.
    pub async fn model(&self) -> Model {
        self.loop_config.lock().await.model.clone()
    }

    /// Switch model and stream options for subsequent turns.
    pub async fn set_model(&self, model: Model, stream_options: zi_ai::SimpleStreamOptions) {
        let mut cfg = self.loop_config.lock().await;
        cfg.model = model;
        cfg.stream_options = stream_options;
    }

    /// Subscribe to agent lifecycle events.
    pub fn subscribe(&self) -> AgentEventReceiver {
        self.event_tx.subscribe()
    }

    /// Conversation transcript (copy of stored messages).
    pub async fn messages(&self) -> Vec<AgentMessage> {
        self.inner.lock().await.messages.clone()
    }

    /// Replace executable tools for subsequent runs.
    pub async fn set_tools(&self, tools: Vec<Arc<dyn AgentTool>>) {
        self.inner.lock().await.tools = tools;
    }

    /// Replace deferred tools (mounted on first request by name).
    pub async fn set_deferred_tools(&self, tools: Vec<Arc<dyn AgentTool>>) {
        self.inner.lock().await.deferred_tools = tools;
    }

    /// Update loop config fields used for compaction / budgets / archives.
    pub async fn configure_loop<F>(&self, f: F)
    where
        F: FnOnce(&mut AgentLoopConfig),
    {
        let mut cfg = self.loop_config.lock().await;
        f(&mut cfg);
    }

    /// Abort the current run, if any.
    pub fn abort(&self) {
        self.abort.store(true, Ordering::Relaxed);
    }

    /// Whether a prompt loop is currently executing.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Wait until the current run finishes.
    pub async fn wait_for_idle(&self) {
        while self.running.load(Ordering::Relaxed) {
            self.idle.notified().await;
        }
    }

    /// Replace the stored transcript (e.g. `/new`, `/clear`, `/resume`).
    pub async fn replace_messages(&self, messages: Vec<AgentMessage>) {
        self.inner.lock().await.messages = messages;
    }

    /// Compact the in-memory transcript (LLM summary with structural fallback).
    pub async fn compact_messages(&self) -> Option<crate::StructuralCompaction> {
        let mut inner = self.inner.lock().await;
        let cfg = self.loop_config.lock().await.clone();
        let archive_path = cfg.transcript_archive_dir.as_ref().map(|dir| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            dir.join(format!("compact-{ts}.txt"))
        });
        let compacted = crate::summarize_compact(
            &inner.messages,
            crate::SummarizeCompactOptions {
                keep_recent_tokens: cfg.keep_recent_message_tokens,
                stream_fn: self.stream_fn.clone(),
                model: cfg.model.clone(),
                stream_options: cfg.stream_options.clone(),
                abort: None,
                archive_path,
            },
        )
        .await?;
        inner.messages = compacted.messages.clone();
        let kept_messages: Vec<_> = compacted
            .messages
            .iter()
            .filter(|m| !matches!(m, Message::System(_)))
            .filter(|m| match m {
                Message::User(u) => match &u.content {
                    UserContent::Text(t) => !t.contains("<compaction>"),
                    _ => true,
                },
                _ => true,
            })
            .cloned()
            .collect();
        let tx = self.event_tx.clone();
        let _ = tx.send(AgentEvent::Compaction {
            summary: compacted.summary.clone(),
            tokens_before: compacted.tokens_before,
            tokens_after: compacted.tokens_after,
            dropped_messages: compacted.dropped_messages,
            kept_messages,
        });
        Some(compacted)
    }

    /// Start a new turn from text or structured messages.
    pub async fn prompt(&self, input: AgentPromptInput<'_>) -> Result<(), AgentError> {
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return Err(AgentError::AlreadyRunning);
        }

        self.abort.store(false, Ordering::Relaxed);
        let prompts = input.into_messages();
        let inner = self.inner.lock().await;
        let mut context = AgentContext {
            messages: inner.messages.clone(),
            tools: inner.tools.clone(),
            deferred_tools: inner.deferred_tools.clone(),
            explore_calls_since_user: 0,
        };
        drop(inner);

        let emit = self.make_emit_sink();
        let abort = self.abort.clone();
        let stream_fn = self.stream_fn.clone();
        let config = self.loop_config.lock().await.clone();

        let result = run_agent_loop(prompts, &mut context, config, emit, abort, stream_fn).await;

        if result.is_ok() {
            let mut inner = self.inner.lock().await;
            inner.messages = context.messages;
            inner.tools = context.tools;
            inner.deferred_tools = context.deferred_tools;
        }

        self.running.store(false, Ordering::Relaxed);
        self.idle.notify_waiters();
        result.map(|_| ())
    }

    fn make_emit_sink(&self) -> AgentEventSink {
        let tx = self.event_tx.clone();
        Arc::new(move |event| {
            if tx.send(event).is_err() {
                debug!("agent event dropped: no subscribers");
            }
        })
    }
}

/// Input to [`Agent::prompt`].
pub enum AgentPromptInput<'a> {
    Text(&'a str),
    Messages(Vec<AgentMessage>),
}

impl<'a> AgentPromptInput<'a> {
    fn into_messages(self) -> Vec<AgentMessage> {
        match self {
            Self::Text(text) => vec![Message::User(UserMessage {
                role: UserRole::User,
                content: UserContent::Blocks(vec![UserContentBlock::Text(zi_ai::TextContent {
                    kind: zi_ai::TextContentKind::Text,
                    text: text.to_string(),
                    text_signature: None,
                })]),
                timestamp: wall_now_ms(),
            })],
            Self::Messages(messages) => messages,
        }
    }
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
