// Simple in-memory job queue (standing work stub).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ChordError, ChordResult};

pub type JobRunId = Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed { message: String },
    Aborted,
}

pub struct JobDefinition {
    pub name: String,
}

pub struct JobHandle {
    pub id: JobRunId,
    pub name: String,
    status: Arc<Mutex<JobStatus>>,
}

impl JobHandle {
    pub fn status(&self) -> JobStatus {
        self.status.lock().clone()
    }
}

#[derive(Default)]
pub struct JobQueue {
    runs: Mutex<HashMap<JobRunId, Arc<Mutex<JobStatus>>>>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(
        &self,
        job: &JobDefinition,
        _args: Value,
    ) -> ChordResult<JobHandle> {
        let id = Uuid::now_v7();
        let status = Arc::new(Mutex::new(JobStatus::Queued));
        self.runs.lock().insert(id, status.clone());
        let handle = JobHandle {
            id,
            name: job.name.clone(),
            status: status.clone(),
        };
        // TODO(slice 5): spawn run on host executor with ctx.jobId and abort signal.
        *status.lock() = JobStatus::Running;
        *status.lock() = JobStatus::Succeeded;
        Ok(handle)
    }

    pub fn abort(&self, id: JobRunId) -> ChordResult<()> {
        let runs = self.runs.lock();
        let status = runs
            .get(&id)
            .ok_or_else(|| ChordError::gone("unknown job run"))?;
        *status.lock() = JobStatus::Aborted;
        Ok(())
    }
}
