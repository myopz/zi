// Conversation compaction policy and runner stub (pi harness/compaction.ts subset).

use crate::error::{DurableError, DurableResult};
use crate::types::{ConversationId, EntryId};

/// When to trigger transcript compaction.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    pub max_entries: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            max_entries: 256,
        }
    }
}

/// Result of a compaction pass (stub).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub conversation_id: ConversationId,
    pub removed_entries: Vec<EntryId>,
    pub summary_entry: Option<EntryId>,
}

/// Returns whether compaction should run for the current entry count.
pub fn should_compact(policy: &CompactionPolicy, visible_entries: usize) -> bool {
    visible_entries > policy.max_entries
}

/// Stub: records intent only; full summarization via zi-ai is Phase 8+ carry-over.
pub fn compact_conversation(
    conversation_id: ConversationId,
    _policy: &CompactionPolicy,
) -> DurableResult<CompactionResult> {
    Err(DurableError::CompactionNotImplemented(format!(
        "conversation {}",
        conversation_id.0
    )))
}
