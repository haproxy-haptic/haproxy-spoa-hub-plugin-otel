//! Response-side SPOE message handler (tasks T028, T029, T032).

use super::ARG_CORRELATION;
use crate::config::{Scope, StatusCode};
use crate::correlation::{CorrelationHandle, EvictionReason};
use crate::dispatch::{RuntimeState, scope as scope_mod};
use haproxy_spoa_hub_plugin_api::{ProcessingResult, SpoeMessage};
use opentelemetry::trace::{Status, TraceContextExt};

pub fn handle(state: &RuntimeState, scope: &Scope, message: &SpoeMessage) -> ProcessingResult {
    // 1. Look up the correlation handle. Missing or malformed → emit a
    //    minimal error span and increment the health counter (T032).
    let Some(handle_str) = message.get_string(ARG_CORRELATION) else {
        state.health.correlation_missing_inc();
        log::warn!(
            "otel plugin: response scope `{}` (event `{}`) lacks correlation arg",
            scope.name,
            scope.event
        );
        return ProcessingResult::from_vars(Vec::new());
    };
    let handle = match CorrelationHandle::from_hex(handle_str) {
        Ok(h) => h,
        Err(e) => {
            state.health.correlation_missing_inc();
            log::warn!(
                "otel plugin: response scope `{}` carried unparseable correlation handle `{handle_str}`: {e}",
                scope.name
            );
            return ProcessingResult::from_vars(Vec::new());
        }
    };
    let Some(in_flight) = state.correlation.remove(&handle) else {
        state.health.correlation_missing_inc();
        log::warn!("otel plugin: correlation handle `{handle}` not found in store");
        return ProcessingResult::from_vars(Vec::new());
    };

    // 2. Apply scope rules to the live span.
    let context = &in_flight.context;
    let span = context.span();

    for rule in &scope.attributes {
        if let Some(kv) = scope_mod::attribute_kv(rule, message) {
            span.set_attribute(kv);
        }
    }
    for ev in &scope.events {
        let attrs = ev
            .attributes
            .iter()
            .filter_map(|a| scope_mod::attribute_kv(a, message))
            .collect::<Vec<_>>();
        span.add_event(ev.name.clone(), attrs);
    }
    if let Some(status) = &scope.status {
        if scope_mod::condition_holds(&status.condition, message) {
            match status.code {
                StatusCode::Ok => span.set_status(Status::Ok),
                StatusCode::Error => span.set_status(Status::error("error")),
                StatusCode::Unset => {}
            }
        }
    }

    // 3. End the span. The SDK's batch span processor exports it
    //    asynchronously; the plugin returns immediately.
    span.end();

    // 4. Emit any instruments and log records configured on this scope.
    //    Log records emitted on a response-side scope inherit the active
    //    span's trace context for trace ↔ log correlation.
    scope_mod::emit_instruments(state, scope, message);
    scope_mod::emit_log_records(state, scope, message, Some(context));

    // Explicitly drop the in-flight entry so the BoxedSpan inside its
    // captured context is dropped now (not at end of scope) — that drop
    // is what actually queues the span for export by the batch processor.
    drop(in_flight);

    state.health.eviction_inc(EvictionReason::Completed);

    ProcessingResult::from_vars(Vec::new())
}
