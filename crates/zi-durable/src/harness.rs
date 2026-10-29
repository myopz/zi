// Durable harness skeleton: session, append, prompt stub, docs/tasks stubs.

use std::sync::{Arc, Mutex};

use zi_telemetry::{TelemetryContext, TelemetryContextExt};
use serde_json::{json, Value};

use crate::compaction::{should_compact, CompactionPolicy};
use crate::error::{DurableError, DurableResult};
use crate::storage::Storage;
use crate::types::{
    ConversationId, ConversationRecord, DocumentStub, EntryId, EntryRecord, Seq, StorageWrite,
    TaskStub, ROOT_CONVERSATION_ID,
};

/// Options for [`DurableHarness::new`].
pub struct DurableHarnessOptions {
    pub telemetry: Arc<dyn TelemetryContext + Send + Sync>,
    pub compaction: CompactionPolicy,
    #[cfg(feature = "codemode")]
    pub codemode_enabled: bool,
}

impl Default for DurableHarnessOptions {
    fn default() -> Self {
        Self {
            telemetry: Arc::new(zi_telemetry::NoopTelemetryContext),
            compaction: CompactionPolicy::default(),
            #[cfg(feature = "codemode")]
            codemode_enabled: false,
        }
    }
}

/// Outcome of a prompt round (generation deferred to later slices).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptResult {
    pub user_entry: EntryId,
    pub assistant_entry: Option<EntryId>,
    pub commit_seq: Seq,
}

/// Session host aligned with pi `Harness` conveniences (subset).
pub struct DurableHarness {
    storage: Arc<Mutex<dyn Storage>>,
    conversation_id: ConversationId,
    options: DurableHarnessOptions,
}

impl DurableHarness {
    /// Attach to storage and ensure a root conversation exists.
    pub async fn new(
        storage: Arc<Mutex<dyn Storage>>,
        options: DurableHarnessOptions,
    ) -> DurableResult<Self> {
        let mut guard = storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
        if guard.conversation(ROOT_CONVERSATION_ID).is_none() {
            guard.commit(&[StorageWrite::Conversation {
                value: ConversationRecord {
                    id: ROOT_CONVERSATION_ID,
                },
            }])?;
        }
        drop(guard);
        Ok(Self {
            storage,
            conversation_id: ROOT_CONVERSATION_ID,
            options,
        })
    }

    /// Create an independent conversation and select it as active.
    pub async fn create_session(&mut self) -> DurableResult<ConversationId> {
        let id = {
            let mut guard = self.storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
            let id = ConversationId(guard.mint_id());
            guard.commit(&[StorageWrite::Conversation {
                value: ConversationRecord { id },
            }])?;
            id
        };
        self.conversation_id = id;
        Ok(id)
    }

    pub fn conversation_id(&self) -> ConversationId {
        self.conversation_id
    }

    /// Append a commit-visible entry to the active conversation.
    pub async fn append(&mut self, kind: impl Into<String>, data: Value) -> DurableResult<EntryId> {
        self.append_entry(kind, data, true).await
    }

    /// Append with explicit visibility (false = prepared/invisible until a later commit marks visible).
    pub async fn append_entry(
        &mut self,
        kind: impl Into<String>,
        data: Value,
        visible: bool,
    ) -> DurableResult<EntryId> {
        let mut guard = self.storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
        let id = EntryId(guard.mint_id());
        let seq = guard.commit(&[StorageWrite::Entry {
            value: EntryRecord {
                id,
                conversation_id: self.conversation_id,
                kind: kind.into(),
                data,
                visible,
            },
        }])?;
        drop(guard);
        let _ = self
            .options
            .telemetry
            .start_span(
                zi_telemetry::SpanOptions {
                    name: "durable.append".into(),
                    attributes: None,
                    links: None,
                },
                move |_span| async move { seq },
            )
            .await;
        Ok(id)
    }

    /// Record a user message; model generation is not wired in this skeleton.
    pub async fn prompt(&mut self, text: &str) -> DurableResult<PromptResult> {
        let user_entry = self
            .append(String::from("user"), json!({ "text": text }))
            .await?;
        let commit_seq = {
            let guard = self.storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
            guard
                .entry(user_entry)
                .map(|(_, seq)| seq)
                .unwrap_or(Seq(0))
        };
        self.maybe_schedule_compaction().await?;
        Ok(PromptResult {
            user_entry,
            assistant_entry: None,
            commit_seq,
        })
    }

    async fn maybe_schedule_compaction(&self) -> DurableResult<()> {
        let count = {
            let guard = self.storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
            guard.entries_for_conversation(self.conversation_id).len()
        };
        if should_compact(&self.options.compaction, count) {
            let _policy = &self.options.compaction;
            // Full compaction task graph is carry-over; policy hook exists for callers.
        }
        Ok(())
    }

    /// List committed entries for the active conversation.
    pub fn entries(&self) -> DurableResult<Vec<(EntryRecord, Seq)>> {
        let guard = self.storage.lock().map_err(|_| DurableError::Storage("lock poisoned".into()))?;
        Ok(guard.entries_for_conversation(self.conversation_id))
    }

    /// Document registry stub (Chord document store integration deferred).
    pub fn list_docs(&self) -> Vec<DocumentStub> {
        Vec::new()
    }

    /// Task scheduler stub (builtin tasks deferred).
    pub fn list_tasks(&self) -> Vec<TaskStub> {
        Vec::new()
    }
}
