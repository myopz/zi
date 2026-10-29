// Storage backends for durable session state.

mod jsonl;
mod memory;

#[cfg(feature = "sqlite")]
mod sqlite;

pub use jsonl::JsonlStorage;
pub use memory::MemoryStorage;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;

use crate::error::{DurableError, DurableResult};
use crate::types::{
    ConversationId, ConversationRecord, EntryId, EntryRecord, Seq, StorageWrite,
};

/// Atomic durable log: commit batches and read committed records.
pub trait Storage: Send + Sync {
    /// Persist one batch and return its sequence number.
    fn commit(&mut self, writes: &[StorageWrite]) -> DurableResult<Seq>;

    /// Next id greater than every id minted or stored since open.
    fn mint_id(&mut self) -> u64;

    fn conversation(&self, id: ConversationId) -> Option<ConversationRecord>;

    fn entry(&self, id: EntryId) -> Option<(EntryRecord, Seq)>;

    fn entries_for_conversation(&self, conversation_id: ConversationId) -> Vec<(EntryRecord, Seq)>;
}

/// In-memory adoption hook run synchronously after a successful commit (pi `adopt` subset).
pub type AdoptFn = dyn FnMut(Seq) + Send;

pub(crate) fn apply_writes(state: &mut memory::MemoryState, writes: &[StorageWrite], seq: Seq) {
    for write in writes {
        match write {
            StorageWrite::Conversation { value } => {
                state.conversations.insert(value.id, value.clone());
                if !state.conversation_ids.contains(&value.id) {
                    state.conversation_ids.push(value.id);
                    state.conversation_ids.sort();
                }
            }
            StorageWrite::Entry { value } => {
                state.entries.insert(value.id, value.clone());
                state
                    .entry_ids
                    .entry(value.conversation_id)
                    .or_default()
                    .push(value.id);
                state.entry_commit_seqs.insert(value.id, seq);
            }
        }
    }
    state.next_seq = Seq(state.next_seq.0 + 1);
}

pub(crate) fn validate_writes(writes: &[StorageWrite]) -> DurableResult<()> {
    if writes.is_empty() {
        return Err(DurableError::Storage("commit batch is empty".into()));
    }
    Ok(())
}
