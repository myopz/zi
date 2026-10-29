//! Telemetry span/metrics trait and NOOP + in-memory adapters for zi.

mod in_memory;
mod metrics;
mod noop;
mod types;

pub use in_memory::{
    InMemoryTelemetryContext, RecordedTelemetryEvent, RecordedTelemetryLink, RecordedTelemetryMeasurement,
    RecordedTelemetrySpan,
};
pub use metrics::{
    create_typed_metrics, HistogramOptions, InstrumentOptions, TelemetryCounter, TelemetryHistogram,
    TelemetryMetrics, TelemetryUpDownCounter, TypedTelemetryMetrics,
};
pub use noop::{NoopTelemetryContext, NOOP_TELEMETRY_CONTEXT, NOOP_TELEMETRY_METRICS, NOOP_TELEMETRY_PROPAGATION};
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_span_runs_callback() {
        let value = NOOP_TELEMETRY_CONTEXT
            .start_span(
                SpanOptions {
                    name: "test".to_string(),
                    attributes: None,
                    links: None,
                },
                |mut _span| async move { 42 },
            )
            .await;
        assert_eq!(value, 42);
    }

    #[tokio::test]
    async fn in_memory_records_span() {
        let telemetry = InMemoryTelemetryContext::new();
        telemetry
            .start_span(
                SpanOptions {
                    name: "root".to_string(),
                    attributes: None,
                    links: None,
                },
                |mut span| async move {
                    span.add_event("evt", None);
                    "done"
                },
            )
            .await;
        let spans = telemetry.get_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "root");
        assert!(spans[0].settled);
        assert_eq!(spans[0].events.len(), 1);
    }

    #[tokio::test]
    async fn in_memory_records_counter() {
        let telemetry = InMemoryTelemetryContext::new();
        let counter = telemetry.counter("requests", None);
        counter.add(1.0, None);
        assert_eq!(telemetry.get_metrics().len(), 1);
    }
}
