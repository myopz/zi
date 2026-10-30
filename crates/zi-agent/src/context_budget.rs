//! Context budget helpers: token estimates, tool-result folding, structural compaction.

use std::collections::HashMap;

use zi_ai::{
    Message, TextContent, TextContentKind, ToolResultContentBlock, ToolResultMessage,
    ToolResultRole, UserContent, UserMessage, UserRole,
};

/// Token budget for recent tool results kept in full for the LLM request.
pub const DEFAULT_KEEP_RECENT_TOOL_RESULT_TOKENS: u64 = 8_000;

/// Default fraction of the model context window that triggers structural compaction.
/// Matches Cursor's ~90% auto-summarization trigger on small/medium windows.
pub const DEFAULT_COMPACT_THRESHOLD_RATIO: f32 = 0.90;

/// Absolute token ceiling for auto-compact. Large advertised windows (e.g. 1M)
/// would otherwise never hit the ratio threshold while Σ input still explodes.
pub const DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT: u64 = 48_000;

/// Rough chars-per-token for budget estimates (conservative for code-heavy tool dumps).
pub const CHARS_PER_TOKEN: usize = 4;

/// Token budget for the trailing window kept after structural / LLM compaction.
pub const DEFAULT_KEEP_RECENT_MESSAGE_TOKENS: u64 = 12_000;

/// Max chars retained for a pinned user goal.
const PINNED_GOAL_MAX_CHARS: usize = 2_000;

