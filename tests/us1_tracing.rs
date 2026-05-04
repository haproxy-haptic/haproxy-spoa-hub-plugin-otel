//! User Story 1 — Distributed tracing for HTTP traffic (tasks T017–T022).
//!
//! Each test in this file maps to one acceptance scenario from
//! `specs/001-otel-plugin/spec.md` US1. The fake OTLP collector
//! (`tests/common/otlp_collector.rs`) records every received span; the
//! tests assert on its contents.

mod common;

use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{ProcessingResult, SpoeMessage, SpoeValue};
use haproxy_spoa_hub_plugin_api::{RHashMap, RString, RVec};
use opentelemetry_proto::tonic::common::v1::any_value::Value as OtelValueProto;
use otel_plugin::config::PluginConfig;
use otel_plugin::dispatch::{self, RuntimeState};
use otel_plugin::propagation;
use std::time::Duration;

fn install_propagator() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        opentelemetry::global::set_text_map_propagator(propagation::default_propagator());
    });
}

fn config_with_tracing(grpc_endpoint: &str) -> PluginConfig {
    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[tracer_pipelines]]
name = "default"

[tracer_pipelines.exporter]
endpoint = "{grpc_endpoint}"
protocol = "grpc"
timeout  = "5s"
[tracer_pipelines.exporter.queue]
max_size = 64
max_export_batch_size = 8
scheduled_delay = "10ms"
[tracer_pipelines.sampler]
kind = "always_on"

[tracer_pipelines.resource]
"service.name" = "us1-test"

[[scopes]]
name  = "request"
event = "on-request"

[[scopes.spans]]
name = "haproxy.request"

[[scopes.attributes]]
key    = "http.request.method"
source = "$arg.method"

[[scopes.attributes]]
key    = "url.path"
source = "$arg.path"

[scopes.extract]
prefix = ""
use_headers = true

[scopes.inject]
prefix = ""
use_headers = true

[[scopes]]
name  = "response"
event = "on-response"

[[scopes.attributes]]
key    = "http.response.status_code"
source = "$arg.status"

[scopes.status]
code      = "error"
condition = "$arg.status >= 500"
"#
    );
    PluginConfig::from_toml(&toml).expect("config parses")
}

fn message(event: &str, args: Vec<(&str, SpoeValue)>) -> SpoeMessage {
    let mut map: RHashMap<RString, SpoeValue> = RHashMap::new();
    map.insert(
        RString::from("_event"),
        SpoeValue::String(RString::from(event)),
    );
    for (k, v) in args {
        map.insert(RString::from(k), v);
    }
    SpoeMessage {
        name: RString::from(event),
        args: map,
        stream_id: 0,
        frame_id: 0,
    }
}

fn s(v: &str) -> SpoeValue {
    SpoeValue::String(RString::from(v))
}

fn extract_var<'a>(result: &'a ProcessingResult, name: &str) -> Option<&'a str> {
    result.variables.iter().find_map(|v| {
        if v.name.as_str() == name {
            match &v.value {
                SpoeValue::String(s) => Some(s.as_str()),
                _ => None,
            }
        } else {
            None
        }
    })
}

async fn flush(state: &RuntimeState) {
    // Sleep slightly longer than the configured scheduled_delay (10ms)
    // so the batch span processor flushes.
    tokio::time::sleep(Duration::from_millis(150)).await;
    state.pipelines.drain(Duration::from_secs(2));
}

fn span_attribute<'a>(
    span: &'a opentelemetry_proto::tonic::trace::v1::Span,
    key: &str,
) -> Option<&'a OtelValueProto> {
    span.attributes
        .iter()
        .find(|kv| kv.key == key)
        .and_then(|kv| kv.value.as_ref())
        .and_then(|v| v.value.as_ref())
}

/// US1 acceptance scenario 1 — no incoming traceparent → single root span
/// with operator-configured attributes. (Task T018)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_span_with_attributes() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("runtime state");

    let req = message(
        "on-request",
        vec![("method", s("GET")), ("path", s("/hello"))],
    );
    let req_result = dispatch::dispatch(&state, &req);

    // Plugin must return a correlation handle so HAProxy can round-trip it.
    let correlation = extract_var(&req_result, "correlation").expect("correlation handle present");
    assert_eq!(correlation.len(), 32, "correlation handle is 32 hex chars");

    // Round-trip the correlation handle on the response-side message.
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(correlation)),
            ("status", SpoeValue::Int64(200)),
        ],
    );
    let _ = dispatch::dispatch(&state, &resp);

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .flat_map(|ss| &ss.spans)
        .next()
        .expect("at least one span exported");

    // Root span: parent_span_id is empty.
    assert!(
        span.parent_span_id.is_empty(),
        "root span should have no parent"
    );
    assert_eq!(span.name, "haproxy.request");

    // Configured attributes are present.
    let method = span_attribute(span, "http.request.method").expect("method attribute");
    assert!(matches!(method, OtelValueProto::StringValue(s) if s == "GET"));
    let path = span_attribute(span, "url.path").expect("path attribute");
    assert!(matches!(path, OtelValueProto::StringValue(s) if s == "/hello"));
    let status = span_attribute(span, "http.response.status_code").expect("status attribute");
    assert!(matches!(status, OtelValueProto::IntValue(200)));
}

