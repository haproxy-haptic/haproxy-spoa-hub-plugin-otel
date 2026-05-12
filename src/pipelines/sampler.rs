//! Forcing-sampler wrapper.
//!
//! opentelemetry-rust 0.32 removed `SpanBuilder::with_sampling_result`,
//! so per-message overrides (the `_sample` SPOE arg, spec §FR-002)
//! can no longer short-circuit the configured sampler by pre-baking a
//! `SamplingResult` into the builder. The SDK now unconditionally
//! invokes `Sampler::should_sample` for every span, with one input
//! the caller controls — the parent `Context`.
//!
//! `ForcingSampler` wraps the configured `SdkSampler` and consults the
//! parent `Context` for a [`ForceSampling`] value. If present, that
//! value is returned verbatim and the inner sampler is skipped. If
//! absent, the inner sampler decides as before.
//!
//! The request-side dispatcher injects `ForceSampling` into the context
//! it hands to `build_with_context` whenever `_sample=force_record` /
//! `force_drop` is asserted on the incoming SPOE message — see
//! `dispatch/request.rs`. This keeps the forced-decision plumbing
//! single-shot (the value lives only on the span's parent context, not
//! the propagated traceparent), preserving the old behaviour where
//! every fresh span re-evaluates `_sample`.
//!
//! The default `SdkSampler` path (no override) is byte-identical to
//! pre-0.32 behaviour because `should_sample` was always invoked on
//! the configured sampler when no `SpanBuilder::with_sampling_result`
//! was set.

use opentelemetry::trace::{Link, SpanKind, TraceId};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{Sampler as SdkSampler, SamplingResult, ShouldSample};

/// Context-attached forced sampling decision. The request-side
/// dispatcher inserts this via `Context::with_value` for spans whose
/// `_sample` override is `force_record` / `force_drop`; the wrapping
/// [`ForcingSampler`] reads it back during `should_sample`. Stored as
/// a thin wrapper rather than the raw `SamplingResult` so the
/// downcast in `Context::get::<ForceSampling>()` keys on this type and
/// can't collide with any other consumer storing a `SamplingResult`.
#[derive(Clone, Debug)]
pub struct ForceSampling(pub SamplingResult);

/// Wraps an `SdkSampler` so per-message overrides can short-circuit it.
#[derive(Clone, Debug)]
pub struct ForcingSampler {
    inner: SdkSampler,
}

impl ForcingSampler {
    pub fn new(inner: SdkSampler) -> Self {
        Self { inner }
    }
}

impl ShouldSample for ForcingSampler {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        if let Some(cx) = parent_context {
            if let Some(force) = cx.get::<ForceSampling>() {
                return force.0.clone();
            }
        }
        self.inner
            .should_sample(parent_context, trace_id, name, span_kind, attributes, links)
    }
}
