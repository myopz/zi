// Contract descriptors (L1): local and remote shapes without full schema codegen.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::host_kind::Address;

pub const LOCAL_VARIANT: &str = "members";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContractId {
    pub name: String,
    pub version: u32,
}

#[derive(Clone, Debug)]
pub struct ParamField {
    pub name: String,
    pub required: bool,
}

#[derive(Clone, Debug)]
pub struct MethodSpec {
    pub name: String,
}

#[derive(Clone)]
pub struct ContractSpec {
    pub id: ContractId,
    pub kind: ContractKind,
    pub params: Vec<ParamField>,
    pub methods: Vec<MethodSpec>,
    pub state_members: Vec<String>,
    pub fingerprint: String,
    pub placement: Option<Arc<dyn Fn(&Map<String, Value>) -> Address + Send + Sync>>,
}

impl std::fmt::Debug for ContractSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractSpec")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("params", &self.params)
            .field("methods", &self.methods)
            .field("state_members", &self.state_members)
            .field("fingerprint", &self.fingerprint)
            .field("placement", &self.placement.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

/// Branded local contract handle (immutable by convention).
#[derive(Clone)]
pub struct LocalContract {
    pub spec: Arc<ContractSpec>,
}

impl LocalContract {
    pub fn new(
        name: impl Into<String>,
        version: u32,
        params: Vec<ParamField>,
        methods: Vec<MethodSpec>,
        state_members: Vec<String>,
    ) -> Self {
        let name = name.into();
        let fingerprint = format!("local:{name}:v{version}");
        Self {
            spec: Arc::new(ContractSpec {
                id: ContractId { name, version },
                kind: ContractKind::Local,
                params,
                methods,
                state_members,
                fingerprint,
                placement: None,
            }),
        }
    }
}

/// Remote contract with optional placement (slice 4a).
#[derive(Clone)]
pub struct RemoteContract {
    pub spec: Arc<ContractSpec>,
}

impl RemoteContract {
    pub fn new(
        name: impl Into<String>,
        version: u32,
        params: Vec<ParamField>,
        methods: Vec<MethodSpec>,
        state_members: Vec<String>,
        placement: Arc<dyn Fn(&Map<String, Value>) -> Address + Send + Sync>,
    ) -> Self {
        let name = name.into();
        let fingerprint = format!("remote:{name}:v{version}");
        Self {
            spec: Arc::new(ContractSpec {
                id: ContractId { name, version },
                kind: ContractKind::Remote,
                params,
                methods,
                state_members,
                fingerprint,
                placement: Some(placement),
            }),
        }
    }
}

impl ContractSpec {
    pub fn check_params(&self, params: &Map<String, Value>) -> Result<(), crate::error::ChordError> {
        for field in &self.params {
            if field.required && !params.contains_key(&field.name) {
                return Err(crate::error::ChordError::invalid(
                    "params",
                    format!("missing param `{}`", field.name),
                ));
            }
        }
        Ok(())
    }
}
