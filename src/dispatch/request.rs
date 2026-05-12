//! Request-side SPOE message handler (tasks T027, T029, T031).
//!
//! Per-message overrides of the per-scope `root` setting and the
//! configured sampler decision live in `super::overrides`; see
//! `specs/002-trust-incoming-span/spec.md`.

use crate::config::{Scope, StatusCode};
use crate::correlation::{CorrelationHandle, InFlightSpan};
use crate::dispatch::overrides::{OverrideResolution, SampleDecision};
use crate::dispatch::{RuntimeState, scope as scope_mod};
use crate::pipelines::sampler::ForceSampling;
use crate::propagation::{SpoeArgsExtractor, SpoeVarsInjector};
use haproxy_spoa_hub_plugin_api::RString;
use haproxy_spoa_hub_plugin_api::{
    ProcessingResult, SpoeMessage, SpoeValue, TxnVariable, VarScope,
};
use opentelemetry::Context;
use opentelemetry::trace::{
    Span, SpanBuilder, SpanKind, Status, TraceContextExt, TraceState, Tracer, TracerProvider as _,
};
use opentelemetry_sdk::trace::{SamplingDecision, SamplingResult};

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
        // Resolve per-message overrides once per request. Folds the
        // scope's TOML defaults against `_root` and `_sample` SPOE args,
        // counting any malformed values into the health registry.
        let resolution =
            OverrideResolution::resolve(message, span_rule.root, &scope.name, &state.health);

        let pipeline_name = span_rule.tracer_pipeline.as_deref();
        if let Some(provider) = state.pipelines.tracer_or_default(pipeline_name) {
            let tracer = provider.tracer(state.config.instrumentation.name.clone());
            let builder =
                SpanBuilder::from_name(span_rule.name.clone()).with_kind(SpanKind::Server);
            // opentelemetry-rust 0.32 removed `SpanBuilder::with_sampling_result`,
            // so per-message overrides now flow through Context: the
            // `ForcingSampler` configured on every tracer pipeline reads
            // back the `ForceSampling` value here in `should_sample` and
            // returns it verbatim. See `pipelines/sampler.rs`.
            let base_context = if resolution.effective_root {
                Context::new()
            } else {
                parent_context.clone()
            };
            let active_context = match forced_sampling_result(resolution.sample_decision) {
                Some(forced) => base_context.with_value(ForceSampling(forced)),
                None => base_context,
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
            // Transaction-scoped so HTTP/2 streams multiplexed on the
            // same TCP session (HAProxy "session") don't share the
            // variable slot. Session-scoped would be a correctness
            // hazard under HTTP/2: stream A could pick up stream B's
            // correlation and end the wrong span.
            let handle = CorrelationHandle::generate();
            vars.push(TxnVariable {
                scope: VarScope::Transaction,
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
    // metric-only / log-only scopes too. Also independent of any
    // `_sample` override: dropping a span never silences metrics or logs.
    scope_mod::emit_instruments(state, scope, message);
    scope_mod::emit_log_records(state, scope, message, None);

    ProcessingResult::from_vars(vars)
}

/// Translate a per-message `SampleDecision` into a `SamplingResult`. The
/// caller injects the returned value into the parent `Context` via
/// `ForceSampling`; `ForcingSampler::should_sample` reads it back and
/// short-circuits the configured sampler with it. Returns `None` when
/// no override is requested (the configured sampler decides as usual).
/// Spec §FR-002.
fn forced_sampling_result(decision: SampleDecision) -> Option<SamplingResult> {
    let sampling_decision = match decision {
        SampleDecision::UseSampler => return None,
        SampleDecision::ForceRecord => SamplingDecision::RecordAndSample,
        SampleDecision::ForceDrop => SamplingDecision::Drop,
    };
    Some(SamplingResult {
        decision: sampling_decision,
        attributes: Vec::new(),
        trace_state: TraceState::default(),
    })
}
