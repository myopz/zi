// JSON link envelopes (wire subset). Slice 3a replaces with binary codec + parts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkMessage {
    MethodCall(MethodCall),
    MethodResponse(MethodResponse),
    StatePush {
        contract: String,
        params: Value,
        member: String,
        value: Value,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkEnvelope {
    pub id: Uuid,
    pub body: LinkMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodCall {
    pub contract: String,
    pub params: Value,
    pub method: String,
    pub args: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodResponse {
    pub call_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl LinkEnvelope {
    pub fn new(body: LinkMessage) -> Self {
        Self {
            id: Uuid::now_v7(),
            body,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}
