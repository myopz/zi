//! Session v3 NDJSON persistence for CLI runs.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use zi_ai::Message;
use zi_session::{
    append_entry_line, encode_cwd_for_sessions, load_entries_from_path, FileEntry,
    NewSessionOptions, SessionEntry, SessionHeader, SessionWriter,
};
use serde_json::Value;
use uuid::Uuid;

/// Agent data root (`ZI_AGENT_DIR` or `~/.zi/agent`).
pub fn resolve_agent_dir() -> PathBuf {
    std::env::var_os("ZI_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(zi_session::agent_dir)
}

fn sessions_dir_for_cwd(cwd: &Path) -> PathBuf {
    resolve_agent_dir()
        .join("sessions")
        .join(encode_cwd_for_sessions(cwd))
}

fn iso8601_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    let datetime = unix_to_datetime(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        datetime.year,
        datetime.month,
        datetime.day,
        datetime.hour,
        datetime.minute,
        datetime.second,
        nanos / 1_000_000
    )
}

struct DateTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn unix_to_datetime(secs: u64) -> DateTime {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    let second = (rem % 60) as u32;
    let z = days as i64 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp as i64 + if mp < 10 { 3 } else { -9 }) as u32;
    let year = (y + if month <= 2 { 1 } else { 0 }) as i32;
    DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// Append-only session file for one CLI run.
pub struct SessionStore {
    enabled: bool,
    path: Option<PathBuf>,
    header: Option<SessionHeader>,
    last_entry_id: Option<String>,
    /// Display name from the latest `session_info` entry.
    name: Option<String>,
}

