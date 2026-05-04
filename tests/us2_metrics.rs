//! User Story 2 — HAProxy-flow metrics via OTLP (tasks T033–T035).

mod common;

use common::otlp_collector;
use haproxy_spoa_hub_plugin_api::{RHashMap, RString};
use haproxy_spoa_hub_plugin_api::{SpoeMessage, SpoeValue};
use opentelemetry_proto::tonic::metrics::v1::metric::Data as MetricData;
use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value as DPValue;
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

fn config_with_metrics(grpc_endpoint: &str, instruments_toml: &str) -> PluginConfig {
    let toml = format!(
        r#"
[instrumentation]
name = "test"

[[meter_pipelines]]
name = "default"

[meter_pipelines.exporter]
endpoint = "{grpc_endpoint}"
protocol = "grpc"
timeout  = "5s"

[meter_pipelines.reader]
interval = "100ms"
timeout  = "1s"

[meter_pipelines.resource]
"service.name" = "us2-test"

[[scopes]]
name = "request"
event = "on-request"
kind = "request"

{instruments_toml}
"#
    );
    PluginConfig::from_toml(&toml).expect("config parses")
}

async fn flush_metrics() {
    // Wait one reader interval + a bit for export.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// US2 acceptance scenario 1 — counter `http.requests` keyed on attributes. (T033)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counter_records_per_attribute_combination() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let cfg = config_with_metrics(
        &collector.grpc_url(),
        r#"
[[scopes.instruments]]
kind = "cnt_int"
name = "http.requests"
value = 1

[[scopes.instruments.attributes]]
key = "status_class"
source = "$arg.status_class"
"#,
    );
    let state = RuntimeState::build(cfg).expect("state");

    for _ in 0..30 {
        let m = message("on-request", vec![("status_class", s("2xx"))]);
        let _ = dispatch::dispatch(&state, &m);
    }
    for _ in 0..7 {
        let m = message("on-request", vec![("status_class", s("5xx"))]);
        let _ = dispatch::dispatch(&state, &m);
    }

    flush_metrics().await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let mut by_class: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for export in &sink.metric_exports {
        for rm in &export.resource_metrics {
            for sm in &rm.scope_metrics {
                for metric in &sm.metrics {
                    if metric.name != "http.requests" {
                        continue;
                    }
                    if let Some(MetricData::Sum(sum)) = &metric.data {
                        for dp in &sum.data_points {
                            let class = dp
                                .attributes
                                .iter()
                                .find(|kv| kv.key == "status_class")
                                .and_then(|kv| kv.value.as_ref())
                                .and_then(|v| v.value.as_ref())
                                .and_then(|v| match v {
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            let count = match dp.value {
                                Some(DPValue::AsInt(i)) => i as u64,
                                Some(DPValue::AsDouble(f)) => f as u64,
                                None => 0,
                            };
                            *by_class.entry(class).or_insert(0) = count;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(by_class.get("2xx"), Some(&30), "2xx counter should be 30");
    assert_eq!(by_class.get("5xx"), Some(&7), "5xx counter should be 7");
}

/// US2 acceptance scenario 2 — histogram with explicit bounds reflects
/// the served distribution. (T034)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn histogram_records_distribution() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let cfg = config_with_metrics(
        &collector.grpc_url(),
        r#"
[[scopes.instruments]]
kind = "hist_int"
name = "http.server.duration"
unit = "ms"
value = "$arg.duration_ms"
bounds = [10, 50, 100]
"#,
    );
    let state = RuntimeState::build(cfg).expect("state");

    let durations = [5_i64, 30, 30, 80, 200, 200, 200];
    for d in durations {
        let m = message("on-request", vec![("duration_ms", SpoeValue::Int64(d))]);
        let _ = dispatch::dispatch(&state, &m);
    }

    flush_metrics().await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let mut sample_count = 0u64;
    for export in &sink.metric_exports {
        for rm in &export.resource_metrics {
            for sm in &rm.scope_metrics {
                for metric in &sm.metrics {
                    if metric.name != "http.server.duration" {
                        continue;
                    }
                    if let Some(MetricData::Histogram(h)) = &metric.data {
                        for dp in &h.data_points {
                            sample_count = dp.count;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        sample_count,
        durations.len() as u64,
        "histogram saw all samples"
    );
}

/// US2 acceptance scenario 3 — up-down counter tracks current value. (T035)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updown_counter_increments_and_decrements() {
    install_propagator();
    let collector = otlp_collector::start().await;

    let cfg = config_with_metrics(
        &collector.grpc_url(),
        r#"
[[scopes.instruments]]
kind = "udcnt_int"
name = "http.in_flight"
value = "$arg.delta"
"#,
    );
    let state = RuntimeState::build(cfg).expect("state");

    for _ in 0..10 {
        let m = message("on-request", vec![("delta", SpoeValue::Int64(1))]);
        let _ = dispatch::dispatch(&state, &m);
    }
    for _ in 0..3 {
        let m = message("on-request", vec![("delta", SpoeValue::Int64(-1))]);
        let _ = dispatch::dispatch(&state, &m);
    }

    flush_metrics().await;
    state.pipelines.drain(Duration::from_secs(2));
    collector.shutdown();

    let sink = collector.sink.lock().await;
    let mut sum: i64 = 0;
    for export in &sink.metric_exports {
        for rm in &export.resource_metrics {
            for sm in &rm.scope_metrics {
                for metric in &sm.metrics {
                    if metric.name != "http.in_flight" {
                        continue;
                    }
                    if let Some(MetricData::Sum(s)) = &metric.data {
                        for dp in &s.data_points {
                            if let Some(DPValue::AsInt(i)) = dp.value {
                                sum = i;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(sum, 7, "in-flight = 10 increments - 3 decrements = 7");
}
