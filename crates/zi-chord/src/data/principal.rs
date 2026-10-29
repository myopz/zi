// Who calls (design §4). Links stamp principals on remote calls (slice 3a — stub here).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Open principal record; apps pick concrete shapes per deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    Anonymous,
    Server,
    Guest { session: String },
    Custom(Value),
}

pub type PrincipalKeyFn = fn(&Principal) -> String;

pub fn default_principal_key(p: &Principal) -> String {
    serde_json::to_string(p).unwrap_or_else(|_| "anonymous".to_string())
}
