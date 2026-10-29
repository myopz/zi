// Chord errors (L1 / design §6): tagged outcomes for callers and wire.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Application-visible failure kinds aligned with Chord v2 `ChordError`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChordErrorKind {
    Invalid {
        reason: String,
        message: String,
    },
    Released {
        message: String,
    },
    Unavailable {
        reason: String,
        message: String,
    },
    Gone {
        message: String,
    },
    Broken {
        reason: String,
        message: String,
    },
}

#[derive(Error, Debug, Clone)]
#[error("{kind:?}: {message}")]
pub struct ChordError {
    pub kind: ChordErrorKind,
    pub message: String,
}

impl ChordError {
    pub fn invalid(reason: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: ChordErrorKind::Invalid {
                reason: reason.into(),
                message: message.clone(),
            },
            message,
        }
    }

    pub fn released(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: ChordErrorKind::Released {
                message: message.clone(),
            },
            message,
        }
    }

    pub fn unavailable(reason: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: ChordErrorKind::Unavailable {
                reason: reason.into(),
                message: message.clone(),
            },
            message,
        }
    }

    pub fn gone(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: ChordErrorKind::Gone {
                message: message.clone(),
            },
            message,
        }
    }

    pub fn broken(reason: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: ChordErrorKind::Broken {
                reason: reason.into(),
                message: message.clone(),
            },
            message,
        }
    }

    pub fn is_kind(&self, kind: &str) -> bool {
        match (&self.kind, kind) {
            (ChordErrorKind::Invalid { .. }, "invalid") => true,
            (ChordErrorKind::Released { .. }, "released") => true,
            (ChordErrorKind::Unavailable { .. }, "unavailable") => true,
            (ChordErrorKind::Gone { .. }, "gone") => true,
            (ChordErrorKind::Broken { .. }, "broken") => true,
            _ => false,
        }
    }
}

pub type ChordResult<T> = Result<T, ChordError>;
