// Shared store (design §10). Slice 6a: tables, queries, outbox, commit queue.

mod memory;

#[cfg(feature = "sqlite")]
mod sqlite;

pub use memory::{InMemoryStore, StoreDocument, StoreError, StoreResult};

// TODO(slice 6a): backends, catalog, applier, export/import, jsonl archive.
// TODO(slice 6b): Durable StoreStorage integration.
