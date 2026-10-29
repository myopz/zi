//! Migrate legacy session versions to v3 in place.

use std::collections::HashSet;

use serde_json::json;

use crate::types::{FileEntry, SessionEntry, SessionHeader};
use crate::CURRENT_SESSION_VERSION;

/// Run migrations on parsed entries. Returns true if any migration was applied.
pub fn migrate_session_entries(entries: &mut [FileEntry]) -> bool {
    let version = entries
        .first()
        .and_then(|e| match e {
            FileEntry::Header(h) => Some(h.version.unwrap_or(1)),
            _ => None,
        })
        .unwrap_or(1);

    if version >= CURRENT_SESSION_VERSION {
        return false;
    }

    let mut changed = false;
    if version < 2 {
        changed |= migrate_v1_to_v2(entries);
    }
    if version < 3 {
        changed |= migrate_v2_to_v3(entries);
    }
    changed
}

fn migrate_v1_to_v2(entries: &mut [FileEntry]) -> bool {
    let mut ids = HashSet::new();
    let mut prev_id: Option<String> = None;
    for entry in entries.iter_mut() {
        match entry {
            FileEntry::Header(header) => {
                header.version = Some(2);
            }
            FileEntry::Entry(session_entry) => {
                let id = generate_id(&ids);
                ids.insert(id.clone());
                apply_tree_fields(session_entry, id, prev_id.clone());
                prev_id = session_entry.id().map(str::to_string);
                if let SessionEntry::Compaction {
                    first_kept_entry_id,
                    ..
                } = session_entry
                {
                    if first_kept_entry_id.is_empty() {
                        if let Some(pid) = prev_id.clone() {
                            *first_kept_entry_id = pid;
                        }
                    }
                }
            }
            FileEntry::Raw(value) => {
                let id = generate_id(&ids);
                ids.insert(id.clone());
                value
                    .as_object_mut()
                    .expect("raw entry object")
                    .insert("id".to_string(), json!(id));
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("parentId".to_string(), json!(prev_id));
                prev_id = value.get("id").and_then(|v| v.as_str()).map(str::to_string);
            }
        }
    }
    true
}

fn migrate_v2_to_v3(entries: &mut [FileEntry]) -> bool {
    for entry in entries.iter_mut() {
        if let FileEntry::Header(header) = entry {
            header.version = Some(3);
        }
        if let FileEntry::Entry(SessionEntry::Message { message, .. }) = entry {
            if message.get("role").and_then(|r| r.as_str()) == Some("hookMessage") {
                message
                    .as_object_mut()
                    .expect("message object")
                    .insert("role".to_string(), json!("custom"));
            }
        }
        if let FileEntry::Raw(value) = entry {
            if value.get("type").and_then(|t| t.as_str()) == Some("message") {
                if let Some(role) = value.pointer("/message/role").and_then(|r| r.as_str()) {
                    if role == "hookMessage" {
                        value
                            .pointer_mut("/message/role")
                            .expect("role pointer")
                            .clone_from(&json!("custom"));
                    }
                }
            }
        }
    }
    true
}

fn apply_tree_fields(entry: &mut SessionEntry, id: String, parent_id: Option<String>) {
    match entry {
        SessionEntry::Message {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::ThinkingLevelChange {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::ModelChange {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::Usage {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::Compaction {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::BranchSummary {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::Custom {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::CustomMessage {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::ContextEdit {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::Label {
            id: eid,
            parent_id: pid,
            ..
        }
        | SessionEntry::SessionInfo {
            id: eid,
            parent_id: pid,
            ..
        } => {
            *eid = id;
            *pid = parent_id;
        }
    }
}

fn generate_id(existing: &HashSet<String>) -> String {
    for _ in 0..100 {
        let id = uuid::Uuid::now_v7().to_string()[..8].to_string();
        if !existing.contains(&id) {
            return id;
        }
    }
    uuid::Uuid::now_v7().to_string()
}

pub fn session_version(header: &SessionHeader) -> u32 {
    header.version.unwrap_or(1)
}
