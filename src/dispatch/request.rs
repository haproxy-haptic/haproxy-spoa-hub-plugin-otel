//! Request-side SPOE message handler (tasks T027, T029, T031).

use crate::config::{Scope, StatusCode};
use crate::correlation::{CorrelationHandle, InFlightSpan};
use crate::dispatch::{RuntimeState, scope as scope_mod};
use crate::propagation::{SpoeArgsExtractor, SpoeVarsInjector};
use abi_stable::std_types::RString;
use haproxy_spoa_hub_plugin_api::{
    ProcessingResult, SpoeMessage, SpoeValue, TxnVariable, VarScope,
};
use opentelemetry::Context;
use opentelemetry::trace::{
    Span, SpanBuilder, SpanKind, Status, TraceContextExt, Tracer, TracerProvider as _,
};

pub fn handle(state: &RuntimeState, scope: &Scope, message: &SpoeMessage) -> ProcessingResult {
    // Extract any incoming W3C TraceContext / Baggage from SPOE args.
    let prefix = scope
        .extract
        .as_ref()
        .map(|e| e.prefix.as_str())
        .unwrap_or("");
    let extractor = SpoeArgsExtractor::new(message, prefix);
    let parent_context: Context =
        opentelemetry::global::get_text_map_propagator(|p| p.extract(&extractor));

    let mut vars: Vec<TxnVariable> = Vec::new();

    // Span-emitting path: only when the scope declares a span rule and a
    // tracer pipeline is available. Metric-only / log-only scopes skip
    // this entire block and just emit instruments / log records below.
    if let Some(span_rule) = scope.spans.first() {
        let pipeline_name = span_rule.tracer_pipeline.as_deref();
        if let Some(provider) = state.pipelines.tracer_or_default(pipeline_name) {
            let tracer = provider.tracer(state.config.instrumentation.name.clone());
            let builder =
                SpanBuilder::from_name(span_rule.name.clone()).with_kind(SpanKind::Server);
            let active_context = if span_rule.root {
                Context::new()
            } else {
                parent_context.clone()
            };
            let mut span = tracer.build_with_context(builder, &active_context);

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

            let context = active_context.with_span(span);

            // Inject the new context into return variables.
            let inject_prefix = scope
                .inject
                .as_ref()
                .map(|i| i.prefix.as_str())
                .unwrap_or("");
            let mut injector = SpoeVarsInjector::new(inject_prefix);
            opentelemetry::global::get_text_map_propagator(|p| {
                p.inject_context(&context, &mut injector)
            });
            vars.extend(injector.into_vars());

            // Emit a correlation handle so HAProxy can round-trip it.
            let handle = CorrelationHandle::generate();
            vars.push(TxnVariable {
                scope: VarScope::Session,
                name: RString::from("correlation"),
                value: SpoeValue::String(RString::from(handle.to_hex())),
            });
            state.correlation.insert(handle, InFlightSpan::new(context));
        } else {
            log::debug!(
                "otel plugin: scope `{}` declares spans but no tracer pipeline is configured; skipping span creation",
                scope.name
            );
        }
    }

    // Emit instruments and log records regardless of whether a span was
    // opened — these are independent telemetry types and should fire on
    // metric-only / log-only scopes too.
    scope_mod::emit_instruments(state, scope, message);
    scope_mod::emit_log_records(state, scope, message, None);

    ProcessingResult::from_vars(vars)
}
