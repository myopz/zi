//! LLM-backed conversation compaction (Cursor/pi-style structured summary).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use zi_ai::{
    AssistantMessageEvent, Context, Message, StopReason, UserContent, UserMessage, UserRole,
};

use crate::context_budget::{
    collect_pins_from_dropped, estimate_messages_tokens, keep_tail_by_tokens, latest_user_goal,
    repair_orphaned_tool_results_pub, strip_pinned_dupes_from_kept, structural_compact,
    tool_result_text_pub, StructuralCompaction, CHARS_PER_TOKEN,
};
use crate::stream_fn::StreamFn;

/// Soft cap on summarizer input (~32k tokens as chars/4).
const MAX_SUMMARY_INPUT_CHARS: usize = 32_000 * CHARS_PER_TOKEN;

const SUMMARIZATION_SYSTEM: &str = "You summarize coding-agent transcripts into structured checkpoints. \
Output only the summary markdown. Do not call tools. Do not continue the user's task.";

const SUMMARIZATION_PROMPT: &str = r#"The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish?]

## Constraints & Preferences
- [Or "(none)"]

## Progress
### Done
- [x] [Completed work]

### In Progress
- [ ] [Current work]

### Blocked
- [Or "(none)"]

## Key Decisions
- **[Decision]**: [Rationale]

## Next Steps
1. [Ordered next actions]

## Critical Context
- [Paths, errors, APIs needed to continue — or "(none)"]

Keep each section concise. Preserve exact file paths, function names, and error messages."#;

const UPDATE_SUMMARIZATION_PROMPT: &str = r#"The messages above are NEW conversation messages to incorporate into the existing summary in <previous-summary>.

PRESERVE existing goals/progress; ADD new progress; UPDATE Next Steps. Use the same EXACT section format (Goal, Constraints, Progress, Key Decisions, Next Steps, Critical Context).
Preserve exact file paths and error messages."#;

/// Options for one LLM compaction pass.
pub struct SummarizeCompactOptions<'a> {
    pub keep_recent_tokens: u64,
    pub stream_fn: Arc<dyn StreamFn>,
    pub model: zi_ai::Model,
    pub stream_options: zi_ai::SimpleStreamOptions,
    pub abort: Option<&'a AtomicBool>,
    /// When set, write the pre-compact transcript here and append the path to the summary.
    pub archive_path: Option<PathBuf>,
}

/// Compact via LLM summary; on failure fall back to [`structural_compact`].
pub async fn summarize_compact(
    messages: &[Message],
    options: SummarizeCompactOptions<'_>,
) -> Option<StructuralCompaction> {
    let tokens_before = estimate_messages_tokens(messages);
    let Some((systems, dropped, kept)) =
        split_for_compact(messages, options.keep_recent_tokens)
    else {
        return None;
    };
    if dropped.is_empty() {
        return None;
    }

    if let Some(path) = &options.archive_path {
        if let Err(err) = write_transcript_archive(path, messages) {
            tracing::warn!(error = %err, path = %path.display(), "transcript archive write failed");
        }
    }

    let previous_summary = extract_previous_compaction_summary(&dropped);
    let conversation = serialize_for_summary(&dropped);
    let llm_summary = match generate_llm_summary(
        &conversation,
        previous_summary.as_deref(),
        &options,
    )
    .await
    {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            tracing::warn!("LLM compact returned empty summary; falling back to structural");
            return fallback_structural(
                messages,
                options.keep_recent_tokens,
                options.archive_path.as_deref(),
            );
        }
        Err(err) => {
            tracing::warn!(error = %err, "LLM compact failed; falling back to structural");
            return fallback_structural(
                messages,
                options.keep_recent_tokens,
                options.archive_path.as_deref(),
            );
        }
    };

    let mut summary = llm_summary;
    if let Some(path) = &options.archive_path {
        summary.push_str(&format!(
            "\n\nFull transcript archive: {} (use read with offset/limit if details missing)",
            path.display()
        ));
    }

    let mut out = systems;
    out.push(Message::User(UserMessage {
        role: UserRole::User,
        content: UserContent::Text(format!("<compaction>\n{summary}\n</compaction>")),
        timestamp: wall_now_ms(),
    }));
    let pins = collect_pins_from_dropped(&dropped);
    out.extend(pins);
    let goal = latest_user_goal(messages);
    if let Some(goal_msg) = goal {
        if !summary.contains("## Goal") {
            out.push(goal_msg);
        }
    }
    let mut kept = kept;
    strip_pinned_dupes_from_kept(&mut kept, &dropped);
    out.extend(kept);
    repair_orphaned_tool_results_pub(&mut out);

    let tokens_after = estimate_messages_tokens(&out);
    Some(StructuralCompaction {
        messages: out,
        summary,
        tokens_before,
        tokens_after,
        dropped_messages: dropped.len(),
    })
}

fn fallback_structural(
    messages: &[Message],
    keep_recent_tokens: u64,
    archive_path: Option<&Path>,
) -> Option<StructuralCompaction> {
    let mut compacted = structural_compact(messages, keep_recent_tokens)?;
    if let Some(path) = archive_path {
        compacted.summary.push_str(&format!(
            " Full transcript archive: {} (use read with offset/limit if details missing).",
            path.display()
        ));
        for m in &mut compacted.messages {
            if let Message::User(u) = m {
                if let UserContent::Text(t) = &mut u.content {
                    if t.starts_with("<compaction>") {
                        *t = format!("<compaction>\n{}\n</compaction>", compacted.summary);
                        break;
                    }
                }
            }
        }
    }
    Some(compacted)
}

