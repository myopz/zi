// Links between hosts (design §3, §22). Slice 3a: wire codec; slice 4a: IPC/WebSocket, flow control.

mod memory;
mod message;

pub use memory::{memory_link_pair, Link, LinkEnd};
pub use message::{LinkEnvelope, LinkMessage, MethodCall, MethodResponse};

// TODO(slice 3a): protocol version handshake, holds, one message per commit.
// TODO(slice 3b): re-holds, parking, ref.state, linger.
// TODO(slice 4a): relays, heartbeat, ack-clocked windows.
// TODO(slice 4b): compression, size limits.
