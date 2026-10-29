// createHost, load, get, link dispatch (design §3 low-level host API subset).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use zi_telemetry::{NoopTelemetryContext, TelemetryContext};
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tracing::instrument;
use uuid::Uuid;

use super::ctx::{CallContext, HostContext};
use super::object::LiveObject;
use super::reference::Reference;
use crate::contracts::contract::{ContractKind, ContractSpec};
use crate::contracts::facet::{set_up_facet, PluginSpec, ServeOptions};
use crate::contracts::host_kind::Address;
use crate::data::identity::identity_key;
use crate::data::Principal;
use crate::error::{ChordError, ChordResult};
use crate::link::{Link, LinkEnvelope, LinkMessage, MethodCall, MethodResponse};
use crate::store::InMemoryStore;

#[derive(Clone)]
pub struct HostOptions {
    pub address: Option<Address>,
    pub principal: Option<Principal>,
    pub on_error: Arc<dyn Fn(HostError) + Send + Sync>,
    pub telemetry: Arc<dyn TelemetryContext>,
    pub store: Option<InMemoryStore>,
}

impl HostOptions {
    pub fn client(on_error: impl Fn(HostError) + Send + Sync + 'static) -> Self {
        Self {
            address: None,
            principal: None,
            on_error: Arc::new(on_error),
            telemetry: Arc::new(NoopTelemetryContext),
            store: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum HostError {
    Background(String),
    Protocol(String),
    Storage(String),
}

pub struct HostHandle {
    inner: Arc<HostInner>,
}

pub struct Host {
    inner: Arc<HostInner>,
}

struct HostInner {
    id: Uuid,
    #[allow(dead_code)]
    options: HostOptions,
    served: RwLock<HashMap<String, (Arc<ContractSpec>, crate::host::object::ObjectFactory, ServeOptions)>>,
    objects: RwLock<HashMap<String, Arc<LiveObject>>>,
    pending_calls: RwLock<HashMap<Uuid, oneshot::Sender<ChordResult<Value>>>>,
    link: RwLock<Option<Arc<dyn Link>>>,
}

impl Host {
    pub fn id(&self) -> Uuid {
        self.inner.id
    }

    pub async fn load(&self, plugins: Vec<PluginSpec>, ctx: CallContext) -> ChordResult<()> {
        let _ = ctx;
        let mut served = self.inner.served.write();
        for plugin in plugins {
            for (spec, factory, options) in set_up_facet(&plugin.name, &plugin.facet) {
                served.insert(spec.id.name.clone(), (spec, factory, options));
            }
        }
        // TODO(slice 1): activate empty-params local objects before load resolves (start barrier).
        Ok(())
    }

    #[instrument(skip(self, ctx))]
    pub async fn get(
        &self,
        contract_name: &str,
        params: Map<String, Value>,
        ctx: CallContext,
    ) -> ChordResult<Reference> {
        let _ = ctx;
        // Drop the served lock before any `.await` — parking_lot guards are not `Send`.
        let (kind, factory) = {
            let served = self.inner.served.read();
            let (spec, factory, _options) = served
                .get(contract_name)
                .ok_or_else(|| ChordError::invalid("contract", format!("type `{contract_name}` not served")))?;
            spec.check_params(&params)?;
            (spec.kind, factory.clone())
        };
        if kind == ContractKind::Remote {
            return self.get_remote(contract_name, params).await;
        }
        let key = identity_key(contract_name, &params);
        if let Some(obj) = self.inner.objects.read().get(&key).cloned() {
            return Reference::new(obj);
        }
        let host_ctx = HostContext {
            call: CallContext::background(),
            host_id: self.inner.id,
            store_owner_prefix: Arc::from(format!("host:{}", self.inner.id)),
        };
        let live = factory(host_ctx, params.clone());
        let obj = Arc::new(live);
        self.inner.objects.write().insert(key, obj.clone());
        Reference::new(obj)
    }

    async fn get_remote(&self, contract_name: &str, params: Map<String, Value>) -> ChordResult<Reference> {
        let link = self
            .inner
            .link
            .read()
            .clone()
            .ok_or_else(|| ChordError::unavailable("link", "no link for remote get"))?;
        let call_id = Uuid::now_v7();
        let (tx, rx) = oneshot::channel();
        self.inner.pending_calls.write().insert(call_id, tx);
        let envelope = LinkEnvelope {
            id: call_id,
            body: LinkMessage::MethodCall(MethodCall {
                contract: contract_name.to_string(),
                params: Value::Object(params),
                method: "__hold__".to_string(),
                args: Value::Null,
            }),
        };
        link.send(envelope.to_bytes().map_err(|e| ChordError::invalid("wire", e.to_string()))?);
        let _value = rx
            .await
            .map_err(|_| ChordError::unavailable("link", "call dropped"))??;
        // TODO(slice 3a): hydrate remote reference from hold response instead of local stub.
        Err(ChordError::unavailable(
            "remote",
            "remote references not implemented in this skeleton",
        ))
    }

    pub async fn call_local(
        &self,
        contract_name: &str,
        params: Map<String, Value>,
        method: &str,
        args: Value,
        ctx: CallContext,
    ) -> ChordResult<Value> {
        let reference = self.get(contract_name, params, ctx.clone()).await?;
        let result = reference.call(method, args, ctx).await;
        let _ = reference.release();
        result
    }

    /// Invokes a method on a peer host over the attached link (slice 3a wire subset).
    pub async fn call_over_link(
        &self,
        contract_name: &str,
        params: Map<String, Value>,
        method: &str,
        args: Value,
    ) -> ChordResult<Value> {
        let link = self
            .inner
            .link
            .read()
            .clone()
            .ok_or_else(|| ChordError::unavailable("link", "no link"))?;
        let call_id = Uuid::now_v7();
        let (tx, rx) = oneshot::channel();
        self.inner.pending_calls.write().insert(call_id, tx);
        let envelope = LinkEnvelope {
            id: call_id,
            body: LinkMessage::MethodCall(MethodCall {
                contract: contract_name.to_string(),
                params: Value::Object(params),
                method: method.to_string(),
                args,
            }),
        };
        link.send(envelope.to_bytes().map_err(|e| ChordError::invalid("wire", e.to_string()))?);
        rx.await
            .map_err(|_| ChordError::unavailable("link", "call dropped"))?
    }

    pub fn attach_link(&self, link: Arc<dyn Link>) {
        *self.inner.link.write() = Some(link.clone());
        let inner = self.inner.clone();
        link.on_message(Box::new(move |bytes| {
            if let Ok(envelope) = LinkEnvelope::from_bytes(&bytes) {
                let inner = inner.clone();
                tokio::spawn(async move {
                    if let Err(err) = inner.dispatch_envelope(envelope).await {
                        (inner.options.on_error)(HostError::Protocol(err.to_string()));
                    }
                });
            }
        }));
    }
}

impl HostInner {
    async fn dispatch_envelope(self: &Arc<Self>, envelope: LinkEnvelope) -> ChordResult<()> {
        match envelope.body {
            LinkMessage::MethodCall(call) => {
                let host = Host { inner: self.clone() };
                let response = host.handle_method_call(envelope.id, call).await;
                if let Some(link) = self.link.read().clone() {
                    let body = match response {
                        Ok(ok) => LinkMessage::MethodResponse(MethodResponse {
                            call_id: envelope.id,
                            ok: Some(ok),
                            error: None,
                        }),
                        Err(err) => LinkMessage::MethodResponse(MethodResponse {
                            call_id: envelope.id,
                            ok: None,
                            error: Some(err.message),
                        }),
                    };
                    link.send(LinkEnvelope::new(body).to_bytes().unwrap());
                }
            }
            LinkMessage::MethodResponse(resp) => {
                if let Some(tx) = self.pending_calls.write().remove(&resp.call_id) {
                    let result = match (resp.ok, resp.error) {
                        (Some(v), None) => Ok(v),
                        (None, Some(msg)) => Err(ChordError::broken("call", msg)),
                        _ => Err(ChordError::broken("call", "invalid method response")),
                    };
                    let _ = tx.send(result);
                }
            }
            LinkMessage::StatePush { .. } => {
                // TODO(slice 3a): apply replicated state ops to watchables.
            }
        }
        Ok(())
    }
}

impl Host {
    async fn handle_method_call(&self, _id: Uuid, call: MethodCall) -> ChordResult<Value> {
        let params = match call.params {
            Value::Object(map) => map,
            _ => {
                return Err(ChordError::invalid("params", "expected object"));
            }
        };
        if call.method == "__hold__" {
            let reference = self
                .get(&call.contract, params, CallContext::background())
                .await?;
            let _ = reference.release();
            return Ok(Value::String("held".into()));
        }
        self.call_local(&call.contract, params, &call.method, call.args, CallContext::background())
            .await
    }
}

/// Creates a host (slice 1 subset).
pub fn create_host(options: HostOptions) -> HostHandle {
    let inner = Arc::new(HostInner {
        id: Uuid::now_v7(),
        options,
        served: RwLock::new(HashMap::new()),
        objects: RwLock::new(HashMap::new()),
        pending_calls: RwLock::new(HashMap::new()),
        link: RwLock::new(None),
    });
    HostHandle {
        inner: inner.clone(),
    }
}

impl HostHandle {
    pub fn host(&self) -> Host {
        Host {
            inner: self.inner.clone(),
        }
    }
}

/// Connects two hosts with an in-memory link and installs message handlers.
pub fn connect_hosts(a: &Host, b: &Host) -> (Arc<dyn Link>, Arc<dyn Link>) {
    let (link_a, link_b) = crate::link::memory_link_pair();
    a.attach_link(link_a.clone());
    b.attach_link(link_b.clone());
    (link_a, link_b)
}
