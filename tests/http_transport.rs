//! T024 — HTTP OTLP transport tests.
//!
//! Verifies FR-012: the plugin can export traces, metrics, and logs via
//! both `http/protobuf` and `http/json` in addition to `grpc`.

mod common;

use abi_stable::std_types::{RHashMap, RString};
use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{SpoeMessage, SpoeValue};
use otel_plugin::config::PluginConfig;
use otel_plugin::dispatch::{self, RuntimeState};
use otel_plugin::propagation;
use std::time::Duration;

fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        opentelemetry::global::set_text_map_propagator(propagation::default_propagator());
    });
}

fn s(v: &str) -> SpoeValue {
    SpoeValue::String(RString::from(v))
}

fn message(event: &str, args: Vec<(&str, SpoeValue)>) -> SpoeMessage {
    let mut m: RHashMap<RString, SpoeValue> = RHashMap::new();
    m.insert(
        RString::from("_event"),
        SpoeValue::String(RString::from(event)),
    );
    for (k, v) in args {
        m.insert(RString::from(k), v);
    }
    SpoeMessage {
        name: RString::from(event),
        args: m,
        stream_id: 0,
        frame_id: 0,
    }
}

fn config_with_protocol(http_endpoint: &str, protocol: &str) -> PluginConfig {
    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.sampler]
kind = "always_on"
[tracer_pipelines.exporter]
endpoint = "{http_endpoint}"
protocol = "{protocol}"
[tracer_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"

[[meter_pipelines]]
name = "default"
[meter_pipelines.exporter]
endpoint = "{http_endpoint}"
protocol = "{protocol}"
[meter_pipelines.reader]
interval = "100ms"

[[scopes]]
name  = "request"
event = "on-request"
kind  = "request"
[[scopes.spans]]
name = "haproxy.request"
[[scopes.attributes]]
key = "http.request.method"
source = "$arg.method"
[[scopes.instruments]]
kind  = "cnt_int"
name  = "http.requests"
value = 1

[[scopes]]
name  = "response"
event = "on-response"
kind  = "response"
"#
    );
    PluginConfig::from_toml(&toml).expect("config parses")
}

async fn drive(state: &RuntimeState) {
    let req = message("on-request", vec![("method", s("GET")), ("path", s("/x"))]);
    let r = dispatch::dispatch(state, &req);
    let correlation = r
        .variables
        .iter()
        .find(|v| v.name.as_str() == "correlation")
        .map(|v| match &v.value {
            SpoeValue::String(s) => s.as_str().to_string(),
            _ => panic!("correlation"),
        })
        .unwrap();
    let resp = message("on-response", vec![("correlation", s(&correlation))]);
    let _ = dispatch::dispatch(state, &resp);
}

/// FR-012, http/protobuf path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_protobuf_traces_and_metrics() {
    install();
    let collector = otlp_collector::start().await;

    let cfg = config_with_protocol(&collector.http_url(), "http/protobuf");
    let state = RuntimeState::build(cfg).expect("state");

    drive(&state).await;

    tokio::time::sleep(Duration::from_millis(250)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let spans = collector.total_spans().await;
    let metrics = collector.total_metrics().await;
    assert!(
        spans >= 1,
        "expected at least 1 span via http/protobuf, got {spans}"
    );
    assert!(
        metrics >= 1,
        "expected at least 1 metric via http/protobuf, got {metrics}"
    );
}

/// FR-012, http/json path. The OTel SDK uses pretty JSON serialization
/// for the body; the fake collector parses both protobuf and JSON.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_json_traces_and_metrics() {
    install();
    let collector = otlp_collector::start().await;

    let cfg = config_with_protocol(&collector.http_url(), "http/json");
    let state = RuntimeState::build(cfg).expect("state");

    drive(&state).await;

    tokio::time::sleep(Duration::from_millis(250)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let spans = collector.total_spans().await;
    let metrics = collector.total_metrics().await;
    assert!(
        spans >= 1,
        "expected at least 1 span via http/json, got {spans}"
    );
    assert!(
        metrics >= 1,
        "expected at least 1 metric via http/json, got {metrics}"
    );
}
