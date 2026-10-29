//! Shared NOOP telemetry context and instruments.

use std::any::Any;

use crate::metrics::{
    HistogramOptions, InstrumentOptions, TelemetryCounter, TelemetryHistogram, TelemetryMetrics,
    TelemetryUpDownCounter,
};
use crate::types::{
    SpanLink, SpanOptions, SpanStatus, StartSpanCallback, TelemetryCarrier, TelemetryContext,
    TelemetryPropagation, TelemetrySpan,
};

struct NoopCounter;
impl TelemetryCounter for NoopCounter {
    fn add(&self, _value: f64, _attributes: Option<crate::types::SpanAttributes>) {}
}

struct NoopUpDownCounter;
impl TelemetryUpDownCounter for NoopUpDownCounter {
    fn add(&self, _value: f64, _attributes: Option<crate::types::SpanAttributes>) {}
}

struct NoopHistogram;
impl TelemetryHistogram for NoopHistogram {
    fn record(
        &self,
        _value: f64,
        _attributes: Option<crate::types::SpanAttributes>,
        _context: Option<&dyn TelemetryContext>,
    ) {
    }
}

pub struct NoopMetrics;
impl TelemetryMetrics for NoopMetrics {
    fn counter(&self, _name: &str, _options: Option<InstrumentOptions>) -> Box<dyn TelemetryCounter> {
        Box::new(NoopCounter)
    }
    fn up_down_counter(&self, _name: &str, _options: Option<InstrumentOptions>) -> Box<dyn TelemetryUpDownCounter> {
        Box::new(NoopUpDownCounter)
    }
    fn histogram(&self, _name: &str, _options: Option<HistogramOptions>) -> Box<dyn TelemetryHistogram> {
        Box::new(NoopHistogram)
    }
}

pub static NOOP_TELEMETRY_METRICS: NoopMetrics = NoopMetrics;

pub struct NoopPropagation;
#[async_trait::async_trait]
impl TelemetryPropagation for NoopPropagation {
    async fn inject(&self, _context: &dyn TelemetryContext) -> Option<TelemetryCarrier> {
        None
    }
    async fn extract(&self, _carrier: Option<&TelemetryCarrier>) -> Box<dyn TelemetryContext> {
        Box::new(NoopTelemetryContext)
    }
}

pub static NOOP_TELEMETRY_PROPAGATION: NoopPropagation = NoopPropagation;

/// Shared telemetry context used when an application does not provide one.
pub struct NoopTelemetryContext;

#[async_trait::async_trait]
impl TelemetryContext for NoopTelemetryContext {
    fn metrics(&self) -> Option<&dyn TelemetryMetrics> {
        Some(&NOOP_TELEMETRY_METRICS)
    }
    fn propagation(&self) -> Option<&dyn TelemetryPropagation> {
        Some(&NOOP_TELEMETRY_PROPAGATION)
    }
    async fn start_span_erased(&self, _options: SpanOptions, callback: StartSpanCallback) -> Box<dyn Any + Send> {
        callback(Box::new(NoopTelemetrySpan)).await
    }
}

struct NoopTelemetrySpan;

#[async_trait::async_trait]
impl TelemetryContext for NoopTelemetrySpan {
    fn metrics(&self) -> Option<&dyn TelemetryMetrics> {
        Some(&NOOP_TELEMETRY_METRICS)
    }
    fn propagation(&self) -> Option<&dyn TelemetryPropagation> {
        Some(&NOOP_TELEMETRY_PROPAGATION)
    }
    async fn start_span_erased(&self, options: SpanOptions, callback: StartSpanCallback) -> Box<dyn Any + Send> {
        NoopTelemetryContext.start_span_erased(options, callback).await
    }
}

impl TelemetrySpan for NoopTelemetrySpan {
    fn add_link(&mut self, _link: SpanLink) {}
    fn add_event(&mut self, _name: &str, _attributes: Option<crate::types::SpanAttributes>) {}
    fn set_attributes(&mut self, _attributes: crate::types::SpanAttributes) {}
    fn set_status(&mut self, _status: SpanStatus) {}
}

/// Alias matching the reference export name.
pub static NOOP_TELEMETRY_CONTEXT: NoopTelemetryContext = NoopTelemetryContext;
