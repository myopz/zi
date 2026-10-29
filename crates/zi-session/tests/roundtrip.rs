use zi_session::{
    load_entries_from_str, migrate_session_entries, parse_session_entries, write_session_file, FileEntry,
    SessionEntry, SessionHeader, SessionWriter, NewSessionOptions,
};
use serde_json::json;
use tempfile::NamedTempFile;

#[test]
fn writes_and_reads_v3_session() {
    let mut writer = SessionWriter::new("/project", &NewSessionOptions {
        id: Some("sess0001".to_string()),
        parent_session: None,
    })
    .expect("writer");
    writer.push_entry(SessionEntry::Message {
        id: "a1b2c3d4".to_string(),
        parent_id: None,
        timestamp: "2024-12-03T14:00:01.000Z".to_string(),
        message: json!({"role":"user","content":"Hello","timestamp":1733234401000}),
    });

    let file = NamedTempFile::new().expect("temp");
    writer.write_to(file.path()).expect("write");

    let loaded = load_entries_from_str(&std::fs::read_to_string(file.path()).expect("read"));
    assert_eq!(loaded.len(), 2);
    match &loaded[0] {
        FileEntry::Header(SessionHeader { version, id, cwd, .. }) => {
            assert_eq!(*version, Some(3));
            assert_eq!(id, "sess0001");
            assert_eq!(cwd, "/project");
        }
        _ => panic!("expected header"),
    }
}

#[test]
fn migrates_hook_message_role_to_custom() {
    let content = r#"{"type":"session","version":2,"id":"s1","timestamp":"2024-01-01T00:00:00.000Z","cwd":"/"}
{"type":"message","id":"m1","parentId":null,"timestamp":"2024-01-01T00:00:01.000Z","message":{"role":"hookMessage","content":[]}}
"#;
    let mut entries = parse_session_entries(content);
    assert!(migrate_session_entries(&mut entries));
    match &entries[1] {
        FileEntry::Entry(SessionEntry::Message { message, .. }) => {
            assert_eq!(message.get("role").and_then(|v| v.as_str()), Some("custom"));
        }
        _ => panic!("message entry"),
    }
}

#[test]
fn rewrite_preserves_entries() {
    let entries = parse_session_entries(
        r#"{"type":"session","version":3,"id":"x","timestamp":"t","cwd":"/"}
{"type":"model_change","id":"1","parentId":null,"timestamp":"t","provider":"openai","modelId":"gpt-4o"}"#,
    );
    let file = NamedTempFile::new().expect("temp");
    write_session_file(file.path(), &entries).expect("write");
    let again = load_entries_from_str(&std::fs::read_to_string(file.path()).expect("read"));
    assert_eq!(entries.len(), again.len());
}
