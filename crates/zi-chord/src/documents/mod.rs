// Object documents (design §9). Slice 6a: persist, lines, keyed slots, store engine.

mod memory;

pub use memory::{DocumentStore, DocumentTx, JsonObject};

// TODO(slice 1): full transactions, draft/retire, seal fence per object line.
// TODO(slice 6a): ChordStore-backed documents, migrations, checkpointWhen.
