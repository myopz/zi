// Watchables and batching (design §11). Slice 6a: keyed watches; slice 8: NativeWatchable.

mod replicated;
pub(crate) mod watchable;

pub use replicated::ReplicatedState;
pub use watchable::{WatchHandle, Watchable};

// TODO(slice 1 full): Batch, Derive, Constant sources wired to documents engine.
// TODO(slice 12): self.derive, mapShared, lazy unwatched computes.