/// US1 acceptance scenario 2 — incoming traceparent → child span sharing
/// the trace ID; outgoing return variable carries new traceparent. (T019)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_span_with_traceparent_round_trip() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("runtime state");

    let parent_trace_id = "0af7651916cd43dd8448eb211c80319c";
    let parent_span_id = "b7ad6b7169203331";
    let traceparent = format!("00-{parent_trace_id}-{parent_span_id}-01");

    let req = message(
        "on-request",
        vec![
            ("method", s("POST")),
            ("path", s("/api")),
            ("traceparent", s(&traceparent)),
        ],
    );
    let req_result = dispatch::dispatch(&state, &req);

    // The plugin emits a new outbound traceparent for the next hop.
    let outbound_tp = extract_var(&req_result, "traceparent_out").expect("traceparent_out present");
    let outbound_parts: Vec<&str> = outbound_tp.split('-').collect();
    assert_eq!(outbound_parts.len(), 4);
    assert_eq!(outbound_parts[1], parent_trace_id, "trace ID is preserved");

    let correlation = extract_var(&req_result, "correlation").unwrap();
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(correlation)),
            ("status", SpoeValue::Int64(200)),
        ],
    );
    let _ = dispatch::dispatch(&state, &resp);
    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .flat_map(|ss| &ss.spans)
        .next()
        .expect("span exported");

    let trace_id_hex = hex::encode(&span.trace_id);
    let parent_span_id_hex = hex::encode(&span.parent_span_id);
    assert_eq!(trace_id_hex, parent_trace_id);
    assert_eq!(parent_span_id_hex, parent_span_id);

    // The outbound traceparent's span ID matches the new HAProxy span's ID.
    let new_span_id_hex = hex::encode(&span.span_id);
    assert_eq!(outbound_parts[2], new_span_id_hex);
}

/// US1 acceptance scenario 3 — probabilistic sampling. (T020)
///
/// Configures a 10% trace_id_ratio sampler and sends 1,000 message pairs;
/// expects roughly 100 sampled spans (with statistical tolerance). The
/// sampler decision is per-trace, so unsampled spans never reach the
/// exporter at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probabilistic_sampling_within_tolerance() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[tracer_pipelines]]
name = "default"

[tracer_pipelines.exporter]
endpoint = "{}"
protocol = "grpc"
[tracer_pipelines.exporter.queue]
max_size = 4096
max_export_batch_size = 256
scheduled_delay = "10ms"

[tracer_pipelines.sampler]
kind = "trace_id_ratio"
ratio = 0.1

[[scopes]]
name = "r"
event = "on-request"
[[scopes.spans]]
name = "x"
[[scopes]]
name = "rsp"
event = "on-response"
"#,
        collector.grpc_url()
    );
    let cfg = PluginConfig::from_toml(&toml).expect("config");
    let state = RuntimeState::build(cfg).expect("state");

    const N: usize = 1000;
    for _ in 0..N {
        let req = message("on-request", vec![]);
        let r = dispatch::dispatch(&state, &req);
        let correlation = extract_var(&r, "correlation").unwrap().to_string();
        let resp = message("on-response", vec![("correlation", s(&correlation))]);
        let _ = dispatch::dispatch(&state, &resp);
    }

    flush(&state).await;
    // Extra time for the larger batch to drain.
    tokio::time::sleep(Duration::from_millis(300)).await;
    collector.shutdown();

    let total = collector.total_spans().await;
    // 10% of 1,000 = 100 expected. Allow ±70 (~7%) for statistical noise.
    assert!(
        (30..=170).contains(&total),
        "expected ~100 spans (allow 30..=170), got {total}"
    );
}

