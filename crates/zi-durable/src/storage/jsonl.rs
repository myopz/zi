// JSONL commit log storage (pi JsonlStorage subset: main.jsonl replay).

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DurableError, DurableResult};
use crate::storage::memory::MemoryState;
use crate::storage::{apply_writes, validate_writes, Storage};
use crate::types::{
    ConversationId, ConversationRecord, EntryId, EntryRecord, Seq, StorageWrite,
};

const FORMAT_VERSION: u32 = 1;
const MAIN_FILE: &str = "main.jsonl";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitMarker {
    format: u32,
    #[serde(rename = "type")]
    kind: String,
    seq: u64,
    writes: Vec<StorageWrite>,
}

/// Directory-backed storage: replays `main.jsonl` on open and appends commit markers.
pub struct JsonlStorage {
    dir: PathBuf,
    state: MemoryState,
}

impl JsonlStorage {
    /// Open or create storage under `dir`.
    pub fn open(dir: impl AsRef<Path>) -> DurableResult<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let mut storage = Self {
            dir,
            state: MemoryState {
                next_seq: Seq(1),
                next_id: 1,
                ..MemoryState::default()
            },
        };
        storage.replay()?;
        Ok(storage)
    }

    fn main_path(&self) -> PathBuf {
        self.dir.join(MAIN_FILE)
    }

    fn replay(&mut self) -> DurableResult<()> {
        let path = self.main_path();
        if !path.exists() {
            return Ok(());
        }
        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let marker: CommitMarker = serde_json::from_str(&line).map_err(|error| {
                DurableError::Storage(format!("corrupt jsonl at {}: {error}", path.display()))
            })?;
            if marker.format != FORMAT_VERSION || marker.kind != "commit" {
                return Err(DurableError::Storage(format!(
                    "unsupported marker in {}",
                    path.display()
                )));
            }
            let seq = Seq(marker.seq);
            if seq.0 >= self.state.next_seq.0 {
                self.state.next_seq = Seq(seq.0 + 1);
            }
            apply_writes(&mut self.state, &marker.writes, seq);
            for write in &marker.writes {
                match write {
                    StorageWrite::Conversation { value } => {
                        self.state.next_id = self.state.next_id.max(value.id.0 + 1);
                    }
                    StorageWrite::Entry { value } => {
                        self.state.next_id = self.state.next_id.max(value.id.0 + 1);
                    }
                }
            }
        }
        Ok(())
    }

    fn append_marker(&self, seq: Seq, writes: &[StorageWrite]) -> DurableResult<()> {
        let marker = CommitMarker {
            format: FORMAT_VERSION,
            kind: "commit".into(),
            seq: seq.0,
            writes: writes.to_vec(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.main_path())?;
        serde_json::to_writer(&mut file, &marker)?;
        file.write_all(b"\n")?;
        Ok(())
    }
}

impl Storage for JsonlStorage {
    fn commit(&mut self, writes: &[StorageWrite]) -> DurableResult<Seq> {
        validate_writes(writes)?;
        let seq = self.state.next_seq;
        self.append_marker(seq, writes)?;
        apply_writes(&mut self.state, writes, seq);
        Ok(seq)
    }

    fn mint_id(&mut self) -> u64 {
        let id = self.state.next_id;
        self.state.next_id += 1;
        id
    }

    fn conversation(&self, id: ConversationId) -> Option<ConversationRecord> {
        self.state.conversations.get(&id).cloned()
    }

    fn entry(&self, id: EntryId) -> Option<(EntryRecord, Seq)> {
        let entry = self.state.entries.get(&id)?.clone();
        if !entry.visible {
            return None;
        }
        let seq = self.state.entry_commit_seqs.get(&id).copied()?;
        Some((entry, seq))
    }

    fn entries_for_conversation(&self, conversation_id: ConversationId) -> Vec<(EntryRecord, Seq)> {
        let Some(ids) = self.state.entry_ids.get(&conversation_id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for id in ids {
            if let Some(pair) = self.entry(*id) {
                out.push(pair);
            }
        }
        out
    }
}
