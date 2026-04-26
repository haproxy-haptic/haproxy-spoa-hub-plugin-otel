//! Smoke test for the fake OTLP collector fixture (task T015).
//!
//! Confirms the fixture starts, accepts a gRPC export, records it, and
//! shuts down. Each user-story integration test file builds on top.

mod common;

use common::otlp_collector;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span, Tracer, TracerProvider as _};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::TracerProvider;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collector_records_an_exported_span() {
    let collector = otlp_collector::start().await;

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(collector.grpc_url())
        .with_protocol(Protocol::Grpc)
        .build()
        .expect("build span exporter");

    let provider = TracerProvider::builder()
        .with_resource(Resource::new(vec![KeyValue::new("service.name", "smoke")]))
        .with_batch_exporter(exporter, runtime::Tokio)
        .build();

    let tracer = provider.tracer("smoke");
    let mut span = tracer.start("smoke-span");
    span.end();

    provider.shutdown().expect("shutdown provider");

    // Wait briefly for the batch exporter to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(collector.total_spans().await, 1);
    collector.shutdown();
}
