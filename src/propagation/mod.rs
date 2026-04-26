//! W3C TraceContext + Baggage propagation across SPOE messages
//! (task T012).
//!
//! On the request-side message the operator passes the inbound
//! `traceparent`, `tracestate`, and `baggage` headers to the plugin as
//! SPOE arguments. The plugin uses `SpoeArgsExtractor` to parse them
//! into an OTel `Context`. After building the local span, the plugin
//! uses `SpoeVarsInjector` to write the outbound forms as session-scoped
//! return variables that HAProxy then sets as outgoing headers.

pub mod extractor;
pub mod injector;

pub use extractor::SpoeArgsExtractor;
pub use injector::SpoeVarsInjector;

use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};

/// Build the default propagator stack: W3C TraceContext + Baggage.
/// Per research item R12 — no B3 / Jaeger in v1.
#[must_use]
pub fn default_propagator() -> TextMapCompositePropagator {
    TextMapCompositePropagator::new(vec![
        Box::new(TraceContextPropagator::new()),
        Box::new(BaggagePropagator::new()),
    ])
}
