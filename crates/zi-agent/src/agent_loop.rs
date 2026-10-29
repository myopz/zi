// Agent loop: prompt, stream assistant, execute tools (aligned with pi agent-loop.ts).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use zi_ai::{
    AssistantContentBlock, AssistantMessage, AssistantMessageEvent, Context, Message, StopReason,
    Tool, ToolResultMessage, ToolResultRole,
};
use zi_ai::transcript::normalize_context;
use serde_json::Value;
use tracing::debug;

use crate::stream_fn::StreamFn;
use crate::types::{
    AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AgentTool, AgentToolCallOutcome,
    AgentToolResult, ToolExecutionMode,
};

/// Sink for agent lifecycle events.
pub type AgentEventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

/// Run the agent loop from new user (or system) prompts until the model stops or the run aborts.
pub async fn run_agent_loop(
    prompts: Vec<AgentMessage>,
    context: &mut AgentContext,
    config: AgentLoopConfig,
    emit: AgentEventSink,
    abort: Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
) -> Result<Vec<AgentMessage>, crate::error::AgentError> {
    let mut new_messages = Vec::new();

    emit(AgentEvent::AgentStart);

    for message in prompts {
        emit(AgentEvent::MessageStart {
            message: message.clone(),
        });
        emit(AgentEvent::MessageEnd {
            message: message.clone(),
        });
        context.messages.push(message.clone());
        new_messages.push(message);
    }

    run_inner_loop(
        context,
        &mut new_messages,
        &config,
        &emit,
        &abort,
        stream_fn,
    )
    .await?;

    emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    });
    Ok(new_messages)
}

async fn run_inner_loop(
    context: &mut AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
) -> Result<(), crate::error::AgentError> {
    loop {
        if abort.load(Ordering::Relaxed) {
            return Ok(());
        }

        emit(AgentEvent::TurnStart);

        let assistant =
            stream_assistant_response(context, config, emit, abort, stream_fn.clone()).await?;

        new_messages.push(Message::Assistant(assistant.clone()));

        if matches!(
            assistant.stop_reason,
            StopReason::Error | StopReason::Aborted
        ) {
            let tool_results = Vec::new();
            emit(AgentEvent::TurnEnd {
                message: Message::Assistant(assistant),
                tool_results: tool_results.clone(),
            });
            return Ok(());
        }

        let tool_calls = tool_calls_from_assistant(&assistant);
        if tool_calls.is_empty() {
            emit(AgentEvent::TurnEnd {
                message: Message::Assistant(assistant),
                tool_results: Vec::new(),
            });
            return Ok(());
        }

        let tool_results = if assistant.stop_reason == StopReason::Length {
            fail_truncated_tool_calls(&tool_calls, emit).await
        } else {
            execute_tool_calls(context, &assistant, &tool_calls, config, emit, abort).await
        };

        for result in &tool_results {
            context.messages.push(Message::ToolResult(result.clone()));
            new_messages.push(Message::ToolResult(result.clone()));
        }

        emit(AgentEvent::TurnEnd {
            message: Message::Assistant(assistant),
            tool_results: tool_results.clone(),
        });

        if abort.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
}

async fn stream_assistant_response(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
    stream_fn: Arc<dyn StreamFn>,
) -> Result<AssistantMessage, crate::error::AgentError> {
    let llm_messages = default_convert_to_llm(&context.messages);
    let tools = agent_tools_for_llm(&context.tools);
    let transcript = normalize_context(Context {
        system_prompt: None,
        messages: llm_messages,
        tools,
    });

    let response = stream_fn
        .stream(
            &config.model,
            transcript,
            config.stream_options.clone(),
        )
        .await;

    let mut event_stream = response.events();
    let mut partial_started = false;

    while let Some(event) = event_stream.next().await {
        if abort.load(Ordering::Relaxed) {
            debug!("agent loop abort during assistant stream");
            break;
        }

        match &event {
            AssistantMessageEvent::Start { partial } => {
                context.messages.push(Message::Assistant(partial.clone()));
                partial_started = true;
                emit(AgentEvent::MessageStart {
                    message: Message::Assistant(partial.clone()),
                });
            }
            AssistantMessageEvent::TextStart { partial, .. }
            | AssistantMessageEvent::TextDelta { partial, .. }
            | AssistantMessageEvent::TextEnd { partial, .. }
            | AssistantMessageEvent::ThinkingStart { partial, .. }
            | AssistantMessageEvent::ThinkingDelta { partial, .. }
            | AssistantMessageEvent::ThinkingEnd { partial, .. }
            | AssistantMessageEvent::ToolcallStart { partial, .. }
            | AssistantMessageEvent::ToolcallDelta { partial, .. }
            | AssistantMessageEvent::ToolcallEnd { partial, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = partial.clone();
                    }
                    emit(AgentEvent::MessageUpdate {
                        message: Message::Assistant(partial.clone()),
                        assistant_message_event: event.clone(),
                    });
                }
            }
            AssistantMessageEvent::Done { message, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = message.clone();
                    }
                } else {
                    context.messages.push(Message::Assistant(message.clone()));
                    emit(AgentEvent::MessageStart {
                        message: Message::Assistant(message.clone()),
                    });
                }
                emit(AgentEvent::MessageEnd {
                    message: Message::Assistant(message.clone()),
                });
                return Ok(message.clone());
            }
            AssistantMessageEvent::Error { error, .. } => {
                if partial_started {
                    if let Some(Message::Assistant(last)) = context.messages.last_mut() {
                        *last = error.clone();
                    }
                } else {
                    context.messages.push(Message::Assistant(error.clone()));
                    emit(AgentEvent::MessageStart {
                        message: Message::Assistant(error.clone()),
                    });
                }
                emit(AgentEvent::MessageEnd {
                    message: Message::Assistant(error.clone()),
                });
                return Ok(error.clone());
            }
        }
    }

    let fallback = response.result().await;
    if !partial_started {
        context.messages.push(Message::Assistant(fallback.clone()));
        emit(AgentEvent::MessageStart {
            message: Message::Assistant(fallback.clone()),
        });
    } else if let Some(Message::Assistant(last)) = context.messages.last_mut() {
        *last = fallback.clone();
    }
    emit(AgentEvent::MessageEnd {
        message: Message::Assistant(fallback.clone()),
    });
    Ok(fallback)
}

