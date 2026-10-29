// Object identity keys from contract name + canonical params (L1).

use serde_json::{Map, Value};

pub type IdentityKey = String;

/// Builds `name|param1|param2` from sorted param keys (canonical JSON values).
pub fn identity_key(contract_name: &str, params: &Map<String, Value>) -> IdentityKey {
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort();
    let mut parts = vec![contract_name.to_string()];
    for k in keys {
        parts.push(format!("{k}={}", params[k]));
    }
    parts.join("|")
}
