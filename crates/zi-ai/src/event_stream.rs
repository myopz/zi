// Async event stream for one assistant response (aligned with pi event-stream.ts).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_stream::stream;
use futures::Stream;
use tokio::sync::Notify;

use crate::types::{AssistantMessage, AssistantMessageEvent};

struct StreamInner {
    queue: VecDeque<AssistantMessageEvent>,
    done: bool,
    result: Option<AssistantMessage>,
    result_waiters: Vec<tokio::sync::oneshot::Sender<AssistantMessage>>,
    notify: Arc<Notify>,
}

struct SharedStream {
    inner: Mutex<StreamInner>,
    started_at: Instant,
    started_at_wall_ms: i64,
}

fn wall_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Event stream of one assistant response; attaches `duration_ms` on terminal messages.
#[derive(Clone)]
pub struct AssistantMessageEventStream {
    shared: Arc<SharedStream>,
}

impl AssistantMessageEventStream {
    /// Create an empty assistant message event stream.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedStream {
                inner: Mutex::new(StreamInner {
                    queue: VecDeque::new(),
                    done: false,
                    result: None,
                    result_waiters: Vec::new(),
                    notify: Arc::new(Notify::new()),
                }),
                started_at: Instant::now(),
                started_at_wall_ms: wall_now_ms(),
            }),
        }
    }

    /// Push a normalized assistant event.
    pub fn push(&self, mut event: AssistantMessageEvent) {
        if let AssistantMessageEvent::Done { ref mut message, .. } = event {
            Self::time_message(message, self.shared.started_at, self.shared.started_at_wall_ms);
        } else if let AssistantMessageEvent::Error { ref mut error, .. } = event {
            Self::time_message(error, self.shared.started_at, self.shared.started_at_wall_ms);
        }

        let mut inner = self.shared.inner.lock().expect("assistant stream lock");
        if inner.done {
            return;
        }

        if event.is_terminal() {
            inner.done = true;
            let message = match &event {
                AssistantMessageEvent::Done { message, .. } => message.clone(),
                AssistantMessageEvent::Error { error, .. } => error.clone(),
                _ => unreachable!("terminal event"),
            };
            inner.result = Some(message.clone());
            for waiter in std::mem::take(&mut inner.result_waiters) {
                let _ = waiter.send(message.clone());
            }
        }

        inner.queue.push_back(event);
        inner.notify.notify_waiters();
    }

    /// Finish the stream, optionally supplying a final message.
    pub fn end(&self, result: Option<AssistantMessage>) {
        let mut inner = self.shared.inner.lock().expect("assistant stream lock");
        inner.done = true;
        if let Some(mut message) = result {
            Self::time_message(&mut message, self.shared.started_at, self.shared.started_at_wall_ms);
            inner.result = Some(message.clone());
            for waiter in inner.result_waiters.drain(..) {
                let _ = waiter.send(message.clone());
            }
        }
        inner.notify.notify_waiters();
    }

    /// Await the final assistant message (`done` or `error`).
    pub async fn result(&self) -> AssistantMessage {
        loop {
            {
                let inner = self.shared.inner.lock().expect("assistant stream lock");
                if let Some(ref result) = inner.result {
                    return result.clone();
                }
                if inner.done {
                    break;
                }
            }
            let notify = {
                self.shared
                    .inner
                    .lock()
                    .expect("assistant stream lock")
                    .notify
                    .clone()
            };
            notify.notified().await;
        }
        self.shared
            .inner
            .lock()
            .expect("assistant stream lock")
            .result
            .clone()
            .expect("stream ended without a result")
    }

    /// Iterate events as a `futures` stream.
    pub fn events(&self) -> impl Stream<Item = AssistantMessageEvent> + Send + Unpin {
        let stream = self.clone();
        Box::pin(stream! {
            loop {
                enum Poll {
                    Event(AssistantMessageEvent),
                    Wait(Arc<Notify>),
                    Done,
                }
                // Never hold the mutex across `.await` — `MutexGuard` is not `Send`.
                let poll = {
                    let mut inner = stream.shared.inner.lock().expect("assistant stream lock");
                    if let Some(event) = inner.queue.pop_front() {
                        Poll::Event(event)
                    } else if inner.done {
                        Poll::Done
                    } else {
                        Poll::Wait(inner.notify.clone())
                    }
                };
                match poll {
                    Poll::Event(event) => {
                        let terminal = event.is_terminal();
                        yield event;
                        if terminal {
                            break;
                        }
                    }
                    Poll::Wait(notify) => notify.notified().await,
                    Poll::Done => break,
                }
            }
        })
    }

    fn time_message(message: &mut AssistantMessage, started_at: Instant, started_at_wall_ms: i64) {
        if message.duration_ms.is_some() || message.timestamp < started_at_wall_ms {
            return;
        }
        message.duration_ms = Some(started_at.elapsed().as_millis() as u64);
    }
}

impl Default for AssistantMessageEventStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Factory for extensions and provider adapters.
pub fn create_assistant_message_event_stream() -> AssistantMessageEventStream {
    AssistantMessageEventStream::new()
}

struct EventStreamState<T, R> {
    queue: VecDeque<T>,
    done: bool,
    result: Option<R>,
    result_waiters: Vec<tokio::sync::oneshot::Sender<R>>,
    notify: Arc<Notify>,
    is_complete: fn(&T) -> bool,
    extract_result: fn(&T) -> R,
}

/// Generic event stream with a typed terminal result (mirrors pi `EventStream`).
pub struct EventStream<T, R> {
    inner: Arc<Mutex<EventStreamState<T, R>>>,
}

impl<T: Clone, R: Clone> EventStream<T, R> {
    /// Create a stream that completes on the first event for which `is_complete` returns true.
    pub fn new(is_complete: fn(&T) -> bool, extract_result: fn(&T) -> R) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventStreamState {
                queue: VecDeque::new(),
                done: false,
                result: None,
                result_waiters: Vec::new(),
                notify: Arc::new(Notify::new()),
                is_complete,
                extract_result,
            })),
        }
    }

    /// Push an event to consumers.
    pub fn push(&self, event: T) {
        let mut inner = self.inner.lock().expect("event stream lock");
        if inner.done {
            return;
        }
        if (inner.is_complete)(&event) {
            inner.done = true;
            let result = (inner.extract_result)(&event);
            inner.result = Some(result.clone());
            for waiter in std::mem::take(&mut inner.result_waiters) {
                let _ = waiter.send(result.clone());
            }
        }
        inner.queue.push_back(event);
        inner.notify.notify_waiters();
    }

    /// Mark complete and optionally set the result.
    pub fn end(&self, result: Option<R>) {
        let mut inner = self.inner.lock().expect("event stream lock");
        inner.done = true;
        if let Some(result) = result {
            inner.result = Some(result.clone());
            for waiter in std::mem::take(&mut inner.result_waiters) {
                let _ = waiter.send(result.clone());
            }
        }
        inner.notify.notify_waiters();
    }

    /// Await the extracted terminal result.
    pub async fn result(&self) -> R {
        loop {
            {
                let inner = self.inner.lock().expect("event stream lock");
                if let Some(ref result) = inner.result {
                    return result.clone();
                }
                if inner.done {
                    break;
                }
            }
            let notify = self.inner.lock().expect("event stream lock").notify.clone();
            notify.notified().await;
        }
        self.inner
            .lock()
            .expect("event stream lock")
            .result
            .clone()
            .expect("stream ended without a result")
    }
}
