//! User Story 4 — Mechanical configuration parity with the built-in
//! OTEL addon (tasks T046–T049).

mod common;

use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{RHashMap, RString};
use haproxy_spoa_hub_plugin_api::{SpoeMessage, SpoeValue};
use otel_plugin::config::PluginConfig;
use otel_plugin::dispatch::{self, RuntimeState};
use otel_plugin::error::ConfigError;
use otel_plugin::propagation;
use std::time::Duration;

fn install_propagator() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        opentelemetry::global::set_text_map_propagator(propagation::default_propagator());
    });
}

fn s(v: &str) -> SpoeValue {
    SpoeValue::String(RString::from(v))
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

/// US4 acceptance scenario 1 — a TOML file whose key paths mirror a
/// valid built-in-addon configuration produces equivalent telemetry. (T046)
///
/// The TOML below is the mechanical translation (via
/// `contracts/addon-directive-mapping.md`) of an addon config that
/// declares an instrumentation, one scope on `on-frontend-http-request`
/// with `attribute http.request.method method`, and a single tracer
/// pipeline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn addon_vocabulary_mirror_loads_and_emits() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let toml = format!(
        r#"
# Mirrors the addon's:
#     otel-instrumentation app
#       scopes request
#     otel-scope request
#       otel-event on-frontend-http-request
#       span haproxy.request
#       attribute http.request.method method
#       attribute url.path path

[instrumentation]
name = "app"
scopes = ["request"]

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.sampler]
kind = "always_on"
[tracer_pipelines.exporter]
endpoint = "{}"
protocol = "grpc"
[tracer_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"

[[scopes]]
name  = "request"
event = "on-frontend-http-request"
kind  = "request"

[[scopes.spans]]
name = "haproxy.request"

[[scopes.attributes]]
key    = "http.request.method"
source = "$arg.method"

[[scopes.attributes]]
key    = "url.path"
source = "$arg.path"

[[scopes]]
name  = "response"
event = "on-http-response"
kind  = "response"
"#,
        collector.grpc_url()
    );

    let cfg = PluginConfig::from_toml(&toml).expect("addon-mirror config parses");
    let state = RuntimeState::build(cfg).expect("state");

    let req = dispatch::dispatch(
        &state,
        &message(
            "on-frontend-http-request",
            vec![("method", s("GET")), ("path", s("/hello"))],
        ),
    );
    let correlation = req
        .variables
        .iter()
        .find(|v| v.name.as_str() == "correlation")
        .map(|v| match &v.value {
            SpoeValue::String(s) => s.as_str().to_string(),
            _ => panic!("correlation"),
        })
        .expect("correlation handle present");

    let _ = dispatch::dispatch(
        &state,
        &message("on-http-response", vec![("correlation", s(&correlation))]),
    );

    tokio::time::sleep(Duration::from_millis(150)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    assert_eq!(collector.total_spans().await, 1);
}

/// US4 acceptance scenario 2 — TOML with a key path that mirrors an
/// unsupported addon directive is refused at startup. (T047, FR-017)
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn unsupported_toml_key_is_rejected_with_named_error() {
    let toml = r#"
[instrumentation]
name = "app"
this_directive_does_not_exist = true
"#;
    let err = PluginConfig::from_toml(toml).unwrap_err();
    match err {
        ConfigError::Schema { key_path, message } => {
            // `additionalProperties: false` rejects with a path that
            // includes the offending key.
            assert!(
                key_path.contains("instrumentation"),
                "error path mentions the offending block: {key_path}"
            );
            // The error message should name the offending key so the
            // operator can find it in the file.
            assert!(
                message.contains("this_directive_does_not_exist"),
                "error mentions the offending key: {message}"
            );
        }
        other => panic!("expected Schema error, got {other:?}"),
    }
}

/// US4 acceptance scenario 3 — every published TOML key path round-trips
/// through `PluginConfig::from_toml` cleanly. The smoke check below uses
/// a maximally-rich TOML that exercises every section we support; if it
/// parses without error and a round-trip dispatch produces telemetry the
/// addon-vocabulary cover is healthy. (T048)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_addon_vocabulary_round_trip() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let toml = format!(
        r#"
[instrumentation]
name = "full"
debug_level = 2
groups = ["g1"]
scopes = ["s1", "s2"]

[instrumentation.option]
disabled = false
dontlog_normal = true
hard_errors = false

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.sampler]
kind = "parent_based"
[tracer_pipelines.sampler.root]
kind = "trace_id_ratio"
ratio = 1.0
[tracer_pipelines.exporter]
endpoint = "{0}"
protocol = "grpc"
[tracer_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"
[tracer_pipelines.resource]
"service.name" = "full"

[[meter_pipelines]]
name = "default"
[meter_pipelines.exporter]
endpoint = "{0}"
protocol = "grpc"
[meter_pipelines.reader]
interval = "100ms"

[[logger_pipelines]]
name = "default"
[logger_pipelines.exporter]
endpoint = "{0}"
protocol = "grpc"

[[scopes]]
name  = "s1"
event = "on-frontend-http-request"
kind  = "request"

[[scopes.spans]]
name = "haproxy.request"
root = false

[[scopes.attributes]]
key = "http.request.method"
source = "$arg.method"
type   = "string"

[[scopes.events]]
name = "request.received"
[[scopes.events.attributes]]
key = "evt.path"
source = "$arg.path"

[[scopes.baggage]]
key = "tenant"
source = "$arg.tenant"

[[scopes.instruments]]
kind  = "cnt_int"
name  = "http.requests"
value = 1

[[scopes.log_records]]
severity = "INFO"
body = "received"

[[scopes]]
name  = "s2"
event = "on-http-response"
kind  = "response"

[[scopes.attributes]]
key = "http.response.status_code"
source = "$arg.status"
type   = "int"

[scopes.status]
code      = "error"
condition = "$arg.status >= 500"

[[groups]]
name   = "g1"
scopes = ["s1"]
"#,
        collector.grpc_url()
    );

    let cfg = PluginConfig::from_toml(&toml).expect("full vocabulary parses");
    let state = RuntimeState::build(cfg).expect("state");

    let req = dispatch::dispatch(
        &state,
        &message(
            "on-frontend-http-request",
            vec![
                ("method", s("GET")),
                ("path", s("/hello")),
                ("tenant", s("acme")),
            ],
        ),
    );
    let correlation = req
        .variables
        .iter()
        .find(|v| v.name.as_str() == "correlation")
        .map(|v| match &v.value {
            SpoeValue::String(s) => s.as_str().to_string(),
            _ => panic!("correlation"),
        })
        .unwrap();

    let _ = dispatch::dispatch(
        &state,
        &message(
            "on-http-response",
            vec![
                ("correlation", s(&correlation)),
                ("status", SpoeValue::Int64(200)),
            ],
        ),
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    assert_eq!(collector.total_spans().await, 1);
    assert!(collector.total_metrics().await >= 1);
    assert!(collector.total_logs().await >= 1);
}
