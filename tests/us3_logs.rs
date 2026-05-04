//! User Story 3 — Structured log records via OTLP (tasks T041–T042).

mod common;

use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{RHashMap, RString};
use haproxy_spoa_hub_plugin_api::{SpoeMessage, SpoeValue};
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

/// US3 acceptance scenario 1 — log rule WARN with body and attributes. (T041)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warn_log_record_emitted() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[logger_pipelines]]
name = "default"

[logger_pipelines.exporter]
endpoint = "{}"
protocol = "grpc"
[logger_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"

[[scopes]]
name = "rejected"
event = "on-tcp-reject"
kind  = "request"

[[scopes.log_records]]
severity = "WARN"
body = "connection rejected"

[[scopes.log_records.attributes]]
key = "client.address"
source = "$arg.src"
"#,
        collector.grpc_url()
    );
    let cfg = PluginConfig::from_toml(&toml).expect("config");
    let state = RuntimeState::build(cfg).expect("state");

    let _ = dispatch::dispatch(
        &state,
        &message("on-tcp-reject", vec![("src", s("203.0.113.42"))]),
    );

    tokio::time::sleep(Duration::from_millis(150)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let mut found = 0;
    for export in &sink.log_exports {
        for rl in &export.resource_logs {
            for sl in &rl.scope_logs {
                for record in &sl.log_records {
                    found += 1;
                    assert_eq!(record.severity_text, "WARN");
                    let has_client_attr = record.attributes.iter().any(|kv| {
                        kv.key == "client.address"
                            && match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
                                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => s == "203.0.113.42",
                                _ => false,
                            }
                    });
                    assert!(
                        has_client_attr,
                        "client.address attribute present on log record"
                    );
                }
            }
        }
    }
    assert_eq!(found, 1, "exactly one log record exported");
}

/// US3 acceptance scenario 2 — response-side log record carries the
/// active span's trace and span IDs. (T042)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn log_record_carries_active_span_context() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.exporter]
endpoint = "{0}"
protocol = "grpc"
[tracer_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"
[tracer_pipelines.sampler]
kind = "always_on"

[[logger_pipelines]]
name = "default"
[logger_pipelines.exporter]
endpoint = "{0}"
protocol = "grpc"
[logger_pipelines.exporter.queue]
max_size = 64
scheduled_delay = "10ms"

[[scopes]]
name = "request"
event = "on-request"
kind  = "request"
[[scopes.spans]]
name = "haproxy.request"

[[scopes]]
name  = "response"
event = "on-response"
kind  = "response"
[[scopes.log_records]]
severity = "INFO"
body = "request done"
"#,
        collector.grpc_url()
    );
    let cfg = PluginConfig::from_toml(&toml).expect("config");
    let state = RuntimeState::build(cfg).expect("state");

    let req = message("on-request", vec![("path", s("/x"))]);
    let r = dispatch::dispatch(&state, &req);
    let correlation = r
        .variables
        .iter()
        .find(|v| v.name.as_str() == "correlation")
        .map(|v| match &v.value {
            SpoeValue::String(s) => s.as_str().to_string(),
            _ => panic!("correlation should be a string"),
        })
        .expect("correlation set");

    let resp = message("on-response", vec![("correlation", s(&correlation))]);
    let _ = dispatch::dispatch(&state, &resp);

    tokio::time::sleep(Duration::from_millis(150)).await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let mut log_trace_id = Vec::new();
    let mut log_span_id = Vec::new();
    for export in &sink.log_exports {
        for rl in &export.resource_logs {
            for sl in &rl.scope_logs {
                for record in &sl.log_records {
                    log_trace_id = record.trace_id.clone();
                    log_span_id = record.span_id.clone();
                }
            }
        }
    }
    let mut span_trace_id = Vec::new();
    let mut span_span_id = Vec::new();
    for export in &sink.trace_exports {
        for rs in &export.resource_spans {
            for ss in &rs.scope_spans {
                for span in &ss.spans {
                    span_trace_id = span.trace_id.clone();
                    span_span_id = span.span_id.clone();
                }
            }
        }
    }
    assert!(!log_trace_id.is_empty(), "log record carries a trace id");
    assert_eq!(
        log_trace_id, span_trace_id,
        "log trace id matches the span's"
    );
    assert_eq!(log_span_id, span_span_id, "log span id matches the span's");
}
