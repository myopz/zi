//! Write session NDJSON files.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use uuid::Uuid;

use crate::error::SessionError;
use crate::types::{FileEntry, NewSessionOptions, SessionEntry, SessionHeader};

/// Create a new v3 session header (does not write a file).
pub fn new_session_header(cwd: impl Into<String>, options: &NewSessionOptions) -> Result<SessionHeader, SessionError> {
    let id = options
        .id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    assert_valid_session_id(&id)?;
    let timestamp = chrono_now_iso8601();
    Ok(SessionHeader::new(id, timestamp, cwd.into(), options.parent_session.clone()))
}

fn chrono_now_iso8601() -> String {
    // Avoid chrono dependency: use RFC3339 from std time.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    format_iso8601(secs, nanos)
}

fn format_iso8601(secs: u64, nanos: u32) -> String {
    // Minimal UTC formatter sufficient for session filenames and headers.
    let datetime = time_from_unix(secs);
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

fn time_from_unix(secs: u64) -> DateTime {
    // Algorithm from common civil time conversion (UTC).
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

/// Validate session id (same rules as pi session-manager).
pub fn assert_valid_session_id(id: &str) -> Result<(), SessionError> {
    let valid = !id.is_empty()
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id.ends_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if valid {
        Ok(())
    } else {
        Err(SessionError::InvalidSessionId(id.to_string()))
    }
}

/// Write all file entries as NDJSON (one JSON object per line, trailing newline).
pub fn write_session_file(path: &Path, entries: &[FileEntry]) -> Result<(), SessionError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for entry in entries {
        let line = entry_to_json(entry)?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn entry_to_json(entry: &FileEntry) -> Result<String, SessionError> {
    let value = match entry {
        FileEntry::Header(header) => serde_json::to_value(header)?,
        FileEntry::Entry(session_entry) => serde_json::to_value(session_entry)?,
        FileEntry::Raw(value) => value.clone(),
    };
    Ok(serde_json::to_string(&value)?)
}

/// Append one entry line to an existing session file.
pub fn append_entry_line(path: &Path, entry: &FileEntry) -> Result<(), SessionError> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    let line = entry_to_json(entry)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Helper for constructing new in-memory sessions and writing them once.
pub struct SessionWriter {
    pub entries: Vec<FileEntry>,
}

impl SessionWriter {
    pub fn new(cwd: impl Into<String>, options: &NewSessionOptions) -> Result<Self, SessionError> {
        let header = new_session_header(cwd, options)?;
        Ok(Self {
            entries: vec![FileEntry::Header(header)],
        })
    }

    pub fn header(&self) -> Option<&SessionHeader> {
        self.entries.first().and_then(|e| match e {
            FileEntry::Header(h) => Some(h),
            _ => None,
        })
    }

    pub fn push_entry(&mut self, entry: SessionEntry) {
        self.entries.push(FileEntry::Entry(entry));
    }

    pub fn write_to(&self, path: &Path) -> Result<(), SessionError> {
        write_session_file(path, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::load_entries_from_str;
    use serde_json::json;

    #[test]
    fn round_trip_header_and_message() {
        let mut writer = SessionWriter::new("/tmp/project", &NewSessionOptions {
            id: Some("abc12345".to_string()),
            parent_session: None,
        })
        .expect("header");
        writer.push_entry(SessionEntry::Message {
            id: "msg00001".to_string(),
            parent_id: None,
            timestamp: "2024-12-03T14:00:01.000Z".to_string(),
            message: json!({"role":"user","content":"hi","timestamp":1}),
        });
        let serialized: Vec<String> = writer
            .entries
            .iter()
            .map(|e| entry_to_json(e).expect("json"))
            .collect();
        let content = serialized.join("\n") + "\n";
        let loaded = load_entries_from_str(&content);
        assert_eq!(loaded.len(), 2);
    }
}
