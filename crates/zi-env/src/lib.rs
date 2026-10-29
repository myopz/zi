//! **zi-env**: framed stdin/stdout daemon (`zi-envd`) and local/remote execution clients.
//!
//! Wire format matches pi `packages/env/docs/protocol.md` with sync line `ZI-ENV <token>`.

pub mod client;
pub mod daemon;
pub mod error;
pub mod protocol;

pub use client::{
    local_daemon_command, Connection, LocalExecutionEnv, LocalExecutionEnvOptions, RemoteExecutionEnv,
    RemoteExecutionEnvOptions, RemoteInfo, Reply,
};
pub use error::{EnvError, EnvResult};
pub use protocol::{FEATURES, PROTOCOL, SYNC_PREFIX};