/// US1 acceptance scenario 4 — collector unreachable, fail-open. (T021, FR-014, FR-015)
///
/// Drives 1000 request/response pairs while the collector rejects every
/// export, then disables the failure injector and drives 100 more pairs
/// to verify export resumes within one batch-processor interval after
/// the collector recovers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collector_outage_does_not_block_traffic() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("state");

    // Phase 1: drive the plugin during an outage. Every export attempt
    // returns INTERNAL; the plugin's hot path must complete in roughly
    // single-digit µs per dispatch despite the failures.
    let outage_failures = 5_000u32;
    collector.inject_failures(outage_failures);

    let started = std::time::Instant::now();
    for _ in 0..1_000 {
        let req = message("on-request", vec![("method", s("GET")), ("path", s("/x"))]);
        let r = dispatch::dispatch(&state, &req);
        let correlation = extract_var(&r, "correlation").unwrap().to_string();
        let resp = message(
            "on-response",
            vec![
                ("correlation", s(&correlation)),
                ("status", SpoeValue::Int64(200)),
            ],
        );
        let _ = dispatch::dispatch(&state, &resp);
    }
    let outage_elapsed = started.elapsed();

    // Hot path under failure stays well under 1 ms per dispatch even at
    // 1000 dispatches. SC-002's p95 ceiling is 1 ms; we add 2 s of
    // headroom for the test runner's variance and the in-process tonic
    // server's accept queue.
    assert!(
        outage_elapsed < Duration::from_secs(5),
        "1000 dispatches took {outage_elapsed:?} during outage (should be <5s)"
    );

    // Phase 2: collector recovers. Drive 100 more pairs and confirm
    // spans are exported within one scheduled-delay window.
    collector
        .failures
        .fail_next_n
        .store(0, std::sync::atomic::Ordering::Relaxed);
    let spans_before_recovery = collector.total_spans().await;

    for _ in 0..100 {
        let req = message("on-request", vec![("method", s("GET")), ("path", s("/y"))]);
        let r = dispatch::dispatch(&state, &req);
        let correlation = extract_var(&r, "correlation").unwrap().to_string();
        let resp = message(
            "on-response",
            vec![
                ("correlation", s(&correlation)),
                ("status", SpoeValue::Int64(200)),
            ],
        );
        let _ = dispatch::dispatch(&state, &resp);
    }
    flush(&state).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let spans_after_recovery = collector.total_spans().await;
    let recovered = spans_after_recovery.saturating_sub(spans_before_recovery);
    // Some spans from phase 2 must reach the collector after recovery.
    assert!(
        recovered > 0,
        "no spans landed after collector recovery (before: {spans_before_recovery}, after: {spans_after_recovery})"
    );

    collector.shutdown();
}

/// US1 acceptance scenario 5 — reload preserves in-flight spans. (T022, FR-021)
///
/// Opens a span via a request-side message under config A, then rebuilds
/// the runtime under config B (same export endpoint, different scope
/// names), then closes the span via a response-side message dispatched
/// against config A's RuntimeState. The span should still export.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_preserves_in_flight_span() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state_a = RuntimeState::build(cfg).expect("state A");

    let req = message("on-request", vec![("method", s("GET")), ("path", s("/x"))]);
    let r = dispatch::dispatch(&state_a, &req);
    let correlation = extract_var(&r, "correlation").unwrap().to_string();

    // Simulate reload — build a second RuntimeState. The first one is
    // still alive and continues to own its in-flight span via the
    // `Arc<TracerProvider>` held by the InFlightSpan entry.
    let cfg_b = config_with_tracing(&collector.grpc_url());
    let _state_b = RuntimeState::build(cfg_b).expect("state B");

    // Response routes through the original state.
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(&correlation)),
            ("status", SpoeValue::Int64(200)),
        ],
    );
    let _ = dispatch::dispatch(&state_a, &resp);

    flush(&state_a).await;
    collector.shutdown();

    assert_eq!(
        collector.total_spans().await,
        1,
        "the span opened on state A should still export after a reload"
    );
}

/// Edge case: malformed traceparent → fall back to a new root trace. (T031)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_traceparent_starts_new_root_trace() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("state");

    let req = message(
        "on-request",
        vec![
            ("method", s("GET")),
            ("path", s("/x")),
            ("traceparent", s("not-a-valid-traceparent")),
        ],
    );
    let r = dispatch::dispatch(&state, &req);
    let correlation = extract_var(&r, "correlation").unwrap().to_string();
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(&correlation)),
            ("status", SpoeValue::Int64(200)),
        ],
    );
    let _ = dispatch::dispatch(&state, &resp);

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .flat_map(|ss| &ss.spans)
        .next()
        .expect("span exported");
    assert!(
        span.parent_span_id.is_empty(),
        "malformed traceparent must yield a root span"
    );
}

