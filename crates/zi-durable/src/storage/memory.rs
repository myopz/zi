// In-memory storage backend (pi MemoryStorage subset).

use std::collections::HashMap;

use crate::error::DurableResult;
use crate::storage::{apply_writes, validate_writes, Storage};
use crate::types::{
    ConversationId, ConversationRecord, EntryId, EntryRecord, Seq, StorageWrite,
};

#[derive(Clone, Default)]
pub(crate) struct MemoryState {
    pub conversations: HashMap<ConversationId, ConversationRecord>,
    pub conversation_ids: Vec<ConversationId>,
    pub entries: HashMap<EntryId, EntryRecord>,
    pub entry_ids: HashMap<ConversationId, Vec<EntryId>>,
    pub entry_commit_seqs: HashMap<EntryId, Seq>,
    pub next_seq: Seq,
    pub next_id: u64,
}

/// Volatile storage for tests and ephemeral harness runs.
pub struct MemoryStorage {
    state: MemoryState,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    /// Empty store starting at sequence 1.
    pub fn new() -> Self {
        Self {
            state: MemoryState {
                next_seq: Seq(1),
                next_id: 1,
                ..MemoryState::default()
            },
        }
    }
}

impl Storage for MemoryStorage {
    fn commit(&mut self, writes: &[StorageWrite]) -> DurableResult<Seq> {
        validate_writes(writes)?;
        let seq = self.state.next_seq;
        apply_writes(&mut self.state, writes, seq);
        Ok(seq)
    }

    fn mint_id(&mut self) -> u64 {
        let id = self.state.next_id;
        self.state.next_id += 1;
        id
    }

    fn conversation(&self, id: ConversationId) -> Option<ConversationRecord> {
        self.state.conversations.get(&id).cloned()
    }

    fn entry(&self, id: EntryId) -> Option<(EntryRecord, Seq)> {
        let entry = self.state.entries.get(&id)?.clone();
        if !entry.visible {
            return None;
        }
        let seq = self.state.entry_commit_seqs.get(&id).copied()?;
        Some((entry, seq))
    }

    fn entries_for_conversation(&self, conversation_id: ConversationId) -> Vec<(EntryRecord, Seq)> {
        let Some(ids) = self.state.entry_ids.get(&conversation_id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for id in ids {
            if let Some(pair) = self.entry(*id) {
                out.push(pair);
            }
        }
        out
    }
}

impl MemoryStorage {
    /// Prepared commit: entries may be invisible until `commit_prepared` runs.
    pub fn prepare_commit(&mut self, writes: Vec<StorageWrite>) -> PreparedCommit<'_> {
        PreparedCommit {
            storage: self,
            writes,
        }
    }
}

/// A batch staged with optional invisible entries until applied.
pub struct PreparedCommit<'a> {
    storage: &'a mut MemoryStorage,
    writes: Vec<StorageWrite>,
}

impl PreparedCommit<'_> {
    /// Apply the prepared batch once; idempotent on repeated calls with the same seq.
    pub fn apply(&mut self) -> DurableResult<Seq> {
        for write in &mut self.writes {
            if let StorageWrite::Entry { value } = write {
                value.visible = true;
            }
        }
        self.storage.commit(&self.writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ROOT_CONVERSATION_ID;
    use serde_json::json;

    #[test]
    fn invisible_until_prepared_apply() {
        let mut storage = MemoryStorage::new();
        storage
            .commit(&[StorageWrite::Conversation {
                value: ConversationRecord {
                    id: ROOT_CONVERSATION_ID,
                },
            }])
            .unwrap();
        let entry_id = EntryId(storage.mint_id());
        let mut prepared = storage.prepare_commit(vec![StorageWrite::Entry {
            value: EntryRecord {
                id: entry_id,
                conversation_id: ROOT_CONVERSATION_ID,
                kind: "user".into(),
                data: json!({"text": "hi"}),
                visible: false,
            },
        }]);
        assert!(storage.entry(entry_id).is_none());
        let seq = prepared.apply().unwrap();
        assert_eq!(seq, Seq(2));
        assert!(storage.entry(entry_id).is_some());
    }
}
