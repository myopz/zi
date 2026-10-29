//! Parse session NDJSON lines.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::Value;

use crate::error::SessionError;
use crate::migrate::migrate_session_entries;
use crate::types::{FileEntry, SessionEntry, SessionHeader};

/// Parse session content from a string (skips blank lines; skips malformed JSON like pi).
pub fn parse_session_entries(content: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line) {
            Some(entry) => entries.push(entry),
            None => continue,
        }
    }
    entries
}

fn parse_line(line: &str) -> Option<FileEntry> {
    let value: Value = serde_json::from_str(line).ok()?;
    let kind = value.get("type")?.as_str()?;
    if kind == "session" {
        let header: SessionHeader = serde_json::from_value(value).ok()?;
        if header.id.is_empty() {
            return None;
        }
        return Some(FileEntry::Header(header));
    }
    match serde_json::from_value::<SessionEntry>(value.clone()) {
        Ok(entry) => Some(FileEntry::Entry(entry)),
        Err(_) => Some(FileEntry::Raw(value)),
    }
}

/// Load all entries from a session file path.
pub fn load_entries_from_path(path: &Path) -> Result<Vec<FileEntry>, SessionError> {
    let file = File::open(path)?;
    load_entries_from_reader(file)
}

/// Load entries from UTF-8 string and apply migrations in place.
pub fn load_entries_from_str(content: &str) -> Vec<FileEntry> {
    let mut entries = parse_session_entries(content);
    migrate_session_entries(&mut entries);
    entries
}

fn load_entries_from_reader(reader: impl Read) -> Result<Vec<FileEntry>, SessionError> {
    let mut entries = Vec::new();
    let buffered = BufReader::new(reader);
    for line in buffered.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(entry) = parse_line(&line) {
            entries.push(entry);
        }
    }
    if !entries.is_empty() {
        if let FileEntry::Header(header) = &entries[0] {
            if header.kind != "session" {
                return Ok(Vec::new());
            }
        } else {
            return Ok(Vec::new());
        }
    }
    migrate_session_entries(&mut entries);
    Ok(entries)
}

/// Returns the session header if the file begins with a valid header line.
pub fn read_session_header(content: &str) -> Option<SessionHeader> {
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(FileEntry::Header(header)) = parse_line(line) {
            return Some(header);
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"session","version":3,"id":"0192e1a0-1111-7000-8000-000000000001","timestamp":"2024-12-03T14:00:00.000Z","cwd":"/project"}
{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2024-12-03T14:00:01.000Z","message":{"role":"user","content":"Hello","timestamp":1733234401000}}
"#;

    #[test]
    fn parses_header_and_message() {
        let entries = parse_session_entries(SAMPLE);
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], FileEntry::Header(_)));
        assert!(matches!(entries[1], FileEntry::Entry(SessionEntry::Message { .. })));
    }
}