/// Split leading systems / dropped / kept-by-token-budget.
pub fn split_for_compact(
    messages: &[Message],
    keep_recent_tokens: u64,
) -> Option<(Vec<Message>, Vec<Message>, Vec<Message>)> {
    let mut systems = Vec::new();
    let mut rest = Vec::new();
    for message in messages {
        match message {
            Message::System(_) if rest.is_empty() => systems.push(message.clone()),
            other => rest.push(other.clone()),
        }
    }
    if rest.is_empty() {
        return None;
    }
    let (drop_count, kept) = keep_tail_by_tokens(&rest, keep_recent_tokens);
    if drop_count == 0 {
        return None;
    }
    let dropped = rest[..drop_count].to_vec();
    Some((systems, dropped, kept))
}

fn extract_previous_compaction_summary(dropped: &[Message]) -> Option<String> {
    for message in dropped.iter().rev() {
        let Message::User(u) = message else {
            continue;
        };
        let UserContent::Text(t) = &u.content else {
            continue;
        };
        if let Some(inner) = t.strip_prefix("<compaction>\n") {
            if let Some(body) = inner.strip_suffix("\n</compaction>") {
                return Some(body.to_string());
            }
            if let Some(body) = inner.strip_suffix("</compaction>") {
                return Some(body.trim().to_string());
            }
        }
    }
    None
}

/// Serialize messages for the summarizer (capped; oldest truncated).
pub fn serialize_for_summary(messages: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for message in messages {
        let line = match message {
            Message::System(_) => continue,
            Message::User(u) => {
                let text = match &u.content {
                    UserContent::Text(t) => t.clone(),
                    UserContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|b| match b {
                            zi_ai::UserContentBlock::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                format!("user: {text}")
            }
            Message::Assistant(a) => {
                let mut bits = Vec::new();
                for block in &a.content {
                    match block {
                        zi_ai::AssistantContentBlock::Text { text, .. } => bits.push(text.clone()),
                        zi_ai::AssistantContentBlock::Thinking { thinking, .. } => {
                            bits.push(format!("(thinking) {thinking}"));
                        }
                        zi_ai::AssistantContentBlock::ToolCall { name, arguments, .. } => {
                            bits.push(format!("tool_call {name}({arguments})"));
                        }
                    }
                }
                format!("assistant: {}", bits.join("\n"))
            }
            Message::ToolResult(tr) => {
                let text = tool_result_text_pub(tr);
                let clipped = if text.len() > 2_000 {
                    format!("{}…", &text[..2_000])
                } else {
                    text
                };
                format!("tool_result {}: {clipped}", tr.tool_name)
            }
        };
        parts.push(line);
    }
    let mut full = parts.join("\n\n");
    if full.len() > MAX_SUMMARY_INPUT_CHARS {
        let keep = MAX_SUMMARY_INPUT_CHARS;
        let start = full.len() - keep;
        full = format!(
            "…[older transcript truncated for summarizer]…\n\n{}",
            &full[start..]
        );
    }
    full
}

async fn generate_llm_summary(
    conversation: &str,
    previous_summary: Option<&str>,
    options: &SummarizeCompactOptions<'_>,
) -> Result<String, String> {
    let mut prompt = format!("<conversation>\n{conversation}\n</conversation>\n\n");
    if let Some(prev) = previous_summary {
        prompt.push_str(&format!(
            "<previous-summary>\n{prev}\n</previous-summary>\n\n{UPDATE_SUMMARIZATION_PROMPT}"
        ));
    } else {
        prompt.push_str(SUMMARIZATION_PROMPT);
    }

    let context = Context {
        system_prompt: Some(SUMMARIZATION_SYSTEM.to_string()),
        messages: vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text(prompt),
            timestamp: wall_now_ms(),
        })],
        tools: None,
    };
    let transcript = zi_ai::transcript::normalize_context(context);

    let mut stream_options = options.stream_options.clone();
    if stream_options.stream.max_tokens.is_none() {
        stream_options.stream.max_tokens = Some(4_096);
    }

    let response = options
        .stream_fn
        .stream(&options.model, transcript, stream_options)
        .await;

    let mut events = response.events();
    let mut final_message = None;
    while let Some(event) = events.next().await {
        if options
            .abort
            .is_some_and(|a| a.load(Ordering::Relaxed))
        {
            return Err("summarization aborted".into());
        }
        match event {
            AssistantMessageEvent::Done { message, .. } => {
                final_message = Some(message);
                break;
            }
            AssistantMessageEvent::Error { error, .. } => {
                return Err(error
                    .error_message
                    .unwrap_or_else(|| "summarization stream error".into()));
            }
            _ => {}
        }
    }
    let message = final_message.ok_or_else(|| "summarization ended without Done".to_string())?;
    if matches!(message.stop_reason, StopReason::Error | StopReason::Aborted) {
        return Err(message
            .error_message
            .unwrap_or_else(|| "summarization stop error".into()));
    }
    if matches!(message.stop_reason, StopReason::Length) {
        return Err("summarization hit max_tokens".into());
    }
    if message
        .content
        .iter()
        .any(|b| matches!(b, zi_ai::AssistantContentBlock::ToolCall { .. }))
    {
        return Err("summarization attempted a tool call".into());
    }
    let text: String = message
        .content
        .iter()
        .filter_map(|b| match b {
            zi_ai::AssistantContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text.trim().to_string())
}

fn write_transcript_archive(path: &Path, messages: &[Message]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serialize_for_summary(messages);
    std::fs::write(path, body)
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
