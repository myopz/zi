// Per-call context (design §8).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::data::Principal;

/// Call-scoped context: cancellation and principal (telemetry attached on host).
#[derive(Clone)]
pub struct CallContext {
    pub principal: Option<Principal>,
    aborted: Arc<AtomicBool>,
}

impl CallContext {
    pub fn background() -> Self {
        Self {
            principal: None,
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_principal(principal: Principal) -> Self {
        Self {
            principal: Some(principal),
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }
}

/// Host-lifetime context passed into factories.
#[derive(Clone)]
pub struct HostContext {
    pub call: CallContext,
    pub host_id: uuid::Uuid,
    pub store_owner_prefix: Arc<str>,
}
