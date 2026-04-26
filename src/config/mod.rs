//! Configuration tree (tasks T007, T008, T009).
//!
//! See `specs/001-otel-plugin/data-model.md` §1 for the schema and
//! `contracts/plugin-config-schema.json` for the JSON Schema this code
//! mirrors.

use crate::error::ConfigError;
use haproxy_spoa_hub_plugin_api::{ConfigValue, PluginContext};
use serde::Deserialize;

mod group;
mod instrumentation;
mod pipeline;
mod sampler;
pub mod schema;
mod scope;

pub use group::Group;
pub use instrumentation::{AclRule, Instrumentation, InstrumentationOption, LogPolicy, RateLimit};
pub use pipeline::{
    AggregationPolicy, Compression, Exporter, LoggerPipeline, MeterPipeline, MetricReader,
    Protocol, QueueConfig, RetryConfig, TlsConfig, TracerPipeline,
};
pub use sampler::Sampler;
pub use scope::{
    AttributeRule, BaggageRule, EventRule, ExtractRule, FinishRule, InjectRule, InstrumentKind,
    InstrumentRule, LinkRule, LogRecordRule, Scope, ScopeKind, SeverityNumber, SpanRule,
    StatusCode, StatusRule, ValueSource,
};

/// Top-level plugin configuration. Mirrors `data-model.md` §1.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// The single `otel-instrumentation` block.
    pub instrumentation: Instrumentation,

    /// Tracer pipelines. At least one if any scope emits spans.
    #[serde(default)]
    pub tracer_pipelines: Vec<TracerPipeline>,

    /// Meter pipelines.
    #[serde(default)]
    pub meter_pipelines: Vec<MeterPipeline>,

    /// Logger pipelines.
    #[serde(default)]
    pub logger_pipelines: Vec<LoggerPipeline>,

    /// All `otel-scope` blocks.
    #[serde(default)]
    pub scopes: Vec<Scope>,

    /// All `otel-group` blocks.
    #[serde(default)]
    pub groups: Vec<Group>,

    /// Plugin-wide resource attributes (applied to every pipeline that
    /// does not override).
    #[serde(default)]
    pub resource: std::collections::BTreeMap<String, serde_json::Value>,

    /// Hub-local non-addon settings (queue caps, drain timeout, log level).
    #[serde(default)]
    pub runtime: RuntimeKnobs,
}

/// Hub-local configuration that has no addon equivalent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeKnobs {
    /// Maximum number of in-flight transactions kept in the correlation map.
    #[serde(default = "RuntimeKnobs::default_max_in_flight")]
    pub max_in_flight_transactions: usize,

    /// TTL for an in-flight transaction (request → response window).
    #[serde(default = "RuntimeKnobs::default_correlation_ttl")]
    #[serde(with = "humantime_serde")]
    pub correlation_handle_ttl: std::time::Duration,

    /// How long `shutdown()` waits for the SDK exporters to drain.
    #[serde(default = "RuntimeKnobs::default_drain_timeout")]
    #[serde(with = "humantime_serde")]
    pub shutdown_drain_timeout: std::time::Duration,
}

impl Default for RuntimeKnobs {
    fn default() -> Self {
        Self {
            max_in_flight_transactions: Self::default_max_in_flight(),
            correlation_handle_ttl: Self::default_correlation_ttl(),
            shutdown_drain_timeout: Self::default_drain_timeout(),
        }
    }
}

impl RuntimeKnobs {
    fn default_max_in_flight() -> usize {
        100_000
    }
    fn default_correlation_ttl() -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }
    fn default_drain_timeout() -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }
}

impl PluginConfig {
    /// Build a `PluginConfig` from the hub's typed `PluginContext` config
    /// table. Returns a structured error naming the failing key path on
    /// validation failure (FR-018, SC-007).
    pub fn from_context(context: &PluginContext) -> Result<Self, ConfigError> {
        let json = config_table_to_json(&context.config);
        Self::from_json(json)
    }

    /// Validate against the embedded JSON Schema and deserialize.
    fn from_json(value: serde_json::Value) -> Result<Self, ConfigError> {
        schema::validate(&value)?;
        let config: Self = serde_json::from_value(value).map_err(|e| ConfigError::Schema {
            key_path: format!("line {}, column {}", e.line(), e.column()),
            message: e.to_string(),
        })?;
        config.validate_invariants()?;
        Ok(config)
    }