fn agent_tools_for_llm(tools: &[Arc<dyn AgentTool>]) -> Option<Vec<Tool>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|tool| Tool {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters_schema(),
            })
            .collect(),
    )
}

fn default_convert_to_llm(messages: &[AgentMessage]) -> Vec<AgentMessage> {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                Message::System(_) | Message::User(_) | Message::Assistant(_) | Message::ToolResult(_)
            )
        })
        .cloned()
        .collect()
}

#[derive(Clone)]
struct ParsedToolCall {
    id: String,
    name: String,
    arguments: Value,
}

fn tool_calls_from_assistant(message: &AssistantMessage) -> Vec<ParsedToolCall> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(ParsedToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        })
        .collect()
}

async fn fail_truncated_tool_calls(
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
) -> Vec<ToolResultMessage> {
    let mut messages = Vec::new();
    for call in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.arguments.clone(),
        });
        let result = AgentToolResult::error(format!(
            "Tool call \"{}\" was not executed: the response hit the output token limit.",
            call.name
        ));
        emit(AgentEvent::ToolExecutionEnd {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            result: result.clone(),
            is_error: true,
            duration_ms: None,
        });
        let tool_result = tool_result_message(&call.id, &call.name, &result, true, 0);
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

async fn execute_tool_calls(
    context: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    config: &AgentLoopConfig,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let sequential = config.tool_execution == ToolExecutionMode::Sequential
        || tool_calls.iter().any(|call| {
            find_tool(context, &call.name)
                .and_then(|tool| tool.execution_mode())
                .map(|mode| mode == ToolExecutionMode::Sequential)
                .unwrap_or(false)
        });

    if sequential {
        execute_tool_calls_sequential(context, assistant, tool_calls, emit, abort).await
    } else {
        execute_tool_calls_parallel(context, assistant, tool_calls, emit, abort).await
    }
}

