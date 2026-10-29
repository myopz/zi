//! Read and write coding-agent session v3 NDJSON (header + tree entries).

mod error;
mod migrate;
mod parse;
mod paths;
mod types;
mod write;

pub use error::SessionError;
pub use migrate::{migrate_session_entries, session_version};
pub use parse::{
    load_entries_from_path, load_entries_from_str, parse_session_entries, read_session_header,
};
pub use paths::{default_session_dir, encode_cwd_for_sessions, agent_dir, sessions_root};
pub use types::*;
pub use write::{append_entry_line, assert_valid_session_id, new_session_header, write_session_file, SessionWriter};
pub use types::NewSessionOptions;

pub const CURRENT_SESSION_VERSION: u32 = 3;
