//! Per-message override of `_root` and `_sample` SPOE arguments.
//!
//! Each test maps to one acceptance scenario from
//! `specs/002-trust-incoming-span/spec.md` (User Stories 1 and 2). The
//! fake OTLP collector under `tests/common/otlp_collector.rs` records
//! every received span; tests assert on its contents and on the
//! plugin's health snapshot for the malformed-override counter.

mod common;

use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{ProcessingResult, SpoeMessage, SpoeValue};
use haproxy_spoa_hub_plugin_api::{RHashMap, RString};
use otel_plugin::config::PluginConfig;
use otel_plugin::dispatch::{self, RuntimeState};
use otel_plugin::propagation;
use std::time::Duration;

const PARENT_TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
const PARENT_TRACE_ID_HEX: &str = "0af7651916cd43dd8448eb211c80319c";

fn install_propagator() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        opentelemetry::global::set_text_map_propagator(propagation::default_propagator());
    });
}

/// Builds a plugin config with one tracer pipeline and one request /
/// response scope pair. `root_default` toggles the per-scope `root`
/// setting; `sampler_kind` selects the pipeline sampler ("always_on",
/// "always_off", etc.).
fn config(grpc_endpoint: &str, root_default: bool, sampler_kind: &str) -> PluginConfig {
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
kind = "{sampler_kind}"

[tracer_pipelines.resource]
"service.name" = "us2-override-test"

[[scopes]]
name  = "request"
event = "on-request"

[[scopes.spans]]
name = "haproxy.request"
root = {root_default}

[scopes.extract]
prefix = ""
use_headers = true

[scopes.inject]
prefix = ""
use_headers = true

[[scopes]]
name  = "response"
event = "on-response"
"#
    );
    PluginConfig::from_toml(&toml).expect("config parses")
}

fn s(v: &str) -> SpoeValue {
    SpoeValue::String(RString::from(v))
}

fn message(event: &str, args: Vec<(&str, SpoeValue)>) -> SpoeMessage {
    let mut map: RHashMap<RString, SpoeValue> = RHashMap::new();
    map.insert(RString::from("_event"), s(event));
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
    tokio::time::sleep(Duration::from_millis(150)).await;
    state.pipelines.drain(Duration::from_secs(2));
}

/// Helper: run a request+response pair through dispatch and return the
/// (correlation, request_result) pair. Caller is responsible for
/// flushing and asserting on collector contents and health.
fn run_request_response(
    state: &RuntimeState,
    request_args: Vec<(&str, SpoeValue)>,
) -> (Option<String>, ProcessingResult) {
    let req = message("on-request", request_args);
    let req_result = dispatch::dispatch(state, &req);
    let correlation = extract_var(&req_result, "correlation").map(String::from);
    if let Some(c) = correlation.as_deref() {
        let resp = message(
            "on-response",
            vec![("correlation", s(c)), ("status", SpoeValue::Int64(200))],
        );
        let _ = dispatch::dispatch(state, &resp);
    }
    (correlation, req_result)
}

/// US1 scenario 2 — `_root = "true"` with inbound traceparent: span
/// emitted has a fresh trace_id, no parent reference, and
/// `traceparent_out` carries the new trace ID, not the inbound one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_true_overrides_inbound_traceparent() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (_, req_result) = run_request_response(
        &state,
        vec![("traceparent", s(PARENT_TRACEPARENT)), ("_root", s("true"))],
    );

    let traceparent_out =
        extract_var(&req_result, "traceparent_out").expect("traceparent_out emitted");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_eq!(parts.len(), 4, "traceparent_out: {traceparent_out}");
    assert_ne!(
        parts[1], PARENT_TRACE_ID_HEX,
        "fresh root MUST NOT reuse the inbound trace ID"
    );

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
        .expect("a span was exported");
    assert!(
        span.parent_span_id.is_empty(),
        "with _root=true the span MUST be a root span (no parent)"
    );
}

/// US1 scenario 3 — `_root = "false"` with scope default `root = false`
/// is a no-op: the inbound parent is extracted as today.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_false_with_scope_default_false_extracts_parent() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (_, req_result) = run_request_response(
        &state,
        vec![
            ("traceparent", s(PARENT_TRACEPARENT)),
            ("_root", s("false")),
        ],
    );

    let traceparent_out =
        extract_var(&req_result, "traceparent_out").expect("traceparent_out emitted");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_eq!(parts[1], PARENT_TRACE_ID_HEX, "trace ID is preserved");

    flush(&state).await;
    collector.shutdown();
}

/// US1 scenario 4 — scope default `root = true`, per-message
/// `_root = "false"`: per-message override wins, parent extracted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_false_overrides_scope_default_true() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), true, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (_, req_result) = run_request_response(
        &state,
        vec![
            ("traceparent", s(PARENT_TRACEPARENT)),
            ("_root", s("false")),
        ],
    );

    let traceparent_out =
        extract_var(&req_result, "traceparent_out").expect("traceparent_out emitted");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_eq!(
        parts[1], PARENT_TRACE_ID_HEX,
        "per-message override MUST win over per-scope default"
    );
}

/// US1 scenario 5 — malformed `_root`: override ignored, scope default
/// applies, malformed-override counter increments by one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_malformed_falls_back_and_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), true, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (_, req_result) = run_request_response(
        &state,
        vec![("traceparent", s(PARENT_TRACEPARENT)), ("_root", s("yes"))],
    );

    let traceparent_out =
        extract_var(&req_result, "traceparent_out").expect("traceparent_out emitted");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_ne!(
        parts[1], PARENT_TRACE_ID_HEX,
        "scope default root=true applies → fresh root"
    );

    let snap = state.health.snapshot();
    let counter = snap
        .malformed_override_total
        .get(&("_root".to_string(), "request".to_string()))
        .copied()
        .unwrap_or(0);
    assert_eq!(counter, 1, "malformed-override counter increments");

    collector.shutdown();
}

