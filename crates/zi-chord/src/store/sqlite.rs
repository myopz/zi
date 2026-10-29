// SQLite store adapter (feature `sqlite`). Slice 6a: full schema and migrations.

use serde_json::Value;

use super::{InMemoryStore, StoreError, StoreResult};

/// Placeholder until slice 6a wires rusqlite backends.
pub struct SqliteStore {
    fallback: InMemoryStore,
}

impl SqliteStore {
    pub fn open(_path: &str) -> StoreResult<Self> {
        // TODO(slice 6a): open rusqlite connection, migrate schema, replace fallback.
        Ok(Self {
            fallback: InMemoryStore::new(),
        })
    }

    pub fn put(&self, owner: &str, name: &str, value: Value) -> StoreResult<()> {
        self.fallback.put(owner, name, value)
    }

    pub fn get(&self, owner: &str, name: &str) -> StoreResult<Value> {
        self.fallback.get(owner, name)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Failed(err.to_string())
    }
}
