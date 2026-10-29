//! Synchronous metric instrument factories.

use std::collections::HashMap;

use crate::types::{AttributeValue, SpanAttributes, TelemetryContext};

#[derive(Clone)]
pub struct InstrumentOptions {
    pub description: Option<String>,
    pub unit: Option<String>,
}

#[derive(Clone)]
pub struct HistogramOptions {
    pub description: Option<String>,
    pub unit: Option<String>,
    pub boundaries: Option<Vec<f64>>,
}

pub trait TelemetryCounter: Send + Sync {
    fn add(&self, value: f64, attributes: Option<SpanAttributes>);
}

pub trait TelemetryUpDownCounter: Send + Sync {
    fn add(&self, value: f64, attributes: Option<SpanAttributes>);
}

pub trait TelemetryHistogram: Send + Sync {
    fn record(&self, value: f64, attributes: Option<SpanAttributes>, context: Option<&dyn TelemetryContext>);
}

pub trait TelemetryMetrics: Send + Sync {
    fn counter(&self, name: &str, options: Option<InstrumentOptions>) -> Box<dyn TelemetryCounter>;
    fn up_down_counter(&self, name: &str, options: Option<InstrumentOptions>) -> Box<dyn TelemetryUpDownCounter>;
    fn histogram(&self, name: &str, options: Option<HistogramOptions>) -> Box<dyn TelemetryHistogram>;
}

/// Typed metrics wrapper keyed by instrument name (schema validation deferred to callers).
pub struct TypedTelemetryMetrics<'a> {
    inner: &'a dyn TelemetryMetrics,
    counters: HashMap<String, Box<dyn TelemetryCounter>>,
    up_down: HashMap<String, Box<dyn TelemetryUpDownCounter>>,
    histograms: HashMap<String, Box<dyn TelemetryHistogram>>,
}

impl<'a> TypedTelemetryMetrics<'a> {
    pub fn new(inner: &'a dyn TelemetryMetrics) -> Self {
        Self {
            inner,
            counters: HashMap::new(),
            up_down: HashMap::new(),
            histograms: HashMap::new(),
        }
    }

    pub fn counter(&mut self, name: &str, options: Option<InstrumentOptions>) -> &dyn TelemetryCounter {
        let key = name.to_string();
        self.counters
            .entry(key)
            .or_insert_with(|| self.inner.counter(name, options));
        self.counters.get(name).map(|b| b.as_ref()).unwrap()
    }

    pub fn up_down_counter(
        &mut self,
        name: &str,
        options: Option<InstrumentOptions>,
    ) -> &dyn TelemetryUpDownCounter {
        let key = name.to_string();
        self.up_down
            .entry(key)
            .or_insert_with(|| self.inner.up_down_counter(name, options));
        self.up_down.get(name).map(|b| b.as_ref()).unwrap()
    }

    pub fn histogram(
        &mut self,
        name: &str,
        options: Option<HistogramOptions>,
    ) -> &dyn TelemetryHistogram {
        let key = name.to_string();
        self.histograms
            .entry(key)
            .or_insert_with(|| self.inner.histogram(name, options));
        self.histograms.get(name).map(|b| b.as_ref()).unwrap()
    }
}

pub fn create_typed_metrics(inner: &dyn TelemetryMetrics) -> TypedTelemetryMetrics<'_> {
    TypedTelemetryMetrics::new(inner)
}

pub fn attribute_eq(map: &SpanAttributes, key: &str, expected: &AttributeValue) -> bool {
    map.get(key).map(|v| v == expected).unwrap_or(false)
}
