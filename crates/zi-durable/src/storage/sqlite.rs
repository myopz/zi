// SQLite storage adapter stub (full port deferred; use MemoryStorage or JsonlStorage).

use crate::error::{DurableError, DurableResult};
use crate::storage::Storage;
use crate::types::{
    ConversationId, ConversationRecord, EntryId, EntryRecord, Seq, StorageWrite,
};

/// Placeholder for workerd / Cloudflare DO sqlite facade.
pub struct SqliteStorage {
    _path: String,
}

impl SqliteStorage {
    /// Opens sqlite storage at `path` (not implemented in Phase 8 skeleton).
    pub fn open(path: impl Into<String>) -> DurableResult<Self> {
        let _ = path;
        Err(DurableError::Storage(
            "SqliteStorage is a stub; enable a future slice or use JsonlStorage".into(),
        ))
    }
}

impl Storage for SqliteStorage {
    fn commit(&mut self, _writes: &[StorageWrite]) -> DurableResult<Seq> {
        Err(DurableError::Storage("SqliteStorage commit not implemented".into()))
    }

    fn mint_id(&mut self) -> u64 {
        0
    }

    fn conversation(&self, _id: ConversationId) -> Option<ConversationRecord> {
        None
    }

    fn entry(&self, _id: EntryId) -> Option<(EntryRecord, Seq)> {
        None
    }

    fn entries_for_conversation(&self, _conversation_id: ConversationId) -> Vec<(EntryRecord, Seq)> {
        Vec::new()
    }
}
