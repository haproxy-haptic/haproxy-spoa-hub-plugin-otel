//! Meter pipeline construction (task T036, T024 HTTP transport).

use crate::config::{MeterPipeline, Protocol};
use opentelemetry_otlp::{
    MetricExporter, Protocol as OtlpProtocol, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::runtime;
use std::sync::Arc;

/// Build an `SdkMeterProvider` for one meter pipeline.
pub fn build(
    pipeline: &MeterPipeline,
) -> Result<Arc<SdkMeterProvider>, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = build_exporter(pipeline)?;
    let reader = PeriodicReader::builder(exporter, runtime::Tokio)
        .with_interval(pipeline.reader.interval)
        .with_timeout(pipeline.reader.timeout)
        .build();
    let resource = super::tracer::build_resource(&pipeline.resource);
    let provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .build();
    Ok(Arc::new(provider))
}

fn build_exporter(
    pipeline: &MeterPipeline,
) -> Result<MetricExporter, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = pipeline.exporter.endpoint.as_str();
    match pipeline.exporter.protocol {
        Protocol::Grpc => {
            let builder = MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(pipeline.exporter.timeout);
            Ok(builder.build()?)
        }
        Protocol::HttpProtobuf | Protocol::HttpJson => {
            let otlp_protocol = match pipeline.exporter.protocol {
                Protocol::HttpJson => OtlpProtocol::HttpJson,
                _ => OtlpProtocol::HttpBinary,
            };
            let signal_endpoint = super::tracer::append_signal_path(endpoint, "/v1/metrics");
            let mut builder = MetricExporter::builder()
                .with_http()
                .with_endpoint(signal_endpoint)
                .with_timeout(pipeline.exporter.timeout)
                .with_protocol(otlp_protocol);
            if !pipeline.exporter.headers.is_empty() {
                builder =
                    builder.with_headers(pipeline.exporter.headers.clone().into_iter().collect());
            }
            Ok(builder.build()?)
        }
    }
}
