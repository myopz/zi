// In-memory JSON documents per object (P2 subset).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::{Map, Value};

pub type JsonObject = Map<String, Value>;

#[derive(Clone, Default)]
pub struct DocumentStore {
    docs: Arc<RwLock<HashMap<String, Value>>>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&self, name: &str) -> Option<Value> {
        self.docs.read().get(name).cloned()
    }

    pub fn begin(&self) -> DocumentTx {
        DocumentTx {
            store: self.clone(),
            draft: self.docs.read().clone(),
        }
    }
}

pub struct DocumentTx {
    store: DocumentStore,
    draft: HashMap<String, Value>,
}

impl DocumentTx {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.draft.get(name)
    }

    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.draft.insert(name.into(), value);
    }

    pub fn commit(self) {
        *self.store.docs.write() = self.draft;
    }
}
