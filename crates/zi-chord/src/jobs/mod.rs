// Jobs (design §15). Slice 5: intent/run/finish; slice 6b: persisted jobs.

mod queue;

pub use queue::{JobDefinition, JobHandle, JobQueue, JobRunId, JobStatus};

// TODO(slice 5): self.job, busy/concurrency, finish in transaction with state doc.
// TODO(slice 6b): resume, restart limits on store.
