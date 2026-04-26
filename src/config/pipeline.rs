//! Tracer / Meter / Logger pipelines and their exporter configs
//! (data-model.md §1; addon mapping in
//! contracts/addon-directive-mapping.md §4).

use super::sampler::Sampler;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracerPipeline {
    pub name: String,
    #[serde(default)]
    pub sampler: Sampler,
    pub exporter: Exporter,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPipeline {
    pub name: String,
    #[serde(default)]
    pub reader: MetricReader,
    #[serde(default)]
    pub aggregations: Vec<AggregationPolicy>,
    pub exporter: Exporter,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggerPipeline {
    pub name: String,
    pub exporter: Exporter,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricReader {
    #[serde(default = "MetricReader::default_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default = "MetricReader::default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
}

impl MetricReader {
    fn default_interval() -> Duration {
        Duration::from_secs(60)
    }
    fn default_timeout() -> Duration {
        Duration::from_secs(10)
    }
}

impl Default for MetricReader {
    fn default() -> Self {
        Self {
            interval: Self::default_interval(),
            timeout: Self::default_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregationPolicy {
    pub instrument_name: String,
    pub aggregation: AggregationKind,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationKind {
    Sum,
    LastValue,
    Histogram,
    ExpHistogram,
    Drop,
    Default,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exporter {
    pub endpoint: Url,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub compression: Compression,
    #[serde(default = "Exporter::default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub retry: RetryConfig,
    #[serde(default)]
    pub queue: QueueConfig,
}

impl Exporter {
    fn default_timeout() -> Duration {
        Duration::from_secs(10)
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    #[default]
    Grpc,
    #[serde(rename = "http/protobuf")]
    HttpProtobuf,
    #[serde(rename = "http/json")]
    HttpJson,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    #[default]
    None,
    Gzip,
    Zstd,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    #[serde(default)]
    pub ca: Option<std::path::PathBuf>,
    #[serde(default)]
    pub client_cert: Option<std::path::PathBuf>,
    #[serde(default)]
    pub client_key: Option<std::path::PathBuf>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub insecure_skip_verify: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    #[serde(default = "RetryConfig::default_enabled")]
    pub enabled: bool,
    #[serde(default = "RetryConfig::default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "RetryConfig::default_initial", with = "humantime_serde")]
    pub initial_backoff: Duration,
    #[serde(default = "RetryConfig::default_max", with = "humantime_serde")]
    pub max_backoff: Duration,
}

impl RetryConfig {
    fn default_enabled() -> bool {
        true
    }
    fn default_max_attempts() -> u32 {
        5
    }
    fn default_initial() -> Duration {
        Duration::from_secs(1)
    }
    fn default_max() -> Duration {
        Duration::from_secs(30)
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            enabled: Self::default_enabled(),
            max_attempts: Self::default_max_attempts(),
            initial_backoff: Self::default_initial(),
            max_backoff: Self::default_max(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default = "QueueConfig::default_max_size")]
    pub max_size: usize,
    #[serde(default = "QueueConfig::default_batch_size")]
    pub max_export_batch_size: usize,
    #[serde(default = "QueueConfig::default_delay", with = "humantime_serde")]
    pub scheduled_delay: Duration,
}

impl QueueConfig {
    fn default_max_size() -> usize {
        2048
    }
    fn default_batch_size() -> usize {
        512
    }
    fn default_delay() -> Duration {
        Duration::from_secs(5)
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_size: Self::default_max_size(),
            max_export_batch_size: Self::default_batch_size(),
            scheduled_delay: Self::default_delay(),
        }
    }
}