    /// Build directly from a TOML string. Used by tests and by operator
    /// translation tooling. Production builds from `PluginContext`.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let toml: toml::Value = text
            .parse()
            .map_err(|e: toml::de::Error| ConfigError::Toml {
                message: e.to_string(),
            })?;
        let json = toml_to_json(toml);
        Self::from_json(json)
    }

    /// Cross-field semantic validation (e.g. scope references a pipeline
    /// that exists). Pipeline name resolution to default-named pipelines
    /// happens here, not at deserialize time.
    fn validate_invariants(&self) -> Result<(), ConfigError> {
        for scope in &self.scopes {
            for s in &scope.spans {
                if let Some(name) = &s.tracer_pipeline {
                    if !self.tracer_pipelines.iter().any(|p| p.name == *name) {
                        return Err(ConfigError::Invariant {
                            message: format!(
                                "scope `{}` span `{}` references unknown tracer pipeline `{}`",
                                scope.name, s.name, name
                            ),
                        });
                    }
                }
            }
        }
        for group in &self.groups {
            for s in &group.scopes {
                if !self.scopes.iter().any(|sc| sc.name == *s) {
                    return Err(ConfigError::Invariant {
                        message: format!("group `{}` references unknown scope `{}`", group.name, s),
                    });
                }
            }
        }
        Ok(())
    }
}

fn config_table_to_json(
    table: &abi_stable::std_types::RHashMap<abi_stable::std_types::RString, ConfigValue>,
) -> serde_json::Value {
    use abi_stable::std_types::Tuple2;
    let mut map = serde_json::Map::with_capacity(table.len());
    for Tuple2(k, v) in table.iter() {
        map.insert(k.as_str().to_string(), config_value_to_json(v));
    }
    serde_json::Value::Object(map)
}

fn config_value_to_json(v: &ConfigValue) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        ConfigValue::String(s) => J::String(s.as_str().to_string()),
        ConfigValue::Integer(i) => J::Number(serde_json::Number::from(*i)),
        ConfigValue::Float(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
        ConfigValue::Bool(b) => J::Bool(*b),
        ConfigValue::Array(a) => J::Array(a.iter().map(config_value_to_json).collect()),
        ConfigValue::Table(t) => {
            use abi_stable::std_types::Tuple2;
            let mut m = serde_json::Map::with_capacity(t.len());
            for Tuple2(k, v) in t.iter() {
                m.insert(k.as_str().to_string(), config_value_to_json(v));
            }
            J::Object(m)
        }
        ConfigValue::__Other(_, _) => J::Null,
    }
}

fn toml_to_json(v: toml::Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        toml::Value::String(s) => J::String(s),
        toml::Value::Integer(i) => J::Number(serde_json::Number::from(i)),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or(J::Null, J::Number),
        toml::Value::Boolean(b) => J::Bool(b),
        toml::Value::Datetime(dt) => J::String(dt.to_string()),
        toml::Value::Array(a) => J::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            let mut m = serde_json::Map::with_capacity(t.len());
            for (k, v) in t {
                m.insert(k, toml_to_json(v));
            }
            J::Object(m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let cfg = PluginConfig::from_toml("[instrumentation]\nname = \"test\"\n").unwrap();
        assert_eq!(cfg.instrumentation.name, "test");
        assert_eq!(cfg.runtime.max_in_flight_transactions, 100_000);
    }

    #[test]
    fn unknown_top_level_key_rejected() {
        let err =
            PluginConfig::from_toml("[instrumentation]\nname = \"x\"\nfooBaz = 1\n").unwrap_err();
        assert!(matches!(err, ConfigError::Schema { .. }));
    }

    #[test]
    fn invariant_unknown_scope_in_group() {
        let toml = r#"
[instrumentation]
name = "i"

[[groups]]
name = "g"
scopes = ["does-not-exist"]
"#;
        let err = PluginConfig::from_toml(toml).unwrap_err();
        assert!(matches!(err, ConfigError::Invariant { .. }));
    }
}
