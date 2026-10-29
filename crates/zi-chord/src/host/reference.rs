// References returned from host.get (design §7).

use std::sync::Arc;

use serde_json::Value;

use super::ctx::CallContext;
use super::object::LiveObject;
use crate::error::ChordResult;
use crate::state::watchable::Watchable;

#[derive(Clone)]
pub struct Reference {
    object: Arc<LiveObject>,
}

impl Reference {
    pub(crate) fn new(object: Arc<LiveObject>) -> ChordResult<Self> {
        object.add_ref()?;
        Ok(Self { object })
    }

    pub fn state(&self, name: &str) -> ChordResult<Watchable<Value>> {
        self.object.get_state(name)
    }

    pub async fn call(&self, method: &str, args: Value, ctx: CallContext) -> ChordResult<Value> {
        self.object.call_method(method, args, ctx).await
    }

    pub fn release(&self) -> ChordResult<()> {
        self.object.release()
    }
}