async fn execute_tool_calls_sequential(
    context: &AgentContext,
    _assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let mut messages = Vec::new();
    for call in tool_calls {
        if abort.load(Ordering::Relaxed) {
            break;
        }
        let outcome = run_one_tool_call(context, call, emit, abort).await;
        let tool_result = outcome_to_tool_result(&outcome);
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

async fn execute_tool_calls_parallel(
    context: &AgentContext,
    _assistant: &AssistantMessage,
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    join_prepared_tool_calls(context, tool_calls, emit, abort).await
}

async fn join_prepared_tool_calls(
    context: &AgentContext,
    tool_calls: &[ParsedToolCall],
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> Vec<ToolResultMessage> {
    let mut join_set = tokio::task::JoinSet::new();
    let mut immediate = Vec::new();

    for (index, call) in tool_calls.iter().enumerate() {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.arguments.clone(),
        });

        if abort.load(Ordering::Relaxed) {
            let result = AgentToolResult::error("Operation aborted");
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                result: result.clone(),
                is_error: true,
                duration_ms: None,
            });
            immediate.push(outcome_to_tool_result(&AgentToolCallOutcome {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                result,
                is_error: true,
                duration_ms: 0,
            }));
            continue;
        }

        let Some(tool) = find_tool(context, &call.name) else {
            let result = AgentToolResult::error(format!("Tool {} not found", call.name));
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                result: result.clone(),
                is_error: true,
                duration_ms: None,
            });
            immediate.push(outcome_to_tool_result(&AgentToolCallOutcome {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                result,
                is_error: true,
                duration_ms: 0,
            }));
            continue;
        };

        let call_id = call.id.clone();
        let call_name = call.name.clone();
        let args = call.arguments.clone();
        let emit = emit.clone();
        let abort = abort.clone();
        join_set.spawn(async move {
            let started = Instant::now();
            let emit_update = emit.clone();
            let call_id_u = call_id.clone();
            let call_name_u = call_name.clone();
            let args_u = args.clone();
            let on_update: Option<crate::types::AgentToolUpdateCallback> =
                Some(Box::new(move |partial| {
                    emit_update(AgentEvent::ToolExecutionUpdate {
                        tool_call_id: call_id_u.clone(),
                        tool_name: call_name_u.clone(),
                        args: args_u.clone(),
                        partial_result: partial,
                    });
                }));

            let result = if abort.load(Ordering::Relaxed) {
                AgentToolResult::error("Operation aborted")
            } else {
                tool.execute(&call_id, args.clone(), on_update).await
            };
            let is_error = result.is_error.unwrap_or(false);
            let duration_ms = started.elapsed().as_millis() as u64;
            emit(AgentEvent::ToolExecutionEnd {
                tool_call_id: call_id.clone(),
                tool_name: call_name.clone(),
                result: result.clone(),
                is_error,
                duration_ms: Some(duration_ms),
            });
            (
                index,
                AgentToolCallOutcome {
                    tool_call_id: call_id,
                    tool_name: call_name,
                    args,
                    result,
                    is_error,
                    duration_ms,
                },
            )
        });
    }

    let mut spawned = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        if let Ok(outcome) = joined {
            spawned.push(outcome);
        }
    }
    spawned.sort_by_key(|(index, _)| *index);

    let mut messages = immediate;
    for (_, outcome) in spawned {
        let tool_result = outcome_to_tool_result(&outcome);
        emit_tool_result_message(&tool_result, emit);
        messages.push(tool_result);
    }
    messages
}

async fn run_one_tool_call(
    context: &AgentContext,
    call: &ParsedToolCall,
    emit: &AgentEventSink,
    abort: &Arc<AtomicBool>,
) -> AgentToolCallOutcome {
    emit(AgentEvent::ToolExecutionStart {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.arguments.clone(),
    });

    let started = Instant::now();
    let call_id = call.id.clone();
    let call_name = call.name.clone();
    let args = call.arguments.clone();

    let on_update: Option<crate::types::AgentToolUpdateCallback> = Some(Box::new({
        let emit = emit.clone();
        let call_id = call_id.clone();
        let call_name = call_name.clone();
        let args = args.clone();
        move |partial| {
            emit(AgentEvent::ToolExecutionUpdate {
                tool_call_id: call_id.clone(),
                tool_name: call_name.clone(),
                args: args.clone(),
                partial_result: partial,
            });
        }
    }));

    let (result, is_error) = if abort.load(Ordering::Relaxed) {
        (AgentToolResult::error("Operation aborted"), true)
    } else if let Some(tool) = find_tool(context, &call.name) {
        let result = tool
            .execute(&call.id, call.arguments.clone(), on_update)
            .await;
        let is_error = result.is_error.unwrap_or(false);
        (result, is_error)
    } else {
        (
            AgentToolResult::error(format!("Tool {} not found", call.name)),
            true,
        )
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        result: result.clone(),
        is_error,
        duration_ms: Some(duration_ms),
    });

    AgentToolCallOutcome {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.arguments.clone(),
        result,
        is_error,
        duration_ms,
    }
}

fn find_tool(context: &AgentContext, name: &str) -> Option<Arc<dyn AgentTool>> {
    context
        .tools
        .iter()
        .find(|tool| tool.name() == name)
        .cloned()
}

fn outcome_to_tool_result(outcome: &AgentToolCallOutcome) -> ToolResultMessage {
    tool_result_message(
        &outcome.tool_call_id,
        &outcome.tool_name,
        &outcome.result,
        outcome.is_error,
        outcome.duration_ms,
    )
}

fn tool_result_message(
    tool_call_id: &str,
    tool_name: &str,
    result: &AgentToolResult,
    is_error: bool,
    _duration_ms: u64,
) -> ToolResultMessage {
    ToolResultMessage {
        role: ToolResultRole::ToolResult,
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        content: result
            .content
            .iter()
            .cloned()
            .map(zi_ai::ToolResultContentBlock::Text)
            .collect(),
        details: Some(result.details.clone()),
        is_error,
        timestamp: wall_now_ms(),
    }
}

fn emit_tool_result_message(message: &ToolResultMessage, emit: &AgentEventSink) {
    emit(AgentEvent::MessageStart {
        message: Message::ToolResult(message.clone()),
    });
    emit(AgentEvent::MessageEnd {
        message: Message::ToolResult(message.clone()),
    });
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
