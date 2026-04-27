//! Tracer pipeline construction (task T025).

use crate::config::{Compression, Exporter as ExporterCfg, Protocol, Sampler, TracerPipeline};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    Compression as OtlpCompression, Protocol as OtlpProtocol, SpanExporter, WithExportConfig,
    WithHttpConfig, WithTonicConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler as SdkSampler, SdkTracerProvider};
use std::sync::Arc;

/// Build an `SdkTracerProvider` for one tracer pipeline.
pub fn build(
    pipeline: &TracerPipeline,
) -> Result<Arc<SdkTracerProvider>, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = build_exporter(&pipeline.exporter, &pipeline.exporter.protocol)?;
    let sampler = build_sampler(&pipeline.sampler);
    let resource = build_resource(&pipeline.resource);

    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_sampler(sampler)
        .with_batch_exporter(exporter)
        .build();

    Ok(Arc::new(provider))
}

/// Map our `Sampler` config enum into the SDK's `Sampler`.
pub fn build_sampler(s: &Sampler) -> SdkSampler {
    match s {
        Sampler::AlwaysOn => SdkSampler::AlwaysOn,
        Sampler::AlwaysOff => SdkSampler::AlwaysOff,
        Sampler::TraceIdRatio { ratio } => SdkSampler::TraceIdRatioBased(*ratio),
        Sampler::ParentBased { root } => SdkSampler::ParentBased(Box::new(build_sampler(root))),
    }
}

/// Build a `Resource` from a key/value map. Each value is coerced to its
/// natural OTel type via `serde_json::Value` shape.
pub fn build_resource(kvs: &std::collections::BTreeMap<String, serde_json::Value>) -> Resource {
    let pairs: Vec<KeyValue> = kvs
        .iter()
        .filter_map(|(k, v)| match v {
            serde_json::Value::String(s) => Some(KeyValue::new(k.clone(), s.clone())),
            serde_json::Value::Bool(b) => Some(KeyValue::new(k.clone(), *b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(KeyValue::new(k.clone(), i))
                } else {
                    n.as_f64().map(|f| KeyValue::new(k.clone(), f))
                }
            }
            _ => None,
        })
        .collect();
    Resource::builder().with_attributes(pairs).build()
}

fn build_exporter(
    cfg: &ExporterCfg,
    protocol: &Protocol,
) -> Result<SpanExporter, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = cfg.endpoint.as_str();
    match protocol {
        Protocol::Grpc => {
            let mut builder = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(cfg.timeout);
            if let Some(c) = map_compression(cfg.compression) {
                builder = builder.with_compression(c);
            }
            if !cfg.headers.is_empty() {
                let mut md = tonic::metadata::MetadataMap::new();
                for (k, v) in &cfg.headers {
                    if let (Ok(name), Ok(value)) = (
                        k.parse::<tonic::metadata::MetadataKey<_>>(),
                        v.parse::<tonic::metadata::MetadataValue<_>>(),
                    ) {
                        md.insert(name, value);
                    }
                }
                builder = builder.with_metadata(md);
            }
            Ok(builder.build()?)
        }
        Protocol::HttpProtobuf | Protocol::HttpJson => {
            let otlp_protocol = match protocol {
                Protocol::HttpJson => OtlpProtocol::HttpJson,
                _ => OtlpProtocol::HttpBinary,
            };
            let signal_endpoint = append_signal_path(endpoint, "/v1/traces");
            let mut builder = SpanExporter::builder()
                .with_http()
                .with_endpoint(signal_endpoint)
                .with_timeout(cfg.timeout)
                .with_protocol(otlp_protocol);
            if !cfg.headers.is_empty() {
                builder = builder.with_headers(cfg.headers.clone().into_iter().collect());
            }
            Ok(builder.build()?)
        }
    }
}

/// Append the OTLP/HTTP signal-specific path to a base endpoint URL.
/// The OTel SDK treats `with_endpoint(...)` as the full per-signal URL
/// (matching `OTEL_EXPORTER_OTLP_<signal>_ENDPOINT` semantics) and does
/// not append `/v1/traces` etc. itself. To keep the operator's config
/// surface to one endpoint per pipeline, we append the path here unless
/// the operator has already done so.
pub(super) fn append_signal_path(base: &str, path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(path) {
        return trimmed.to_string();
    }
    format!("{trimmed}{path}")
}

fn map_compression(c: Compression) -> Option<OtlpCompression> {
    match c {
        Compression::None => None,
        Compression::Gzip => Some(OtlpCompression::Gzip),
        Compression::Zstd => Some(OtlpCompression::Zstd),
    }
}