impl SessionStore {
    /// Create a new session file under `~/.zi/agent/sessions/...` unless disabled.
    pub fn open(cwd: &Path, no_session: bool) -> Result<Self> {
        if no_session {
            return Ok(Self {
                enabled: false,
                path: None,
                header: None,
                last_entry_id: None,
                name: None,
            });
        }

        let dir = sessions_dir_for_cwd(cwd);
        fs::create_dir_all(&dir).with_context(|| format!("create session dir {}", dir.display()))?;

        let session_id = Uuid::now_v7().to_string();
        let timestamp = iso8601_now();
        let file_name = format!("{}_{session_id}.jsonl", timestamp.replace(':', "-"));
        let path = dir.join(file_name);

        let writer = SessionWriter::new(
            cwd.to_string_lossy().into_owned(),
            &NewSessionOptions {
                id: Some(session_id.clone()),
                parent_session: None,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        writer
            .write_to(&path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let header = writer.header().cloned();

        Ok(Self {
            enabled: true,
            path: Some(path),
            header,
            last_entry_id: None,
            name: None,
        })
    }

    /// Session header line for JSON mode consumers.
    pub fn header(&self) -> Option<&SessionHeader> {
        self.header.as_ref()
    }

    /// Absolute path of the current session file, when enabled.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Optional display name (`/name`).
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Parent session id from the header, if forked.
    pub fn parent_session_id(&self) -> Option<&str> {
        self.header
            .as_ref()
            .and_then(|h| h.parent_session.as_deref())
    }

    /// Start a fresh session file for `cwd` (replaces this store).
    pub fn restart(&mut self, cwd: &Path) -> Result<()> {
        let disabled = !self.enabled;
        *self = Self::open(cwd, disabled)?;
        Ok(())
    }

    /// Open an existing session file and restore message entries as [`Message`]s.
    ///
    /// Compaction entries reset the projected transcript: earlier messages are dropped and the
    /// compaction summary is injected as a user message before messages that follow.
    pub fn open_existing(cwd: &Path, path: &Path) -> Result<(Self, Vec<Message>)> {
        let entries = load_entries_from_path(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut header = None;
        let mut last_entry_id = None;
        let mut name = None;
        let mut messages = Vec::new();
        // Parallel list of message entry ids so compaction can find `first_kept_entry_id`.
        let mut message_entry_ids: Vec<String> = Vec::new();
        let mut leading_system: Vec<Message> = Vec::new();

        for entry in entries {
            match entry {
                FileEntry::Header(h) => header = Some(h),
                FileEntry::Entry(SessionEntry::Message { id, message, .. }) => {
                    last_entry_id = Some(id.clone());
                    match serde_json::from_value::<Message>(message) {
                        Ok(m) => {
                            if matches!(m, Message::System(_)) && message_entry_ids.is_empty() {
                                leading_system.push(m.clone());
                            }
                            messages.push(m);
                            message_entry_ids.push(id);
                        }
                        Err(err) => tracing::warn!("skip session message: {err}"),
                    }
                }
                FileEntry::Entry(SessionEntry::Compaction {
                    id,
                    summary,
                    ..
                }) => {
                    last_entry_id = Some(id);
                    // Reset projection: keep leading system prompts, inject summary, then
                    // subsequent Message entries after this compaction rebuild the tail.
                    let mut projected = leading_system.clone();
                    projected.push(Message::User(zi_ai::UserMessage {
                        role: zi_ai::UserRole::User,
                        content: zi_ai::UserContent::Text(format!(
                            "<compaction>\n{summary}\n</compaction>"
                        )),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                    }));
                    messages = projected;
                    message_entry_ids.clear();
                }
                FileEntry::Entry(SessionEntry::SessionInfo {
                    id,
                    name: n,
                    ..
                }) => {
                    last_entry_id = Some(id);
                    if n.is_some() {
                        name = n;
                    }
                }
                FileEntry::Entry(other) => {
                    if let Some(id) = other.id() {
                        last_entry_id = Some(id.to_string());
                    }
                }
                FileEntry::Raw(_) => {}
            }
        }
        let _ = cwd;
        Ok((
            Self {
                enabled: true,
                path: Some(path.to_path_buf()),
                header,
                last_entry_id,
                name,
            },
            messages,
        ))
    }

    /// List recent session files for `cwd` (newest first).
    pub fn list_sessions(cwd: &Path, limit: usize) -> Result<Vec<PathBuf>> {
        let dir = sessions_dir_for_cwd(cwd);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files: Vec<_> = fs::read_dir(&dir)
            .with_context(|| format!("read sessions {}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .collect();
        files.sort_by(|a, b| b.cmp(a));
        files.truncate(limit);
        Ok(files)
    }

    /// Persist one transcript message when session tracking is enabled.
    pub fn append_message(&mut self, message: &Message) -> Result<()> {
        let entry_id = Uuid::now_v7().to_string();
        let parent_id = self.last_entry_id.clone();
        let message_value: Value = serde_json::to_value(message)?;
        let entry = SessionEntry::Message {
            id: entry_id.clone(),
            parent_id,
            timestamp: iso8601_now(),
            message: message_value,
        };
        self.append_entry(entry)?;
        Ok(())
    }

    /// Set the session display name (`/name`).
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<()> {
        let name = name.into();
        let entry = SessionEntry::SessionInfo {
            id: Uuid::now_v7().to_string(),
            parent_id: self.last_entry_id.clone(),
            timestamp: iso8601_now(),
            name: Some(name.clone()),
        };
        self.append_entry(entry)?;
        self.name = Some(name);
        Ok(())
    }

    /// Append a compaction summary entry and return its id.
    ///
    /// `first_kept_entry_id` should be the first message entry id still present in the
    /// projected transcript (typically the current `last_entry_id` before the compaction
    /// line is written, i.e. the last kept message).
    pub fn append_compaction(
        &mut self,
        summary: impl Into<String>,
        first_kept_entry_id: impl Into<String>,
        tokens_before: u64,
    ) -> Result<String> {
        let entry_id = Uuid::now_v7().to_string();
        let entry = SessionEntry::Compaction {
            id: entry_id.clone(),
            parent_id: self.last_entry_id.clone(),
            timestamp: iso8601_now(),
            summary: summary.into(),
            first_kept_entry_id: first_kept_entry_id.into(),
            tokens_before,
            details: None,
            usage: None,
            from_hook: None,
            system_message: None,
        };
        self.append_entry(entry)?;
        Ok(entry_id)
    }

    /// Last appended entry id (message or otherwise), when known.
    pub fn last_entry_id(&self) -> Option<&str> {
        self.last_entry_id.as_deref()
    }

    /// Append a branch summary (used by `/fork` bookkeeping).
    pub fn append_branch_summary(
        &mut self,
        from_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<()> {
        let entry = SessionEntry::BranchSummary {
            id: Uuid::now_v7().to_string(),
            parent_id: self.last_entry_id.clone(),
            timestamp: iso8601_now(),
            from_id: from_id.into(),
            summary: summary.into(),
            details: None,
            usage: None,
            from_hook: None,
        };
        self.append_entry(entry)
    }

    /// Fork: new session file with `parentSession` pointing at this session.
    pub fn fork(&self, cwd: &Path) -> Result<Self> {
        if !self.enabled {
            anyhow::bail!("session disabled; cannot fork");
        }
        let parent_id = self
            .header
            .as_ref()
            .map(|h| h.id.clone())
            .ok_or_else(|| anyhow::anyhow!("missing session header"))?;

        let dir = sessions_dir_for_cwd(cwd);
        fs::create_dir_all(&dir).with_context(|| format!("create session dir {}", dir.display()))?;

        let session_id = Uuid::now_v7().to_string();
        let timestamp = iso8601_now();
        let file_name = format!("{}_{session_id}.jsonl", timestamp.replace(':', "-"));
        let path = dir.join(file_name);

        let writer = SessionWriter::new(
            cwd.to_string_lossy().into_owned(),
            &NewSessionOptions {
                id: Some(session_id),
                parent_session: Some(parent_id.clone()),
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        writer
            .write_to(&path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let header = writer.header().cloned();

        let mut forked = Self {
            enabled: true,
            path: Some(path),
            header,
            last_entry_id: None,
            name: self.name.clone(),
        };
        forked.append_branch_summary(parent_id, "forked from parent session")?;
        Ok(forked)
    }

    /// Tree view of entry types/ids for the current session file.
    pub fn tree_summary(&self) -> Result<String> {
        let Some(path) = &self.path else {
            return Ok("(no session file)".into());
        };
        let entries = load_entries_from_path(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut lines = Vec::new();
        if let Some(name) = &self.name {
            lines.push(format!("name: {name}"));
        }
        if let Some(parent) = self.parent_session_id() {
            lines.push(format!("parentSession: {parent}"));
        }
        lines.push("entries:".into());
        for entry in entries {
            match entry {
                FileEntry::Header(h) => {
                    lines.push(format!("  [header] id={} cwd={}", h.id, h.cwd));
                }
                FileEntry::Entry(e) => {
                    let id = e.id().unwrap_or("-");
                    let parent = e.parent_id().unwrap_or("-");
                    let kind = match &e {
                        SessionEntry::Message { .. } => "message",
                        SessionEntry::Compaction { .. } => "compaction",
                        SessionEntry::BranchSummary { .. } => "branch_summary",
                        SessionEntry::SessionInfo { .. } => "session_info",
                        SessionEntry::ModelChange { .. } => "model_change",
                        SessionEntry::ThinkingLevelChange { .. } => "thinking_level_change",
                        SessionEntry::Usage { .. } => "usage",
                        SessionEntry::Custom { .. } => "custom",
                        SessionEntry::CustomMessage { .. } => "custom_message",
                        SessionEntry::ContextEdit { .. } => "context_edit",
                        SessionEntry::Label { .. } => "label",
                    };
                    lines.push(format!("  [{kind}] id={id} parent={parent}"));
                }
                FileEntry::Raw(_) => lines.push("  [raw]".into()),
            }
        }
        Ok(lines.join("\n"))
    }

    fn append_entry(&mut self, entry: SessionEntry) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session path missing"))?;
        let id = entry
            .id()
            .ok_or_else(|| anyhow::anyhow!("entry missing id"))?
            .to_string();
        append_entry_line(path, &FileEntry::Entry(entry)).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.last_entry_id = Some(id);
        Ok(())
    }
}
