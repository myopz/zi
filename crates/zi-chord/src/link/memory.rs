// Ordered in-memory link pair (design §3 connect/accept subset).

use std::sync::{Arc, Weak};

use parking_lot::Mutex;

/// Bidirectional byte channel between two hosts.
pub trait Link: Send + Sync {
    fn send(&self, message: Vec<u8>);
    fn on_message(&self, listener: Box<dyn Fn(Vec<u8>) + Send + Sync>);
    fn close(&self);
    fn on_close(&self, listener: Box<dyn Fn() + Send + Sync>);
    fn closed(&self) -> bool;
}

pub struct LinkEnd {
    inner: Arc<MemoryLinkInner>,
}

struct MemoryLinkInner {
    peer: Mutex<Option<Weak<MemoryLinkInner>>>,
    closed: Mutex<bool>,
    message_listener: Mutex<Option<Box<dyn Fn(Vec<u8>) + Send + Sync>>>,
    close_listener: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    queued: Mutex<Vec<Vec<u8>>>,
}

impl LinkEnd {
    fn deliver(&self, message: Vec<u8>) {
        let closed = *self.inner.closed.lock();
        if closed {
            return;
        }
        let listener = self.inner.message_listener.lock();
        if let Some(l) = listener.as_ref() {
            l(message);
        } else {
            self.inner.queued.lock().push(message);
        }
    }

    fn ended(&self) {
        let mut closed = self.inner.closed.lock();
        if *closed {
            return;
        }
        *closed = true;
        let listener = self.inner.close_listener.lock().take();
        self.inner.message_listener.lock().take();
        *self.inner.queued.lock() = Vec::new();
        if let Some(l) = listener {
            l();
        }
    }
}

impl Link for LinkEnd {
    fn send(&self, message: Vec<u8>) {
        if *self.inner.closed.lock() {
            return;
        }
        let peer = self.inner.peer.lock().clone();
        if let Some(peer) = peer.and_then(|w| w.upgrade()) {
            LinkEnd { inner: peer }.deliver(message);
        }
    }

    fn on_message(&self, listener: Box<dyn Fn(Vec<u8>) + Send + Sync>) {
        if *self.inner.closed.lock() {
            return;
        }
        let queued: Vec<_> = self.inner.queued.lock().drain(..).collect();
        *self.inner.message_listener.lock() = Some(listener);
        if let Some(l) = self.inner.message_listener.lock().as_ref() {
            for msg in queued {
                if *self.inner.closed.lock() {
                    return;
                }
                l(msg);
            }
        }
    }

    fn close(&self) {
        let peer = self.inner.peer.lock().take();
        self.ended();
        if let Some(peer) = peer.and_then(|w| w.upgrade()) {
            LinkEnd { inner: peer }.ended();
        }
    }

    fn on_close(&self, listener: Box<dyn Fn() + Send + Sync>) {
        if *self.inner.closed.lock() {
            listener();
        } else {
            *self.inner.close_listener.lock() = Some(listener);
        }
    }

    fn closed(&self) -> bool {
        *self.inner.closed.lock()
    }
}

/// Two ends of one in-process link.
pub fn memory_link_pair() -> (Arc<dyn Link>, Arc<dyn Link>) {
    let a = Arc::new(MemoryLinkInner {
        peer: Mutex::new(None),
        closed: Mutex::new(false),
        message_listener: Mutex::new(None),
        close_listener: Mutex::new(None),
        queued: Mutex::new(Vec::new()),
    });
    let b = Arc::new(MemoryLinkInner {
        peer: Mutex::new(None),
        closed: Mutex::new(false),
        message_listener: Mutex::new(None),
        close_listener: Mutex::new(None),
        queued: Mutex::new(Vec::new()),
    });
    *a.peer.lock() = Some(Arc::downgrade(&b));
    *b.peer.lock() = Some(Arc::downgrade(&a));
    (
        Arc::new(LinkEnd { inner: a }) as Arc<dyn Link>,
        Arc::new(LinkEnd { inner: b }) as Arc<dyn Link>,
    )
}
