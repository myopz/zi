// Host kinds and addresses (design §3). Slice 4a: route, authorize, relay.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::data::identity::identity_key;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostKindId {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKind {
    pub id: HostKindId,
    pub param_fields: Vec<String>,
}

impl HostKind {
    pub fn new(name: impl Into<String>, param_fields: Vec<String>) -> Self {
        Self {
            id: HostKindId {
                name: name.into(),
            },
            param_fields,
        }
    }
}

/// Typed host address: `{ kind, params, key }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub kind: HostKindId,
    pub params: Map<String, Value>,
    pub key: String,
}

impl Address {
    pub fn new(kind: HostKind, params: Map<String, Value>) -> Self {
        let key = identity_key(&kind.id.name, &params);
        Self {
            kind: kind.id,
            params,
            key,
        }
    }
}
