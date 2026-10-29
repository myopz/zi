//! In-process telemetry recorder for tests and local diagnostics.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::metrics::{
    HistogramOptions, InstrumentOptions, TelemetryCounter, TelemetryHistogram, TelemetryMetrics,
    TelemetryUpDownCounter, attribute_eq,
};
use crate::types::{
    MetricKind, MetricQuery, SpanLink, SpanOptions, SpanStatus, StartSpanCallback, StoredMetric, StoredSpan,
    StoredSpanEvent, StoredSpanLink, StoredTrace, TelemetryCarrier, TelemetryContext, TelemetryPropagation,
    TelemetrySpan, TelemetryStore, TraceQuery,
};

const CARRIER_SCOPE: &str = "zi-memory-scope";
const CARRIER_SPAN: &str = "zi-memory-span";
const CARRIER_TRACE: &str = "zi-memory-trace";

#[derive(Clone, Debug)]
pub struct RecordedTelemetryEvent {
    pub name: String,
    pub attributes: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct RecordedTelemetryLink {
    pub context: TelemetryCarrier,
    pub attributes: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct RecordedTelemetrySpan {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub context: TelemetryCarrier,
    pub parent_context: Option<TelemetryCarrier>,
    pub links: Vec<RecordedTelemetryLink>,
    pub name: String,
    pub attributes: HashMap<String, serde_json::Value>,
    pub events: Vec<RecordedTelemetryEvent>,
    pub status: SpanStatus,
    pub settled: bool,
    pub end_sequence: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RecordedTelemetryMeasurement {
    pub kind: MetricKind,
    pub name: String,
    pub description: Option<String>,
    pub unit: Option<String>,
    pub boundaries: Option<Vec<f64>>,
    pub value: f64,
    pub attributes: HashMap<String, serde_json::Value>,
    pub context: Option<TelemetryCarrier>,
}

struct MutableSpan {
    started_ms: u64,
    clock_start: f64,
    ended_ms: Option<u64>,
    id: u32,
    parent_id: Option<u32>,
    context: TelemetryCarrier,
    parent_context: Option<TelemetryCarrier>,
    links: Vec<RecordedTelemetryLink>,
    name: String,
    attributes: HashMap<String, serde_json::Value>,
    events: Vec<(f64, RecordedTelemetryEvent)>,
    status: SpanStatus,
    explicit_status: bool,
    settled: bool,
    end_sequence: Option<u32>,
}

struct InMemoryState {
    scope: String,
    spans: Vec<MutableSpan>,
    measurements: Vec<(u64, RecordedTelemetryMeasurement)>,
    next_span_id: u32,
    next_end_sequence: u32,
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<InMemoryState>>,
}

fn copy_attributes(attributes: Option<HashMap<String, serde_json::Value>>) -> HashMap<String, serde_json::Value> {
    attributes.unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn monotonic_ms() -> f64 {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let base = *START.get_or_init(now_ms);
    (now_ms() as f64) - (base as f64)
}

fn carrier_id(carrier: &TelemetryCarrier) -> String {
    format!("{}:{}", carrier[CARRIER_SCOPE], carrier[CARRIER_SPAN])
}

/// Backend-neutral reference implementation that records spans in process memory.
#[derive(Clone)]
pub struct InMemoryTelemetryContext {
    shared: Shared,
}

impl Default for InMemoryTelemetryContext {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryTelemetryContext {
    pub fn new() -> Self {
        Self {
            shared: Shared {
                state: Arc::new(Mutex::new(InMemoryState {
                    scope: Uuid::now_v7().to_string(),
                    spans: Vec::new(),
                    measurements: Vec::new(),
                    next_span_id: 1,
                    next_end_sequence: 1,
                })),
            },
        }
    }

    pub fn get_spans(&self) -> Vec<RecordedTelemetrySpan> {
        let state = self.shared.state.lock().expect("telemetry lock");
        state
            .spans
            .iter()
            .map(|span| RecordedTelemetrySpan {
                id: span.id,
                parent_id: span.parent_id,
                context: span.context.clone(),
                parent_context: span.parent_context.clone(),
                links: span.links.clone(),
                name: span.name.clone(),
                attributes: span.attributes.clone(),
                events: span.events.iter().map(|(_, e)| e.clone()).collect(),
                status: span.status.clone(),
                settled: span.settled,
                end_sequence: span.end_sequence,
            })
            .collect()
    }

    pub fn get_metrics(&self) -> Vec<RecordedTelemetryMeasurement> {
        let state = self.shared.state.lock().expect("telemetry lock");
        state.measurements.iter().map(|(_, m)| m.clone()).collect()
    }

    fn record_measurement(
        shared: &Shared,
        kind: MetricKind,
        name: &str,
        instrument: Option<InstrumentOptions>,
        histogram: Option<HistogramOptions>,
        value: f64,
        attributes: Option<HashMap<String, serde_json::Value>>,
        context: Option<TelemetryCarrier>,
    ) {
        let mut state = shared.state.lock().expect("telemetry lock");
        let interval_ms = (now_ms() / 10_000) * 10_000;
        let (description, unit) = match (&instrument, &histogram) {
            (_, Some(h)) => (h.description.clone(), h.unit.clone()),
            (Some(i), None) => (i.description.clone(), i.unit.clone()),
            _ => (None, None),
        };
        let boundaries = histogram.and_then(|h| h.boundaries);
        state.measurements.push((
            interval_ms,
            RecordedTelemetryMeasurement {
                kind,
                name: name.to_string(),
                description,
                unit,
                boundaries,
                value,
                attributes: copy_attributes(attributes),
                context,
            },
        ));
    }

    fn stored_spans(&self) -> Vec<StoredSpan> {
        let state = self.shared.state.lock().expect("telemetry lock");
        let mut out: Vec<StoredSpan> = state
            .spans
            .iter()
            .filter(|s| s.settled)
            .map(|span| StoredSpan {
                id: carrier_id(&span.context),
                trace_id: span.context[CARRIER_TRACE].clone(),
                parent_id: span.parent_context.as_ref().map(carrier_id),
                name: span.name.clone(),
                started_ms: span.started_ms,
                ended_ms: span.ended_ms.unwrap_or(span.started_ms),
                attributes: span.attributes.clone(),
                events: span
                    .events
                    .iter()
                    .map(|(time_ms, event)| StoredSpanEvent {
                        name: event.name.clone(),
                        time_ms: *time_ms,
                        attributes: event.attributes.clone(),
                    })
                    .collect(),
                links: span
                    .links
                    .iter()
                    .map(|link| StoredSpanLink {
                        trace_id: link.context.get(CARRIER_TRACE).cloned().unwrap_or_default(),
                        span_id: carrier_id(&link.context),
                        attributes: Some(link.attributes.clone()),
                    })
                    .collect(),
                status: span.status.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
        out
    }

    async fn start_span_with_parent(
        shared: Shared,
        parent_id: Option<u32>,
        parent_context: Option<TelemetryCarrier>,
        options: SpanOptions,
        callback: StartSpanCallback,
    ) -> Box<dyn Any + Send> {
        let (span_id, context) = {
            let mut state = shared.state.lock().expect("telemetry lock");
            let span_id = state.next_span_id;
            state.next_span_id += 1;
            let trace_id = parent_context
                .as_ref()
                .and_then(|c| c.get(CARRIER_TRACE).cloned())
                .unwrap_or_else(|| format!("{}:{}", state.scope, span_id));
            let context = HashMap::from([
                (CARRIER_SCOPE.to_string(), state.scope.clone()),
                (CARRIER_SPAN.to_string(), span_id.to_string()),
                (CARRIER_TRACE.to_string(), trace_id),
            ]);
            state.spans.push(MutableSpan {
                started_ms: now_ms(),
                clock_start: monotonic_ms(),
                ended_ms: None,
                id: span_id,
                parent_id,
                context: context.clone(),
                parent_context: parent_context.clone(),
                links: Vec::new(),
                name: options.name.clone(),
                attributes: copy_attributes(options.attributes.clone()),
                events: Vec::new(),
                status: SpanStatus::Ok,
                explicit_status: false,
                settled: false,
                end_sequence: None,
            });
            (span_id, context)
        };

        let recorder = InMemoryTelemetryContext {
            shared: shared.clone(),
        };
        let span: Box<dyn TelemetrySpan + Send> = Box::new(ActiveSpan {
            recorder: recorder.clone(),
            shared: shared.clone(),
            span_id,
            context,
        });

        let output = callback(span).await;
        Self::finish_span(&shared, span_id);
        output
    }

    fn finish_span(shared: &Shared, span_id: u32) {
        let mut state = shared.state.lock().expect("telemetry lock");
        let seq = state.next_end_sequence;
        let found = if let Some(span) = state.spans.iter_mut().find(|s| s.id == span_id) {
            span.settled = true;
            span.ended_ms = Some(span.started_ms + (monotonic_ms() - span.clock_start) as u64);
            span.end_sequence = Some(seq);
            true
        } else {
            false
        };
        if found {
            state.next_end_sequence += 1;
        }
    }
}

struct ActiveSpan {
    recorder: InMemoryTelemetryContext,
    shared: Shared,
    span_id: u32,
    context: TelemetryCarrier,
}

#[async_trait::async_trait]
impl TelemetryContext for InMemoryTelemetryContext {
    fn metrics(&self) -> Option<&dyn TelemetryMetrics> {
        Some(self)
    }
    fn propagation(&self) -> Option<&dyn TelemetryPropagation> {
        Some(self)
    }
    async fn start_span_erased(&self, options: SpanOptions, callback: StartSpanCallback) -> Box<dyn Any + Send> {
        Self::start_span_with_parent(self.shared.clone(), None, None, options, callback).await
    }
}

#[async_trait::async_trait]
impl TelemetryContext for ActiveSpan {
    fn metrics(&self) -> Option<&dyn TelemetryMetrics> {
        Some(&self.recorder)
    }
    fn propagation(&self) -> Option<&dyn TelemetryPropagation> {
        Some(&self.recorder)
    }
    async fn start_span_erased(&self, options: SpanOptions, callback: StartSpanCallback) -> Box<dyn Any + Send> {
        InMemoryTelemetryContext::start_span_with_parent(
            self.shared.clone(),
            Some(self.span_id),
            Some(self.context.clone()),
            options,
            callback,
        )
        .await
    }
}

impl TelemetrySpan for ActiveSpan {
    fn add_link(&mut self, _link: SpanLink) {}
    fn add_event(&mut self, name: &str, attributes: Option<HashMap<String, serde_json::Value>>) {
        let mut state = self.shared.state.lock().expect("telemetry lock");
        if let Some(span) = state.spans.iter_mut().find(|s| s.id == self.span_id && !s.settled) {
            span.events.push((
                monotonic_ms() - span.clock_start,
                RecordedTelemetryEvent {
                    name: name.to_string(),
                    attributes: copy_attributes(attributes),
                },
            ));
        }
    }
    fn set_attributes(&mut self, attributes: HashMap<String, serde_json::Value>) {
        let mut state = self.shared.state.lock().expect("telemetry lock");
        if let Some(span) = state.spans.iter_mut().find(|s| s.id == self.span_id && !s.settled) {
            span.attributes.extend(attributes);
        }
    }
    fn set_status(&mut self, status: SpanStatus) {
        let mut state = self.shared.state.lock().expect("telemetry lock");
        if let Some(span) = state.spans.iter_mut().find(|s| s.id == self.span_id && !s.settled) {
            span.status = status;
            span.explicit_status = true;
        }
    }
}

impl TelemetryMetrics for InMemoryTelemetryContext {
    fn counter(&self, name: &str, options: Option<InstrumentOptions>) -> Box<dyn TelemetryCounter> {
        let shared = self.shared.clone();
        let name = name.to_string();
        Box::new(CounterRecorder {
            shared,
            name,
            options,
        })
    }
    fn up_down_counter(&self, name: &str, options: Option<InstrumentOptions>) -> Box<dyn TelemetryUpDownCounter> {
        let shared = self.shared.clone();
        let name = name.to_string();
        Box::new(UpDownRecorder {
            shared,
            name,
            options,
        })
    }
    fn histogram(&self, name: &str, options: Option<HistogramOptions>) -> Box<dyn TelemetryHistogram> {
        let shared = self.shared.clone();
        let name = name.to_string();
        Box::new(HistogramRecorder {
            shared,
            name,
            options,
        })
    }
}

struct CounterRecorder {
    shared: Shared,
    name: String,
    options: Option<InstrumentOptions>,
}

impl TelemetryCounter for CounterRecorder {
    fn add(&self, value: f64, attributes: Option<HashMap<String, serde_json::Value>>) {
        InMemoryTelemetryContext::record_measurement(
            &self.shared,
            MetricKind::Counter,
            &self.name,
            self.options.clone(),
            None,
            value,
            attributes,
            None,
        );
    }
}

struct UpDownRecorder {
    shared: Shared,
    name: String,
    options: Option<InstrumentOptions>,
}

impl TelemetryUpDownCounter for UpDownRecorder {
    fn add(&self, value: f64, attributes: Option<HashMap<String, serde_json::Value>>) {
        InMemoryTelemetryContext::record_measurement(
            &self.shared,
            MetricKind::UpDownCounter,
            &self.name,
            self.options.clone(),
            None,
            value,
            attributes,
            None,
        );
    }
}

struct HistogramRecorder {
    shared: Shared,
    name: String,
    options: Option<HistogramOptions>,
}

impl TelemetryHistogram for HistogramRecorder {
    fn record(
        &self,
        value: f64,
        attributes: Option<HashMap<String, serde_json::Value>>,
        _context: Option<&dyn TelemetryContext>,
    ) {
        InMemoryTelemetryContext::record_measurement(
            &self.shared,
            MetricKind::Histogram,
            &self.name,
            None,
            self.options.clone(),
            value,
            attributes,
            None,
        );
    }
}

#[async_trait::async_trait]
impl TelemetryPropagation for InMemoryTelemetryContext {
    async fn inject(&self, _context: &dyn TelemetryContext) -> Option<TelemetryCarrier> {
        None
    }
    async fn extract(&self, carrier: Option<&TelemetryCarrier>) -> Box<dyn TelemetryContext> {
        let scope = carrier.and_then(|c| c.get(CARRIER_SCOPE));
        let span = carrier.and_then(|c| c.get(CARRIER_SPAN));
        let trace = carrier.and_then(|c| c.get(CARRIER_TRACE));
        if scope.is_some() && span.is_some() && trace.is_some() {
            return Box::new(InMemoryTelemetryContext {
                shared: self.shared.clone(),
            });
        }
        Box::new(InMemoryTelemetryContext {
            shared: self.shared.clone(),
        })
    }
}

#[async_trait::async_trait]
impl TelemetryStore for InMemoryTelemetryContext {
    async fn find_traces(&self, query: TraceQuery) -> (Vec<StoredTrace>, Option<String>) {
        let limit = query.limit.unwrap_or(20);
        let mut grouped: HashMap<String, Vec<StoredSpan>> = HashMap::new();
        for span in self.stored_spans() {
            grouped.entry(span.trace_id.clone()).or_default().push(span);
        }
        let mut traces: Vec<StoredTrace> = grouped
            .into_iter()
            .filter(|(_, spans)| {
                let first = &spans[0];
                query.name.as_ref().map(|n| &first.name == n).unwrap_or(true)
                    && query
                        .start_min
                        .map(|min| first.started_ms >= min)
                        .unwrap_or(true)
                    && query
                        .start_max
                        .map(|max| first.started_ms <= max)
                        .unwrap_or(true)
                    && (!query.errors || spans.iter().any(|s| matches!(s.status, SpanStatus::Error { .. })))
                    && query.attributes.as_ref().map(|attrs| {
                        spans.iter().any(|span| {
                            attrs
                                .iter()
                                .all(|(key, value)| attribute_eq(&span.attributes, key, value))
                        })
                    }).unwrap_or(true)
            })
            .map(|(trace_id, spans)| StoredTrace { trace_id, spans })
            .collect();
        traces.sort_by(|a, b| {
            b.spans[0]
                .started_ms
                .cmp(&a.spans[0].started_ms)
                .then_with(|| a.trace_id.cmp(&b.trace_id))
        });
        let page: Vec<StoredTrace> = traces.into_iter().take(limit).collect();
        (page, None)
    }

    async fn get_trace(&self, trace_id: &str) -> Option<StoredTrace> {
        let spans: Vec<StoredSpan> = self
            .stored_spans()
            .into_iter()
            .filter(|s| s.trace_id == trace_id)
            .collect();
        if spans.is_empty() {
            None
        } else {
            Some(StoredTrace {
                trace_id: trace_id.to_string(),
                spans,
            })
        }
    }

    async fn find_metrics(&self, query: MetricQuery) -> Vec<StoredMetric> {
        let state = self.shared.state.lock().expect("telemetry lock");
        state
            .measurements
            .iter()
            .filter(|(interval_ms, measurement)| {
                measurement.name == query.name
                    && query
                        .start_min
                        .map(|min| *interval_ms >= min)
                        .unwrap_or(true)
                    && query
                        .start_max
                        .map(|max| *interval_ms <= max)
                        .unwrap_or(true)
                    && query.attributes.as_ref().map(|attrs| {
                        attrs
                            .iter()
                            .all(|(key, value)| attribute_eq(&measurement.attributes, key, value))
                    }).unwrap_or(true)
            })
            .map(|(interval_ms, measurement)| StoredMetric {
                interval_ms: *interval_ms,
                name: measurement.name.clone(),
                kind: measurement.kind,
                unit: measurement.unit.clone(),
                attributes: measurement.attributes.clone(),
                count: 1,
                sum: measurement.value,
                min: measurement.value,
                max: measurement.value,
            })
            .collect()
    }
}
