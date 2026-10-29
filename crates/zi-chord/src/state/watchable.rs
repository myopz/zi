// In-memory watchable values (Chord Watch subset).

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::broadcast;

use crate::error::{ChordError, ChordResult};

const WATCH_CAP: usize = 64;

#[derive(Clone)]
pub struct Watchable<T: Clone + Send + Sync + 'static> {
    inner: Arc<WatchInner<T>>,
}

struct WatchInner<T: Clone + Send + Sync + 'static> {
    value: RwLock<T>,
    tx: broadcast::Sender<T>,
    released: RwLock<bool>,
}

impl<T: Clone + Send + Sync + 'static> Watchable<T> {
    pub fn new(initial: T) -> Self {
        let (tx, _) = broadcast::channel(WATCH_CAP);
        let _ = tx.send(initial.clone());
        Self {
            inner: Arc::new(WatchInner {
                value: RwLock::new(initial),
                tx,
                released: RwLock::new(false),
            }),
        }
    }

    pub fn value(&self) -> ChordResult<T> {
        if *self.inner.released.read() {
            return Err(ChordError::released("watchable released"));
        }
        Ok(self.inner.value.read().clone())
    }

    pub fn set(&self, next: T) -> ChordResult<()> {
        if *self.inner.released.read() {
            return Err(ChordError::released("watchable released"));
        }
        {
            let mut guard = self.inner.value.write();
            *guard = next.clone();
        }
        let _ = self.inner.tx.send(next);
        Ok(())
    }

    pub fn watch(&self) -> ChordResult<WatchHandle<T>> {
        if *self.inner.released.read() {
            return Err(ChordError::released("watchable released"));
        }
        let rx = self.inner.tx.subscribe();
        Ok(WatchHandle {
            inner: self.inner.clone(),
            rx,
        })
    }

    pub fn release(&self) {
        *self.inner.released.write() = true;
    }
}

pub struct WatchHandle<T: Clone + Send + Sync + 'static> {
    inner: Arc<WatchInner<T>>,
    rx: broadcast::Receiver<T>,
}

impl<T: Clone + Send + Sync + 'static> WatchHandle<T> {
    pub fn value(&self) -> ChordResult<T> {
        if *self.inner.released.read() {
            return Err(ChordError::released("watchable released"));
        }
        Ok(self.inner.value.read().clone())
    }

    pub async fn changed(&mut self) -> ChordResult<T> {
        if *self.inner.released.read() {
            return Err(ChordError::released("watchable released"));
        }
        match self.rx.recv().await {
            Ok(v) => Ok(v),
            Err(broadcast::error::RecvError::Lagged(_)) => Ok(self.inner.value.read().clone()),
            Err(broadcast::error::RecvError::Closed) => {
                Err(ChordError::released("watch channel closed"))
            }
        }
    }
}
