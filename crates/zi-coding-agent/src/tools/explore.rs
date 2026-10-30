//! Explore subagent: scoped read-only search that returns a summary into the parent transcript.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use zi_agent::{
    run_agent_loop, AgentContext, AgentLoopConfig, AgentTool, AgentToolResult, DefaultStreamFn,
    ToolExecutionMode,
};
use zi_ai::{
    Message, Model, SimpleStreamOptions, SystemContent, SystemMessage, SystemRole, UserContent,
    UserMessage, UserRole,
};
use serde_json::{json, Value};

use super::{find_tool, grep_tool, ls_tool, ReadTool};

const DEFAULT_MAX_TURNS: usize = 12;

/// Nested read-only explorer that keeps intermediate tool dumps out of the parent context.
pub struct ExploreTool {
    cwd: PathBuf,
    model: Model,
    stream_options: SimpleStreamOptions,
}

impl ExploreTool {
    /// Bind explore to a workspace and parent model credentials.
    pub fn new(cwd: PathBuf, model: Model, stream_options: SimpleStreamOptions) -> Self {
        Self {
            cwd,
            model,
            stream_options,
        }
    }

    fn explore_tools(&self) -> Vec<Arc<dyn AgentTool>> {
        vec![
            Arc::new(ReadTool::new(self.cwd.clone())) as Arc<dyn AgentTool>,
            grep_tool(self.cwd.clone()) as Arc<dyn AgentTool>,
            find_tool(self.cwd.clone()) as Arc<dyn AgentTool>,
            ls_tool(self.cwd.clone()) as Arc<dyn AgentTool>,
        ]
    }
}

#[async_trait]
impl AgentTool for ExploreTool {
    fn name(&self) -> &str {
        "explore"
    }

    fn description(&self) -> &str {
        "Explored: read-only subagent (read/grep/find/ls, default 12 turns) that returns \
         only a summary (+ Key paths). Call this tool directly for broad / architecture \
         questions when you do not yet know which files matter — do not wrap it in \
         codemode or bash. Prefer parent read/grep/find/ls for point lookups."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "What to learn about the codebase (question or exploration goal)"
                },
                "query": {
                    "type": "string",
                    "description": "Alias for goal"
                },
                "maxTurns": {
                    "type": "number",
                    "description": "Max assistant turns for the subagent (default 12, max 16)"
                }
            },
            "required": ["goal"]
        })
    }

    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        _on_update: Option<zi_agent::AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        let goal = match args
            .get("goal")
            .or_else(|| args.get("query"))
            .and_then(|v| v.as_str())
        {
            Some(g) if !g.trim().is_empty() => g.trim(),
            _ => return AgentToolResult::error("missing required parameter: goal"),
        };
        let max_turns = args
            .get("maxTurns")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, 16))
            .unwrap_or(DEFAULT_MAX_TURNS);

        let system = Message::System(SystemMessage {
            role: SystemRole::System,
            content: SystemContent::Text(format!(
                "You are a read-only codebase explorer in `{}`. Use read/grep/find/ls only.\n\
                 Do not edit files. Prefer README and package manifests first.\n\
                 Use the full turn budget; do not stop early to save tool calls.\n\
                 When done, write a concise structured summary (bullets) answering the goal.\n\
                 Every non-trivial claim must cite a path you actually inspected \
                 (e.g. `packages/agent/src/agent-loop.ts`). Never invent packages, entry \
                 points, or call graphs. Unread is not UNKNOWN — list unread paths under \
                 'Unread for parent' instead of inventing structure.\n\
                 End with a short 'Key paths' list of the most important files you inspected.\n\
                 Do not call tools in your final answer.",
                self.cwd.display()
            )),
            sections: None,
            tools_added: None,
            tools_removed: None,
            timestamp: wall_now_ms(),
        });
        let user = Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text(goal.to_string()),
            timestamp: wall_now_ms(),
        });

        let mut context = AgentContext {
            messages: vec![system, user],
            tools: self.explore_tools(),
            deferred_tools: Vec::new(),
            explore_calls_since_user: 0,
        };

        let mut config = AgentLoopConfig::new(self.model.clone());
        config.stream_options = self.stream_options.clone();
        config.tool_execution = ToolExecutionMode::Parallel;
        config.auto_compact = true;
        config.keep_recent_tool_result_tokens = 8_000;
        config.max_turns = Some(max_turns);
        // Subagent capacity is max_turns; parent cheap-search budget must not self-lock explore.
        config.explore_budget_warn = 0;
        config.explore_budget_block = 0;

        let emit = Arc::new(|_event| {});
        let abort = Arc::new(AtomicBool::new(false));
        let stream_fn = Arc::new(DefaultStreamFn);

        match run_agent_loop(
            Vec::new(),
            &mut context,
            config,
            emit,
            abort,
            stream_fn,
        )
        .await
        {
            Ok(_) => {
                let summary = last_assistant_text(&context.messages).unwrap_or_else(|| {
                    "Explore finished without a text summary.".to_string()
                });
                let tool_rounds = context
                    .messages
                    .iter()
                    .filter(|m| matches!(m, Message::ToolResult(_)))
                    .count();
                AgentToolResult::text(format!(
                    "Explore summary (subagent tool results={tool_rounds}, max_turns={max_turns}):\n\n{summary}"
                ))
            }
            Err(err) => AgentToolResult::error(format!("explore subagent failed: {err}")),
        }
    }
}

fn last_assistant_text(messages: &[Message]) -> Option<String> {
    for message in messages.iter().rev() {
        if let Message::Assistant(a) = message {
            let text: String = a
                .content
                .iter()
                .filter_map(|b| match b {
                    zi_ai::AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
