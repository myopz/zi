// Wire protocol types shared by zi-envd and clients.

pub mod frame;

pub use frame::{decode_frame, Frame, ERROR, EVENT, PING, REQUEST, RESULT, CANCEL, MAX_FRAME};

pub const PROTOCOL: u64 = 1;
pub const SYNC_PREFIX: &str = "ZI-ENV";
pub const FEATURES: &[&str] = &["listFiles", "read", "write"];
