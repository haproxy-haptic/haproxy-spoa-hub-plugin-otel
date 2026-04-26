//! Verifies SC-008 — every operator-actionable failure mode is
//! observable in the plugin's health surface (task T056).

mod common;

use abi_stable::std_types::{RHashMap, RString};
use common::otlp_collector;
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

fn config_for_health(grpc_endpoint: &str) -> PluginConfig {
    let toml = format!(
        r#"
[instrumentation]
name = "health"

[runtime]
max_in_flight_transactions = 3
correlation_handle_ttl = "100ms"

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.sampler]
kind = "always_on"
[tracer_pipelines.exporter]
endpoint = "{grpc_endpoint}"
protocol = "grpc"
[tracer_pipelines.exporter.queue]
max_size = 8
scheduled_delay = "10ms"

[[scopes]]
name = "req"
event = "on-request"
kind = "request"
[[scopes.spans]]
name = "haproxy.request"

[[scopes]]
name  = "resp"
event = "on-response"
kind  = "response"
"#
    );
    PluginConfig::from_toml(&toml).expect("config")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn correlation_missing_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let state = RuntimeState::build(config_for_health(&collector.grpc_url())).expect("state");

    // Three response-side messages without correlation should bump the counter to 3.
    for _ in 0..3 {
        let _ = dispatch::dispatch(&state, &message("on-response", vec![]));
    }
    let snapshot = state.health.snapshot();
    assert_eq!(snapshot.correlation_missing_total, 3);

    collector.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capacity_eviction_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let state = RuntimeState::build(config_for_health(&collector.grpc_url())).expect("state");

    // Open 5 in-flight transactions when capacity is 3 → 2 should be
    // evicted with reason=capacity.
    for _ in 0..5 {
        let _ = dispatch::dispatch(&state, &message("on-request", vec![]));
    }

    let snapshot = state.health.snapshot();
    assert_eq!(
        snapshot.correlation_evictions_capacity, 2,
        "two evictions when 5 are inserted into a cap-3 store"
    );

    collector.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ttl_eviction_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let state = RuntimeState::build(config_for_health(&collector.grpc_url())).expect("state");

    // Open one transaction, wait past TTL (100ms), sweep manually.
    let _ = dispatch::dispatch(&state, &message("on-request", vec![]));
    tokio::time::sleep(Duration::from_millis(150)).await;
    state.correlation.sweep_ttl();

    let snapshot = state.health.snapshot();
    assert_eq!(snapshot.correlation_evictions_ttl, 1);
    assert_eq!(snapshot.in_flight_transactions, 0);

    collector.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_increments_counter() {
    install_propagator();
    let collector = otlp_collector::start().await;
    let state = RuntimeState::build(config_for_health(&collector.grpc_url())).expect("state");

    let req = dispatch::dispatch(&state, &message("on-request", vec![]));
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
        &message("on-response", vec![("correlation", s(&correlation))]),
    );

    let snapshot = state.health.snapshot();
    assert_eq!(snapshot.correlation_completions, 1);
    assert_eq!(snapshot.in_flight_transactions, 0);

    collector.shutdown();
}