/// US2 scenario 1 — pipeline sampler `always_off`, no `_sample`: no
/// span exported (regression check on existing sampler behaviour).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn always_off_with_no_sample_arg_drops_span() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_off");
    let state = RuntimeState::build(cfg).expect("runtime state");

    run_request_response(&state, vec![]);

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(span_count, 0, "always_off must drop the span");
}

/// US2 scenario 2 — pipeline sampler `always_off`, `_sample = "record"`:
/// span IS exported (override forces RecordAndSample).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_record_overrides_always_off() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_off");
    let state = RuntimeState::build(cfg).expect("runtime state");

    run_request_response(&state, vec![("_sample", s("record"))]);

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(span_count, 1, "_sample=record forces export");
}

/// US2 scenario 3 — pipeline sampler `always_on`, `_sample = "drop"`:
/// no span data reaches the collector, but the correlation handle is
/// allocated, `traceparent_out` is emitted with the unsampled flag,
/// and the response-side close does NOT inflate
/// `correlation_missing_total`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_drop_overrides_always_on_but_keeps_correlation() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (correlation, req_result) = run_request_response(&state, vec![("_sample", s("drop"))]);

    assert!(
        correlation.is_some(),
        "correlation handle MUST still be allocated when _sample=drop"
    );

    let traceparent_out = extract_var(&req_result, "traceparent_out")
        .expect("traceparent_out is emitted even when dropped");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_eq!(
        parts[3], "00",
        "W3C unsampled flag (00) on a dropped span; got: {traceparent_out}"
    );

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(span_count, 0, "_sample=drop forces no export");

    let snap = state.health.snapshot();
    assert_eq!(
        snap.correlation_missing_total, 0,
        "dropped spans MUST close cleanly via correlation handle, not inflate the missing counter"
    );
}

/// US2 scenario 4 — `_sample = "default"` is equivalent to the argument
/// being absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_default_equals_absent() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_off");
    let state = RuntimeState::build(cfg).expect("runtime state");

    run_request_response(&state, vec![("_sample", s("default"))]);

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(
        span_count, 0,
        "_sample=default MUST defer to the configured sampler (always_off here)"
    );
}

/// US2 scenario 5 — malformed `_sample`: override ignored, sampler
/// decides, malformed-override counter increments.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_malformed_falls_back_and_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_off");
    let state = RuntimeState::build(cfg).expect("runtime state");

    run_request_response(&state, vec![("_sample", s("maybe"))]);

    let snap = state.health.snapshot();
    let counter = snap
        .malformed_override_total
        .get(&("_sample".to_string(), "request".to_string()))
        .copied()
        .unwrap_or(0);
    assert_eq!(counter, 1, "malformed-override counter increments");

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(
        span_count, 0,
        "with malformed _sample the configured sampler (always_off) decides"
    );
}

/// Edge case: `_root = "true"` AND `_sample = "drop"` — fresh root,
/// non-recording, but `traceparent_out` carries the synthetic root with
/// the unsampled flag. Downstream services see a fresh trace identity
/// even though the local span was dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_true_with_sample_drop_emits_synthetic_unsampled_context() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let (_, req_result) = run_request_response(
        &state,
        vec![
            ("traceparent", s(PARENT_TRACEPARENT)),
            ("_root", s("true")),
            ("_sample", s("drop")),
        ],
    );

    let traceparent_out =
        extract_var(&req_result, "traceparent_out").expect("traceparent_out emitted");
    let parts: Vec<&str> = traceparent_out.split('-').collect();
    assert_eq!(parts.len(), 4);
    assert_ne!(
        parts[1], PARENT_TRACE_ID_HEX,
        "fresh root MUST NOT reuse the inbound trace ID"
    );
    assert_eq!(parts[3], "00", "unsampled flag on a dropped span");

    flush(&state).await;
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let span_count: usize = sink
        .trace_exports
        .iter()
        .flat_map(|e| &e.resource_spans)
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum();
    assert_eq!(span_count, 0, "_sample=drop forces no export");
}

/// Reserved-arg discipline: `_root` and `_sample` on the response-side
/// message are silently ignored (FR-003). Verified indirectly: the
/// response-side closes a recording span, no health counters move, and
/// the request-side outcome is unaffected (already verified in other
/// tests). This test asserts the response-side path produces no errors
/// and no malformed-override counter increments when the args are
/// present on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn response_side_overrides_are_silently_ignored() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let cfg = config(&collector.grpc_url(), false, "always_on");
    let state = RuntimeState::build(cfg).expect("runtime state");

    let req = message("on-request", vec![]);
    let req_result = dispatch::dispatch(&state, &req);
    let correlation = extract_var(&req_result, "correlation")
        .expect("correlation handle present")
        .to_string();

    // Response-side message carrying _root and _sample (operator
    // symmetric wiring). Plugin must NOT count these as malformed and
    // MUST NOT crash.
    let resp = message(
        "on-response",
        vec![
            ("correlation", s(&correlation)),
            ("status", SpoeValue::Int64(200)),
            ("_root", s("yes")),
            ("_sample", s("maybe")),
        ],
    );
    let _ = dispatch::dispatch(&state, &resp);

    let snap = state.health.snapshot();
    assert!(
        snap.malformed_override_total.is_empty(),
        "_root / _sample on response-side must NOT increment the counter; got {:?}",
        snap.malformed_override_total
    );

    flush(&state).await;
    collector.shutdown();
}
