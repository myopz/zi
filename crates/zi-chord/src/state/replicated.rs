// Simple replicated document state: get / set / watch over async channels (in-process).

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::error::{ChordError, ChordResult};

const WATCH_CAP: usize = 64;

/// JSON state replicated locally; cross-host replication uses link ops (stub).
#[derive(Clone)]
pub struct ReplicatedState {
    inner: Arc<Inner>,
}

struct Inner {
    value: Arc<RwLock<Value>>,
    watch_tx: broadcast::Sender<Value>,
    cmd_tx: mpsc::UnboundedSender<StateCommand>,
}

enum StateCommand {
    Set(Value, tokio::sync::oneshot::Sender<ChordResult<()>>),
}

impl ReplicatedState {
    pub fn new(initial: Value) -> Self {
        let (watch_tx, _) = broadcast::channel(WATCH_CAP);
        let _ = watch_tx.send(initial.clone());
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let value = Arc::new(RwLock::new(initial));
        let value_writer = value.clone();
        let watch_writer = watch_tx.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    StateCommand::Set(v, reply) => {
                        {
                            let mut guard = value_writer.write();
                            *guard = v.clone();
                        }
                        let _ = watch_writer.send(v);
                        let _ = reply.send(Ok(()));
                    }
                }
            }
        });
        Self {
            inner: Arc::new(Inner {
                value,
                watch_tx,
                cmd_tx,
            }),
        }
    }

    pub fn get(&self) -> Value {
        self.inner.value.read().clone()
    }

    pub async fn set(&self, next: Value) -> ChordResult<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner
            .cmd_tx
            .send(StateCommand::Set(next, tx))
            .map_err(|_| ChordError::unavailable("state", "replicated state task stopped"))?;
        rx.await
            .map_err(|_| ChordError::unavailable("state", "replicated state task stopped"))?
    }

    pub fn watch(&self) -> broadcast::Receiver<Value> {
        self.inner.watch_tx.subscribe()
    }
}
