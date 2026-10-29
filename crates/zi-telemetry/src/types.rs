//! Core telemetry types aligned with the reference TypeScript contract.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::metrics::TelemetryMetrics;

pub type AttributeValue = serde_json::Value;

pub type SpanAttributes = HashMap<String, AttributeValue>;

/// Opaque, JSON-serializable adapter-owned propagation data.
pub type TelemetryCarrier = HashMap<String, String>;

/// Object-safe callback used by [`TelemetryContext::start_span_erased`].
pub type StartSpanCallback = Box<
    dyn FnOnce(Box<dyn TelemetrySpan + Send>) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>>
        + Send,
>;

#[async_trait::async_trait]
pub trait TelemetryPropagation: Send + Sync {
    async fn inject(&self, context: &dyn TelemetryContext) -> Option<TelemetryCarrier>;
    async fn extract(&self, carrier: Option<&TelemetryCarrier>) -> Box<dyn TelemetryContext>;
}

pub struct SpanLink {
    pub context: Box<dyn TelemetryContext>,
    pub attributes: Option<SpanAttributes>,
}

pub struct SpanOptions {
    pub name: String,
    pub attributes: Option<SpanAttributes>,
    pub links: Option<Vec<SpanLink>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpanStatus {
    Ok,
    Error { name: String, message: String },
}

#[derive(Clone, Debug)]
pub struct StoredSpan {
    pub id: String,
    pub trace_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub attributes: SpanAttributes,
    pub events: Vec<StoredSpanEvent>,
    pub links: Vec<StoredSpanLink>,
    pub status: SpanStatus,
}

#[derive(Clone, Debug)]
pub struct StoredSpanEvent {
    pub name: String,
    pub time_ms: f64,
    pub attributes: SpanAttributes,
}

#[derive(Clone, Debug)]
pub struct StoredSpanLink {
    pub trace_id: String,
    pub span_id: String,
    pub attributes: Option<SpanAttributes>,
}

#[derive(Clone, Debug)]
pub struct StoredTrace {
    pub trace_id: String,
    pub spans: Vec<StoredSpan>,
}

#[derive(Clone, Debug, Default)]
pub struct TraceQuery {
    pub name: Option<String>,
    pub attributes: Option<HashMap<String, AttributeValue>>,
    pub errors: bool,
    pub start_min: Option<u64>,
    pub start_max: Option<u64>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MetricQuery {
    pub name: String,
    pub start_min: Option<u64>,
    pub start_max: Option<u64>,
    pub attributes: Option<HashMap<String, AttributeValue>>,
}

#[derive(Clone, Debug)]
pub struct StoredMetric {
    pub interval_ms: u64,
    pub name: String,
    pub kind: MetricKind,
    pub unit: Option<String>,
    pub attributes: SpanAttributes,
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    Counter,
    UpDownCounter,
    Histogram,
}

#[async_trait::async_trait]
pub trait TelemetryStore: Send + Sync {
    async fn find_traces(&self, query: TraceQuery) -> (Vec<StoredTrace>, Option<String>);
    async fn get_trace(&self, trace_id: &str) -> Option<StoredTrace>;
    async fn find_metrics(&self, query: MetricQuery) -> Vec<StoredMetric>;
}

pub trait TelemetrySpan: TelemetryContext + Send + Sync {
    fn add_link(&mut self, link: SpanLink);
    fn add_event(&mut self, name: &str, attributes: Option<SpanAttributes>);
    fn set_attributes(&mut self, attributes: SpanAttributes);
    fn set_status(&mut self, status: SpanStatus);
}

/// Object-safe telemetry context. Prefer [`TelemetryContextExt::start_span`] for typed returns.
#[async_trait::async_trait]
pub trait TelemetryContext: Send + Sync {
    fn metrics(&self) -> Option<&dyn TelemetryMetrics>;
    fn propagation(&self) -> Option<&dyn TelemetryPropagation>;
    async fn start_span_erased(&self, options: SpanOptions, callback: StartSpanCallback) -> Box<dyn Any + Send>;
}

/// Ergonomic typed wrapper around [`TelemetryContext::start_span_erased`].
pub trait TelemetryContextExt: TelemetryContext {
    fn start_span<'a, T, F, Fut>(
        &'a self,
        options: SpanOptions,
        callback: F,
    ) -> Pin<Box<dyn Future<Output = T> + Send + 'a>>
    where
        T: Send + 'static,
        F: FnOnce(Box<dyn TelemetrySpan + Send>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let erased = self.start_span_erased(
            options,
            Box::new(move |span| {
                Box::pin(async move {
                    let value = callback(span).await;
                    Box::new(value) as Box<dyn Any + Send>
                })
            }),
        );
        Box::pin(async move {
            erased
                .await
                .downcast::<T>()
                .map(|value| *value)
                .expect("zi-telemetry: start_span type mismatch")
        })
    }
}

impl<C: TelemetryContext + ?Sized> TelemetryContextExt for C {}
