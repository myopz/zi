// Live object implementations behind references.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::{Map, Value};

use super::ctx::{CallContext, HostContext};
use crate::contracts::contract::ContractSpec;
use crate::error::{ChordError, ChordResult};
use crate::state::watchable::Watchable;

pub enum ObjectMemberKind {
    State(Watchable<Value>),
    Method(ObjectMethod),
}

#[derive(Clone)]
pub struct ObjectMethod {
    invoke: Arc<dyn Fn(Value, CallContext) -> MethodFuture + Send + Sync>,
}

type MethodFuture = Pin<Box<dyn Future<Output = ChordResult<Value>> + Send>>;

impl ObjectMethod {
    pub fn from_fn<F, Fut>(f: F) -> Self
    where
        F: Fn(Value, CallContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ChordResult<Value>> + Send + 'static,
    {
        Self {
            invoke: Arc::new(move |args, ctx| Box::pin(f(args, ctx))),
        }
    }

    pub async fn call(&self, args: Value, ctx: CallContext) -> ChordResult<Value> {
        (self.invoke)(args, ctx).await
    }
}

pub struct LiveObject {
    pub spec: Arc<ContractSpec>,
    pub params: Map<String, Value>,
    members: RwLock<HashMap<String, ObjectMemberKind>>,
    refs: RwLock<u32>,
    released: RwLock<bool>,
}

impl LiveObject {
    pub fn new(spec: Arc<ContractSpec>, params: Map<String, Value>, members: HashMap<String, ObjectMemberKind>) -> Self {
        Self {
            spec,
            params,
            members: RwLock::new(members),
            refs: RwLock::new(0),
            released: RwLock::new(false),
        }
    }

    pub fn add_ref(&self) -> ChordResult<()> {
        if *self.released.read() {
            return Err(ChordError::released("object ended"));
        }
        *self.refs.write() += 1;
        Ok(())
    }

    pub fn release(&self) -> ChordResult<()> {
        let mut refs = self.refs.write();
        if *refs == 0 {
            return Err(ChordError::released("object not held"));
        }
        *refs -= 1;
        if *refs == 0 {
            *self.released.write() = true;
            let members = self.members.read();
            for member in members.values() {
                if let ObjectMemberKind::State(w) = member {
                    w.release();
                }
            }
        }
        Ok(())
    }

    pub fn get_state(&self, name: &str) -> ChordResult<Watchable<Value>> {
        if *self.released.read() {
            return Err(ChordError::released("object released"));
        }
        let members = self.members.read();
        match members.get(name) {
            Some(ObjectMemberKind::State(w)) => Ok(w.clone()),
            Some(_) => Err(ChordError::invalid("member", format!("`{name}` is not state"))),
            None => Err(ChordError::invalid("member", format!("unknown state `{name}`"))),
        }
    }

    pub async fn call_method(&self, name: &str, args: Value, ctx: CallContext) -> ChordResult<Value> {
        if *self.released.read() {
            return Err(ChordError::released("object released"));
        }
        // Clone the method under the lock, then await without holding a parking_lot guard.
        let method = {
            let members = self.members.read();
            match members.get(name) {
                Some(ObjectMemberKind::Method(m)) => m.clone(),
                Some(_) => {
                    return Err(ChordError::invalid("member", format!("`{name}` is not a method")));
                }
                None => {
                    return Err(ChordError::invalid("member", format!("unknown method `{name}`")));
                }
            }
        };
        method.call(args, ctx).await
    }
}

/// Factory invoked on first `get` for an identity (local) or on remote hold (slice 3a).
pub type ObjectFactory = Arc<dyn Fn(HostContext, Map<String, Value>) -> LiveObject + Send + Sync>;

pub fn factory_sync<F>(f: F) -> ObjectFactory
where
    F: Fn(HostContext, Map<String, Value>) -> LiveObject + Send + Sync + 'static,
{
    Arc::new(f)
}

pub fn factory_async<F, Fut>(f: F) -> ObjectFactory
where
    F: Fn(HostContext, Map<String, Value>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = LiveObject> + Send + 'static,
{
    Arc::new(move |ctx, params| {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f(ctx, params)))
    })
}
