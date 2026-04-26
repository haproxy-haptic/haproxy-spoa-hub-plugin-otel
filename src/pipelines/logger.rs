//! Logger pipeline construction (task T043, T024 HTTP transport).

use crate::config::{LoggerPipeline, Protocol};
use opentelemetry_otlp::{LogExporter, Protocol as OtlpProtocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::runtime;
use std::sync::Arc;

/// Build a `LoggerProvider` for one logger pipeline.
pub fn build(
    pipeline: &LoggerPipeline,
) -> Result<Arc<LoggerProvider>, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = build_exporter(pipeline)?;
    let resource = super::tracer::build_resource(&pipeline.resource);
    let provider = LoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter, runtime::Tokio)
        .build();
    Ok(Arc::new(provider))
}

fn build_exporter(
    pipeline: &LoggerPipeline,
) -> Result<LogExporter, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = pipeline.exporter.endpoint.as_str();
    match pipeline.exporter.protocol {
        Protocol::Grpc => {
            let builder = LogExporter::builder()
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
            let signal_endpoint = super::tracer::append_signal_path(endpoint, "/v1/logs");
            let mut builder = LogExporter::builder()
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