/// Estimate tokens for one message (content text only; ignores images).
pub fn estimate_message_tokens(message: &Message) -> u64 {
    let text = message_plain_text(message);
    ((text.len() + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN) as u64
}

/// Estimate tokens for a transcript.
pub fn estimate_messages_tokens(messages: &[Message]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

fn message_plain_text(message: &Message) -> String {
    match message {
        Message::System(sys) => match &sys.content {
            zi_ai::SystemContent::Text(t) => t.clone(),
            zi_ai::SystemContent::Blocks(blocks) => {
                blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n")
            }
        },
        Message::User(user) => match &user.content {
            UserContent::Text(t) => t.clone(),
            UserContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    zi_ai::UserContentBlock::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        },
        Message::Assistant(asst) => {
            let mut parts = Vec::new();
            for block in &asst.content {
                match block {
                    zi_ai::AssistantContentBlock::Text { text, .. } => parts.push(text.clone()),
                    zi_ai::AssistantContentBlock::Thinking { thinking, .. } => {
                        parts.push(thinking.clone())
                    }
                    zi_ai::AssistantContentBlock::ToolCall {
                        name, arguments, ..
                    } => parts.push(format!("{name}{arguments}")),
                }
            }
            parts.join("\n")
        }
        Message::ToolResult(tr) => tool_result_text(tr),
    }
}

fn tool_result_text(tr: &ToolResultMessage) -> String {
    tr.content
        .iter()
        .filter_map(|b| match b {
            ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
            ToolResultContentBlock::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Public wrapper for summarizer serialization.
pub fn tool_result_text_pub(tr: &ToolResultMessage) -> String {
    tool_result_text(tr)
}

/// Keep a trailing window of `rest` whose estimated tokens fit `budget` (at least one message).
/// Returns `(drop_count, kept)`.
pub fn keep_tail_by_tokens(rest: &[Message], budget: u64) -> (usize, Vec<Message>) {
    if rest.is_empty() {
        return (0, Vec::new());
    }
    let budget = budget.max(1);
    let mut kept_rev: Vec<Message> = Vec::new();
    let mut tokens = 0u64;
    for message in rest.iter().rev() {
        let t = estimate_message_tokens(message);
        if !kept_rev.is_empty() && tokens.saturating_add(t) > budget {
            break;
        }
        tokens = tokens.saturating_add(t);
        kept_rev.push(message.clone());
    }
    kept_rev.reverse();
    let drop_count = rest.len().saturating_sub(kept_rev.len());
    (drop_count, kept_rev)
}

/// Fold older tool results into short placeholders for the LLM request.
///
/// Keeps a trailing set of tool results whose combined tokens fit `keep_recent_tokens`.
/// Earlier large ones become a one-line stub. Pinned tools (edit/write/explore) never fold.
pub fn fold_old_tool_results(
    messages: &[Message],
    keep_recent_tokens: u64,
) -> (Vec<Message>, usize) {
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| matches!(m, Message::ToolResult(_)).then_some(i))
        .collect();

    if tool_indices.is_empty() {
        return (messages.to_vec(), 0);
    }

    let budget = keep_recent_tokens.max(1);
    let mut keep_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut tokens = 0u64;
    // Always retain pinned tool results.
    for &idx in &tool_indices {
        if let Message::ToolResult(tr) = &messages[idx] {
            if is_pinned_tool_result(&tr.tool_name) {
                keep_set.insert(idx);
            }
        }
    }
    for &idx in tool_indices.iter().rev() {
        if keep_set.contains(&idx) {
            let t = estimate_message_tokens(&messages[idx]);
            tokens = tokens.saturating_add(t);
            continue;
        }
        let t = estimate_message_tokens(&messages[idx]);
        if tokens > 0 && tokens.saturating_add(t) > budget {
            break;
        }
        tokens = tokens.saturating_add(t);
        keep_set.insert(idx);
    }
    if let Some(&last) = tool_indices.last() {
        keep_set.insert(last);
    }

    let mut folded = 0usize;
    let out = messages
        .iter()
        .enumerate()
        .map(|(i, message)| {
            if keep_set.contains(&i) {
                return message.clone();
            }
            let Message::ToolResult(tr) = message else {
                return message.clone();
            };
            if is_pinned_tool_result(&tr.tool_name) {
                return message.clone();
            }
            let bytes = tool_result_text(tr).len();
            if bytes <= 160 {
                return message.clone();
            }
            folded += 1;
            Message::ToolResult(ToolResultMessage {
                role: ToolResultRole::ToolResult,
                tool_call_id: tr.tool_call_id.clone(),
                tool_name: tr.tool_name.clone(),
                content: vec![ToolResultContentBlock::Text(TextContent {
                    kind: TextContentKind::Text,
                    text: format!(
                        "[tool `{}` result omitted: {bytes} bytes; kept recent tool results only]",
                        tr.tool_name
                    ),
                    text_signature: None,
                })],
                details: Some(serde_json::json!({
                    "folded": true,
                    "originalBytes": bytes,
                })),
                is_error: tr.is_error,
                timestamp: tr.timestamp,
            })
        })
        .collect();

    (out, folded)
}

/// Result of a structural compaction pass.
#[derive(Debug, Clone)]
pub struct StructuralCompaction {
    pub messages: Vec<Message>,
    pub summary: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub dropped_messages: usize,
}

/// Drop older transcript messages, keep leading system messages and a trailing
/// token budget window, and insert a user summary describing what was omitted.
///
/// `keep_recent_tokens` is the estimated-token budget for the kept tail.
pub fn structural_compact(
    messages: &[Message],
    keep_recent_tokens: u64,
) -> Option<StructuralCompaction> {
    let tokens_before = estimate_messages_tokens(messages);

    let mut systems = Vec::new();
    let mut rest = Vec::new();
    for message in messages {
        match message {
            Message::System(_) if rest.is_empty() => systems.push(message.clone()),
            other => rest.push(other.clone()),
        }
    }

    let (drop_count, mut kept) = keep_tail_by_tokens(&rest, keep_recent_tokens);
    if drop_count == 0 {
        return None;
    }
    let dropped = &rest[..drop_count];

    let pinned_notes = collect_pins_from_dropped(dropped);
    strip_pinned_dupes_from_kept(&mut kept, dropped);

    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut dropped_bytes = 0usize;
    let mut file_ops: Vec<String> = Vec::new();
    let mut seen_file_op_paths: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut dropped_explore = false;
    for message in dropped {
        match message {
            Message::ToolResult(tr) => {
                *tool_counts.entry(tr.tool_name.clone()).or_default() += 1;
                if tr.tool_name == "explore" {
                    dropped_explore = true;
                }
                let text = tool_result_text(tr);
                dropped_bytes += text.len();
                if is_file_mutation_tool(&tr.tool_name) && !tr.is_error {
                    let path = mutation_path(&text);
                    if path.is_empty() || !seen_file_op_paths.insert(path.clone()) {
                        continue;
                    }
                    file_ops.push(format!("{} {path}", tr.tool_name));
                }
            }
            Message::Assistant(a) => {
                dropped_bytes += estimate_message_tokens(&Message::Assistant(a.clone())) as usize
                    * CHARS_PER_TOKEN;
            }
            Message::User(u) => {
                dropped_bytes += estimate_message_tokens(&Message::User(u.clone())) as usize
                    * CHARS_PER_TOKEN;
            }
            Message::System(_) => {}
        }
    }

    let mut tool_parts: Vec<String> = tool_counts
        .into_iter()
        .map(|(name, n)| format!("{name}×{n}"))
        .collect();
    tool_parts.sort();
    let tools_line = if tool_parts.is_empty() {
        "none".to_string()
    } else {
        tool_parts.join(", ")
    };

    let file_ops_line = if file_ops.is_empty() {
        String::new()
    } else {
        // Cap so the compaction marker itself stays small.
        let shown: Vec<_> = file_ops.into_iter().take(12).collect();
        format!(
            " Already completed file ops (do not repeat): {}.",
            shown.join(" | ")
        )
    };

    let explore_line = if dropped_explore {
        " Prior explore summaries covered codebase search — do not re-scan the whole tree with bash."
            .to_string()
    } else {
        String::new()
    };

    let goal_msg = latest_user_goal(messages);
    let goal_excerpt = goal_msg.as_ref().and_then(|m| match m {
        Message::User(u) => match &u.content {
            UserContent::Text(t) => {
                let body = t
                    .strip_prefix("<pinned-goal>\n")
                    .and_then(|s| s.strip_suffix("\n</pinned-goal>"))
                    .unwrap_or(t);
                let short: String = body.chars().take(200).collect();
                Some(short)
            }
            _ => None,
        },
        _ => None,
    });
    let goal_line = goal_excerpt
        .as_ref()
        .map(|g| format!(" User goal (pinned): {g}"))
        .unwrap_or_default();

    let summary = format!(
        "Context compaction: omitted {drop_count} earlier messages (~{dropped_bytes} bytes). \
         Tool results in omitted span: {tools_line}.{file_ops_line}{explore_line}{goal_line} \
         Continue from the remaining transcript; do not redo listed file ops."
    );

    let mut out = systems;
    out.push(Message::User(UserMessage {
        role: UserRole::User,
        content: UserContent::Text(format!("<compaction>\n{summary}\n</compaction>")),
        timestamp: wall_now_ms(),
    }));
    out.extend(pinned_notes);
    if let Some(goal) = goal_msg {
        if !kept_contains_goal(&kept, &goal) {
            out.push(goal);
        }
    }
    out.extend(kept);
    // Keep window may start mid-turn (toolResult without its assistant tool_calls).
    repair_orphaned_tool_results(&mut out);

    let tokens_after = estimate_messages_tokens(&out);
    Some(StructuralCompaction {
        messages: out,
        summary,
        tokens_before,
        tokens_after,
        dropped_messages: drop_count,
    })
}

/// Effective auto-compact trigger: `min(window × ratio, absolute_limit)`.
///
/// Absolute limit protects large-window models; ratio keeps small windows
/// from compacting too early relative to their hard capacity.
pub fn compact_token_limit(
    context_window: u32,
    threshold_ratio: f32,
    absolute_limit: u64,
) -> u64 {
    let ratio = threshold_ratio.clamp(0.1, 0.95);
    let ratio_limit = if context_window == 0 {
        u64::MAX
    } else {
        (context_window as f32 * ratio) as u64
    };
    if absolute_limit == 0 {
        ratio_limit
    } else {
        ratio_limit.min(absolute_limit)
    }
}

/// Whether estimated tokens exceed the effective compaction limit.
pub fn should_compact(
    estimated_tokens: u64,
    context_window: u32,
    threshold_ratio: f32,
    absolute_limit: u64,
) -> bool {
    let limit = compact_token_limit(context_window, threshold_ratio, absolute_limit);
    limit > 0 && estimated_tokens >= limit
}

fn is_file_mutation_tool(name: &str) -> bool {
    matches!(name, "write" | "edit")
}

/// Tool results that must stay intact under fold (mutations + explore summaries).
fn is_pinned_tool_result(name: &str) -> bool {
    is_file_mutation_tool(name) || name == "explore"
}

/// Rewrite a tool success as a user note so providers never see an orphan `role:tool`.
fn pinned_tool_as_user(tr: &ToolResultMessage) -> Message {
    let text = tool_result_text(tr);
    Message::User(UserMessage {
        role: UserRole::User,
        content: UserContent::Text(format!(
            "<pinned-{}>\n{text}\n</pinned-{}>",
            tr.tool_name, tr.tool_name
        )),
        timestamp: tr.timestamp,
    })
}

fn is_synthetic_user_text(text: &str) -> bool {
    text.contains("<compaction>") || text.contains("<pinned-") || text.contains("<session-context>")
}

/// Latest real user goal as a `<pinned-goal>` message, if any.
pub fn latest_user_goal(messages: &[Message]) -> Option<Message> {
    for message in messages.iter().rev() {
        let Message::User(u) = message else {
            continue;
        };
        let text = match &u.content {
            UserContent::Text(t) => t.as_str(),
            UserContent::Blocks(blocks) => {
                // Prefer first text block.
                let joined = blocks
                    .iter()
                    .filter_map(|b| match b {
                        zi_ai::UserContentBlock::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if joined.is_empty() {
                    continue;
                }
                // Can't return ref to local — copy below.
                let body = truncate_goal(&joined);
                return Some(Message::User(UserMessage {
                    role: UserRole::User,
                    content: UserContent::Text(format!("<pinned-goal>\n{body}\n</pinned-goal>")),
                    timestamp: u.timestamp,
                }));
            }
        };
        if text.is_empty() || is_synthetic_user_text(text) {
            continue;
        }
        let body = truncate_goal(text);
        return Some(Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text(format!("<pinned-goal>\n{body}\n</pinned-goal>")),
            timestamp: u.timestamp,
        }));
    }
    None
}

fn truncate_goal(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= PINNED_GOAL_MAX_CHARS {
        return trimmed.to_string();
    }
    let short: String = trimmed.chars().take(PINNED_GOAL_MAX_CHARS).collect();
    format!("{short}\n…[goal truncated]")
}

fn kept_contains_goal(kept: &[Message], goal: &Message) -> bool {
    let Message::User(goal_u) = goal else {
        return false;
    };
    let goal_inner = match &goal_u.content {
        UserContent::Text(t) => t
            .strip_prefix("<pinned-goal>\n")
            .and_then(|s| s.strip_suffix("\n</pinned-goal>"))
            .unwrap_or(t.as_str()),
        _ => return false,
    };
    kept.iter().any(|m| match m {
        Message::User(u) => match &u.content {
            UserContent::Text(t) => {
                if is_synthetic_user_text(t) && !t.contains("<pinned-goal>") {
                    return false;
                }
                t.contains(goal_inner) || goal_inner.contains(t.trim())
            }
            _ => false,
        },
        _ => false,
    })
}

/// Pin successful write/edit/explore from the dropped span as user notes.
pub fn collect_pins_from_dropped(dropped: &[Message]) -> Vec<Message> {
    let mut pinned_notes: Vec<Message> = Vec::new();
    let mut seen_mutation_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut pinned_explore = false;
    for message in dropped {
        let Message::ToolResult(tr) = message else {
            continue;
        };
        if tr.is_error {
            continue;
        }
        if tr.tool_name == "explore" {
            if pinned_explore {
                continue;
            }
            pinned_explore = true;
            pinned_notes.push(pinned_tool_as_user(tr));
            continue;
        }
        if !is_file_mutation_tool(&tr.tool_name) {
            continue;
        }
        let text = tool_result_text(tr);
        let key = mutation_dedupe_key(&tr.tool_name, &text);
        if !seen_mutation_keys.insert(key) {
            continue;
        }
        pinned_notes.push(pinned_tool_as_user(tr));
    }
    pinned_notes
}

/// Remove keep-window mutation/explore toolResults already represented in pins.
pub fn strip_pinned_dupes_from_kept(kept: &mut Vec<Message>, dropped: &[Message]) {
    let mut seen_mutation_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut pinned_explore = false;
    for message in dropped {
        let Message::ToolResult(tr) = message else {
            continue;
        };
        if tr.is_error {
            continue;
        }
        if tr.tool_name == "explore" {
            pinned_explore = true;
            continue;
        }
        if is_file_mutation_tool(&tr.tool_name) {
            let text = tool_result_text(tr);
            seen_mutation_keys.insert(mutation_dedupe_key(&tr.tool_name, &text));
        }
    }
    kept.retain(|message| {
        let Message::ToolResult(tr) = message else {
            return true;
        };
        if tr.is_error {
            return true;
        }
        if tr.tool_name == "explore" {
            return !pinned_explore;
        }
        if !is_file_mutation_tool(&tr.tool_name) {
            return true;
        }
        let text = tool_result_text(tr);
        let key = mutation_dedupe_key(&tr.tool_name, &text);
        !seen_mutation_keys.contains(&key)
    });
}

/// Convert toolResults whose `tool_call_id` is not answered by a prior assistant
/// `toolCall` in the same list into user notes (OpenAI chat requires pairing).
pub fn repair_orphaned_tool_results_pub(messages: &mut Vec<Message>) {
    repair_orphaned_tool_results(messages);
}

fn repair_orphaned_tool_results(messages: &mut Vec<Message>) {
    let mut open_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut i = 0;
    while i < messages.len() {
        match &messages[i] {
            Message::Assistant(a) => {
                for block in &a.content {
                    if let zi_ai::AssistantContentBlock::ToolCall { id, .. } = block {
                        open_ids.insert(id.clone());
                    }
                }
                i += 1;
            }
            Message::ToolResult(tr) => {
                if open_ids.remove(&tr.tool_call_id) {
                    i += 1;
                } else {
                    let note = pinned_tool_as_user(tr);
                    messages[i] = note;
                    i += 1;
                }
            }
            Message::User(_) | Message::System(_) => {
                // New user/system turn: previous tool_calls without results are abandoned.
                open_ids.clear();
                i += 1;
            }
        }
    }
}

/// Extract path from `Updated PATH (…)` / `Wrote PATH (…)` success lines.
fn mutation_path(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let rest = line
        .strip_prefix("Updated ")
        .or_else(|| line.strip_prefix("Wrote "))
        .unwrap_or(line);
    rest.rsplit_once(" (")
        .map(|(path, _)| path.trim().to_string())
        .unwrap_or_else(|| rest.trim().to_string())
}

fn mutation_dedupe_key(tool_name: &str, text: &str) -> String {
    let path = mutation_path(text);
    if path.is_empty() {
        format!("{tool_name}:{text}")
    } else {
        // Path-level dedupe: "Updated foo (2 edit(s))" and "(6 edit(s))" collapse.
        format!("{tool_name}:{path}")
    }
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zi_ai::{AssistantContentBlock, AssistantMessage, AssistantRole, StopReason};

    fn tool_result(name: &str, id: &str, text: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            role: ToolResultRole::ToolResult,
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: vec![ToolResultContentBlock::Text(TextContent {
                kind: TextContentKind::Text,
                text: text.into(),
                text_signature: None,
            })],
            details: None,
            is_error: false,
            timestamp: 1,
        })
    }

    #[test]
    fn compact_limit_caps_large_windows_keeps_ratio_on_small() {
        assert!((DEFAULT_COMPACT_THRESHOLD_RATIO - 0.90).abs() < f32::EPSILON);
        assert_eq!(DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT, 48_000);
        // 1M window: absolute 48k wins over 900k ratio limit.
        assert_eq!(
            compact_token_limit(1_000_000, 0.90, DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT),
            DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT
        );
        assert!(!should_compact(
            47_999,
            1_000_000,
            0.90,
            DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT
        ));
        assert!(should_compact(
            48_000,
            1_000_000,
            0.90,
            DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT
        ));
        // 16k window: ratio 90% = 14_400, below absolute.
        assert_eq!(compact_token_limit(16_000, 0.90, DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT), 14_400);
        assert!(should_compact(
            14_400,
            16_000,
            0.90,
            DEFAULT_COMPACT_ABSOLUTE_TOKEN_LIMIT
        ));
    }

    #[test]
    fn folds_old_tool_results_keeps_recent() {
        let big = "x".repeat(1000);
        let messages = vec![
            tool_result("bash", "1", &big),
            tool_result("bash", "2", &big),
            tool_result("bash", "3", &big),
            tool_result("read", "4", "short recent"),
        ];
        let (folded, n) = fold_old_tool_results(&messages, 80);
        assert_eq!(n, 3);
        assert!(matches!(&folded[3], Message::ToolResult(tr) if tr.content.iter().any(|b| match b {
            ToolResultContentBlock::Text(t) => t.text.contains("short recent"),
            _ => false,
        })));
        assert!(matches!(&folded[0], Message::ToolResult(tr) if {
            let text = tool_result_text(tr);
            text.contains("omitted") && text.len() < 200
        }));
    }

    #[test]
    fn never_folds_edit_or_write_even_when_old() {
        let big_bash = "y".repeat(2000);
        let edit_line = "Updated /tmp/ARCHITECTURE.md (1 edit(s))";
        let write_line = "Wrote /tmp/ARCHITECTURE.md (11920 bytes)";
        let messages = vec![
            tool_result("edit", "e1", edit_line),
            tool_result("write", "w1", write_line),
            tool_result("bash", "b1", &big_bash),
            tool_result("bash", "b2", &big_bash),
            tool_result("bash", "b3", &big_bash),
            tool_result("read", "r1", "recent"),
        ];
        // keep only the last tool result in full; edit/write are older but must stay.
        let (folded, n) = fold_old_tool_results(&messages, 80);
        assert!(n >= 1, "bash dumps should fold");
        assert!(
            matches!(&folded[0], Message::ToolResult(tr) if tool_result_text(tr) == edit_line),
            "edit success must not be folded"
        );
        assert!(
            matches!(&folded[1], Message::ToolResult(tr) if tool_result_text(tr) == write_line),
            "write success must not be folded"
        );
        assert!(matches!(&folded[2], Message::ToolResult(tr) if {
            tool_result_text(tr).contains("omitted")
        }));
    }

    #[test]
    fn never_folds_explore_summary_even_when_old() {
        let explore_text = "Explore summary (subagent tool results=4, max_turns=12):\n\n- packs use chord\nKey paths: packages/chord/src";
        let big = "z".repeat(2000);
        let messages = vec![
            tool_result("explore", "x1", explore_text),
            tool_result("bash", "b1", &big),
            tool_result("bash", "b2", &big),
            tool_result("read", "r1", "recent"),
        ];
        let (folded, n) = fold_old_tool_results(&messages, 80);
        assert!(n >= 1);
        assert!(
            matches!(&folded[0], Message::ToolResult(tr) if {
                tr.tool_name == "explore" && tool_result_text(tr) == explore_text
            }),
            "explore summary must not be folded"
        );
    }

    #[test]
    fn structural_compact_pins_user_goal_when_dropped() {
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("implement feature X in packages/foo".into()),
            timestamp: 1,
        })];
        for i in 0..30 {
            messages.push(tool_result("bash", &format!("b{i}"), &"z".repeat(400)));
        }
        let compact = structural_compact(&messages, 300).expect("compact");
        let pinned = compact.messages.iter().any(|m| match m {
            Message::User(u) => match &u.content {
                UserContent::Text(t) => {
                    t.contains("<pinned-goal>") && t.contains("implement feature X")
                }
                _ => false,
            },
            _ => false,
        });
        assert!(pinned, "dropped user goal must be pinned");
        assert!(
            compact.summary.contains("User goal (pinned):"),
            "summary should mention pinned goal"
        );
    }

    #[test]
    fn structural_compact_inserts_summary() {
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("start".into()),
            timestamp: 1,
        })];
        for i in 0..20 {
            messages.push(tool_result("bash", &format!("c{i}"), &"y".repeat(500)));
            messages.push(Message::Assistant(AssistantMessage {
                role: AssistantRole::Assistant,
                content: vec![AssistantContentBlock::Text {
                    text: format!("ok {i}"),
                    text_signature: None,
                }],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                response_model: None,
                response_id: None,
                usage: Default::default(),
                stop_reason: StopReason::Stop,
                error_message: None,
                raw_stop_reason: None,
                timestamp: i,
                duration_ms: None,
            }));
        }
        let compact = structural_compact(&messages, 800).expect("compact");
        assert!(compact.dropped_messages > 0);
        assert!(compact.tokens_after < compact.tokens_before);
        assert!(matches!(&compact.messages[0], Message::User(u) if match &u.content {
            UserContent::Text(t) => t.contains("compaction"),
            _ => false,
        }));
    }

    #[test]
    fn structural_compact_pins_edit_results_and_mentions_them() {
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("write docs".into()),
            timestamp: 1,
        })];
        messages.push(tool_result(
            "edit",
            "e1",
            "Updated /tmp/ARCHITECTURE.md (1 edit(s))",
        ));
        for i in 0..20 {
            messages.push(tool_result("bash", &format!("b{i}"), &"z".repeat(400)));
        }
        let compact = structural_compact(&messages, 800).expect("compact");
        assert!(
            compact.summary.contains("ARCHITECTURE.md"),
            "summary should list completed file ops: {}",
            compact.summary
        );
        assert!(
            compact.summary.contains("do not redo"),
            "summary should tell the model not to repeat file ops"
        );
        let edit_pinned = compact.messages.iter().any(|m| match m {
            Message::User(u) => match &u.content {
                UserContent::Text(t) => {
                    t.contains("<pinned-edit>") && t.contains("ARCHITECTURE.md")
                }
                _ => false,
            },
            _ => false,
        });
        assert!(edit_pinned, "edit success should stay pinned as user note");
        assert!(
            !compact
                .messages
                .iter()
                .any(|m| matches!(m, Message::ToolResult(_))),
            "compact output must not leave orphan toolResults for OpenAI"
        );
    }

    /// Regression: identical ARCHITECTURE.md edit results must not remain as N copies
    /// after compaction (the web UI bug of repeated RESULT cards).
    #[test]
    fn structural_compact_dedupes_identical_architecture_edits() {
        let edit_line = "Updated /Users/zhangjianlong/workspace/forkmess/pi/ARCHITECTURE.md (1 edit(s))";
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("analyze and write ARCHITECTURE.md".into()),
            timestamp: 1,
        })];
        // 11 identical edit successes (matches the observed web session).
        for i in 0..11 {
            messages.push(tool_result("edit", &format!("e{i}"), edit_line));
            messages.push(tool_result(
                "bash",
                &format!("b{i}"),
                &format!("noise-{i}-{}", "n".repeat(300)),
            ));
        }
        let compact = structural_compact(&messages, 200).expect("compact");
        let edit_count = compact
            .messages
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
            "identical edit results must dedupe to one pinned copy, got {edit_count}"
        );
        assert!(
            compact.summary.contains("ARCHITECTURE.md"),
            "compaction summary must retain the completed file op"
        );
    }

    /// Regression: polish loops that change "N edit(s)" still collapse by path.
    #[test]
    fn structural_compact_dedupes_architecture_edits_by_path() {
        let path = "/Users/zhangjianlong/workspace/forkmess/pi/ARCHITECTURE.md";
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("write ARCHITECTURE.md".into()),
            timestamp: 1,
        })];
        for (i, n) in [12, 7, 4, 2, 2, 6, 6, 6, 2, 5].into_iter().enumerate() {
            messages.push(tool_result(
                "edit",
                &format!("e{i}"),
                &format!("Updated {path} ({n} edit(s))"),
            ));
            messages.push(tool_result(
                "bash",
                &format!("b{i}"),
                &format!("noise-{i}-{}", "n".repeat(300)),
            ));
        }
        let compact = structural_compact(&messages, 200).expect("compact");
        let edit_count = compact
            .messages
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
            "path-equivalent edit results must collapse to one pin, got {edit_count}"
        );
        assert!(
            compact.summary.contains("ARCHITECTURE.md"),
            "summary should list the path once: {}",
            compact.summary
        );
    }

    #[test]
    fn structural_compact_repairs_orphan_tool_results_in_keep_window() {
        use zi_ai::{AssistantContentBlock, AssistantMessage, AssistantRole, StopReason};
        let mut messages = vec![Message::User(UserMessage {
            role: UserRole::User,
            content: UserContent::Text("start".into()),
            timestamp: 1,
        })];
        for i in 0..10 {
            messages.push(tool_result("bash", &format!("old{i}"), &"n".repeat(400)));
        }
        // Keep window starts with a toolResult whose assistant tool_call was dropped.
        messages.push(tool_result("bash", "orphan", "orphan bash output"));
        messages.push(Message::Assistant(AssistantMessage {
            role: AssistantRole::Assistant,
            content: vec![AssistantContentBlock::Text {
                text: "done".into(),
                text_signature: None,
            }],
            api: "test".into(),
            provider: "test".into(),
            model: "test".into(),
            response_model: None,
            response_id: None,
            usage: Default::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            raw_stop_reason: None,
            timestamp: 99,
            duration_ms: None,
        }));
        let compact = structural_compact(&messages, 400).expect("compact");
        let orphan_as_tool = compact.messages.iter().any(|m| {
            matches!(m, Message::ToolResult(tr) if tr.tool_call_id == "orphan")
        });
        assert!(
            !orphan_as_tool,
            "orphan toolResult must be rewritten before LLM"
        );
        let orphan_as_user = compact.messages.iter().any(|m| match m {
            Message::User(u) => match &u.content {
                UserContent::Text(t) => t.contains("orphan bash output"),
                _ => false,
            },
            _ => false,
        });
        assert!(orphan_as_user, "orphan should become a user pin note");
    }
}
