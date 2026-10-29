//! Durable harness for **zi**: session storage, harness skeleton, compaction stub.
//!
//! Aligned with pi `packages/durable` (Phase 8 skeleton).

pub mod compaction;
pub mod env;
pub mod error;
pub mod harness;
pub mod storage;
pub mod types;

pub use compaction::{
    compact_conversation, should_compact, CompactionPolicy, CompactionResult,
};
pub use env::{
    EnvResult, ExecutionEnv, FileError, FileKind, FileListEntry, FileListing, ShellExecResult,
};
pub use error::{DurableError, DurableResult};
pub use harness::{DurableHarness, DurableHarnessOptions, PromptResult};
pub use storage::{JsonlStorage, MemoryStorage, Storage};
#[cfg(feature = "sqlite")]
pub use storage::SqliteStorage;
pub use types::{
    ConversationId, ConversationRecord, DocumentId, DocumentStub, EntryId, EntryRecord, Seq,
    StorageWrite, TaskId, TaskStub, ROOT_CONVERSATION_ID,
};
