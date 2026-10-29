//! Session v3 entry types (aligned with pi `session-manager.ts`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "parentSession")]
    pub parent_session: Option<String>,
}

impl SessionHeader {
    pub fn new(id: String, timestamp: String, cwd: String, parent_session: Option<String>) -> Self {
        Self {
            kind: "session".to_string(),
            version: Some(super::CURRENT_SESSION_VERSION),
            id,
            timestamp,
            cwd,
            parent_session,
        }
    }
}

/// One append-only session entry (excludes the file header).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        message: Value,
    },
    ThinkingLevelChange {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: String,
    },
    ModelChange {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    Usage {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        kind: String,
        provider: String,
        model: String,
        usage: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Compaction {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        summary: String,
        #[serde(rename = "firstKeptEntryId")]
        first_kept_entry_id: String,
        #[serde(rename = "tokensBefore")]
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "fromHook")]
        from_hook: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "systemMessage")]
        system_message: Option<Value>,
    },
    BranchSummary {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "fromId")]
        from_id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "fromHook")]
        from_hook: Option<bool>,
    },
    Custom {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "customType")]
        custom_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
    CustomMessage {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "customType")]
        custom_type: String,
        content: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        display: bool,
    },
    ContextEdit {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "targetId")]
        target_id: String,
        replacement: Option<Value>,
    },
    Label {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "targetId")]
        target_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    SessionInfo {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl SessionEntry {
    pub fn id(&self) -> Option<&str> {
        match self {
            SessionEntry::Message { id, .. }
            | SessionEntry::ThinkingLevelChange { id, .. }
            | SessionEntry::ModelChange { id, .. }
            | SessionEntry::Usage { id, .. }
            | SessionEntry::Compaction { id, .. }
            | SessionEntry::BranchSummary { id, .. }
            | SessionEntry::Custom { id, .. }
            | SessionEntry::CustomMessage { id, .. }
            | SessionEntry::ContextEdit { id, .. }
            | SessionEntry::Label { id, .. }
            | SessionEntry::SessionInfo { id, .. } => Some(id),
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        match self {
            SessionEntry::Message { parent_id, .. }
            | SessionEntry::ThinkingLevelChange { parent_id, .. }
            | SessionEntry::ModelChange { parent_id, .. }
            | SessionEntry::Usage { parent_id, .. }
            | SessionEntry::Compaction { parent_id, .. }
            | SessionEntry::BranchSummary { parent_id, .. }
            | SessionEntry::Custom { parent_id, .. }
            | SessionEntry::CustomMessage { parent_id, .. }
            | SessionEntry::ContextEdit { parent_id, .. }
            | SessionEntry::Label { parent_id, .. }
            | SessionEntry::SessionInfo { parent_id, .. } => parent_id.as_deref(),
        }
    }
}

/// Raw file line: session header or tree entry.
#[derive(Debug, Clone, PartialEq)]
pub enum FileEntry {
    Header(SessionHeader),
    Entry(SessionEntry),
    /// Malformed or unknown JSON object preserved for round-trip debugging.
    Raw(Value),
}

pub struct NewSessionOptions {
    pub id: Option<String>,
    pub parent_session: Option<String>,
}