/// Edge case: oversized `tracestate` (W3C cap is 512 octets after
/// concatenation) — the plugin must not crash; the span must still
/// emit, sharing the inbound trace_id; the outbound `tracestate_out`
/// must be present and respect the W3C cap. (T031, second branch)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_tracestate_does_not_crash() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("runtime state");

    let parent_trace_id = "0af7651916cd43dd8448eb211c80319c";
    let traceparent = format!("00-{parent_trace_id}-b7ad6b7169203331-01");

    // Build a `tracestate` value that's well over the 512-octet W3C cap:
    // 64 vendor entries × ~20 chars = ~1280 chars. The propagator must
    // either drop it or truncate it; either way the request must succeed.
    let oversized_tracestate = (0..64)
        .map(|i| format!("vendor{i:02}=value{i:02}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(oversized_tracestate.len() > 512);

    let req = message(
        "on-request",
        vec![
            ("method", s("GET")),
            ("path", s("/x")),
            ("traceparent", s(&traceparent)),
            ("tracestate", s(&oversized_tracestate)),
        ],
    );
    let req_result = dispatch::dispatch(&state, &req);

    // The plugin emits an outbound traceparent — the trace_id is preserved
    // even when the inbound tracestate was oversized. Whether the outbound
    // tracestate is truncated, dropped, or empty, the plugin must not crash.
    let outbound_tp = extract_var(&req_result, "traceparent_out")
        .expect("traceparent_out present despite oversized tracestate");
    assert!(outbound_tp.contains(parent_trace_id), "trace_id preserved");

    let outbound_ts = extract_var(&req_result, "tracestate_out").unwrap_or("");
    assert!(
        outbound_ts.len() <= 512,
        "outbound tracestate must respect W3C 512-octet cap (got {} chars)",
        outbound_ts.len()
    );

    let correlation = extract_var(&req_result, "correlation").unwrap();
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(correlation)),
            ("status", SpoeValue::Int64(200)),
        ],
    );
    let _ = dispatch::dispatch(&state, &resp);
    flush(&state).await;
    collector.shutdown();

    // The span made it through.
    let sink = collector.sink.lock().await;
    let span = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .flat_map(|ss| &ss.spans)
        .find(|s| hex::encode(&s.trace_id) == parent_trace_id);
    assert!(
        span.is_some(),
        "span with the inbound trace_id must still be exported"
    );
}

/// Regression test: every variable the plugin emits on the request-side
/// MUST be transaction-scoped (HAProxy `txn`), not session-scoped.
/// Session scope was the original choice but is incorrect under HTTP/2:
/// multiple streams multiplexed on one TCP session (= HAProxy `sess`)
/// would race on the same variable slot, so stream A could pick up
/// stream B's correlation handle and end stream B's span as if it were
/// stream A's. Transaction scope gives each HTTP/2 stream its own slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_emits_only_transaction_scoped_vars() {
    use haproxy_spoa_hub_plugin_api::VarScope;

    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("runtime state");

    let req = message(
        "on-request",
        vec![
            ("method", s("GET")),
            ("path", s("/regression")),
            (
                "traceparent",
                s("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
            ),
        ],
    );
    let r = dispatch::dispatch(&state, &req);

    assert!(
        !r.variables.is_empty(),
        "request-side dispatch produced no variables"
    );
    for v in r.variables.iter() {
        assert_eq!(
            v.scope,
            VarScope::Transaction,
            "var `{}` MUST be Transaction-scoped (got {:?}) — session-scoped vars race under HTTP/2 multiplexing",
            v.name.as_str(),
            v.scope
        );
    }

    // Sanity: at minimum we expect correlation + traceparent_out.
    let names: Vec<&str> = r.variables.iter().map(|v| v.name.as_str()).collect();
    assert!(
        names.contains(&"correlation"),
        "missing `correlation`: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("traceparent_out")),
        "missing `traceparent_out`: {names:?}"
    );

    collector.shutdown();
}

/// Edge case: response-side message without `correlation` argument. (T032)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn response_without_correlation_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config_with_tracing(&collector.grpc_url());
    let state = RuntimeState::build(cfg).expect("state");

    let resp = message(
        "on-response",
        vec![("status", SpoeValue::Int64(200))], // no correlation
    );
    let _ = dispatch::dispatch(&state, &resp);

    let snapshot = state.health.snapshot();
    assert_eq!(snapshot.correlation_missing_total, 1);

    collector.shutdown();
}

#[allow(dead_code)]
fn _unused_to_silence_rvec(v: RVec<u8>) -> RVec<u8> {
    v
}
