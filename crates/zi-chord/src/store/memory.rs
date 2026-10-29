// In-memory document/state store backing host objects.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("storage failed: {0}")]
    Failed(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug)]
pub struct StoreDocument {
    pub owner: String,
    pub name: String,
    pub value: Value,
}

#[derive(Clone, Default)]
pub struct InMemoryStore {
    documents: Arc<RwLock<HashMap<String, StoreDocument>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(owner: &str, name: &str) -> String {
        format!("{owner}/{name}")
    }

    pub fn put(&self, owner: &str, name: &str, value: Value) -> StoreResult<()> {
        self.documents.write().insert(
            Self::key(owner, name),
            StoreDocument {
                owner: owner.to_string(),
                name: name.to_string(),
                value,
            },
        );
        Ok(())
    }

    pub fn get(&self, owner: &str, name: &str) -> StoreResult<Value> {
        self.documents
            .read()
            .get(&Self::key(owner, name))
            .map(|d| d.value.clone())
            .ok_or_else(|| StoreError::NotFound(name.to_string()))
    }

    pub fn list_owner(&self, owner: &str) -> Vec<StoreDocument> {
        self.documents
            .read()
            .values()
            .filter(|d| d.owner == owner)
            .cloned()
            .collect()
    }
}
