//! SDK provider construction per pipeline (tasks T025, T036, T043).
//!
//! Phase 2 supplies the registry shape and the empty-state defaults; the
//! actual provider builds land in Phases 3–5 with each user story.

pub mod logger;
pub mod meter;
pub mod tracer;

use crate::config::PluginConfig;
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::TracerProvider;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Built pipeline set; one instance per active config. Held by every
/// `InFlightSpan` via `Arc` so the SDK providers stay alive across
/// reload until the last in-flight span finishes (research item R7).
#[derive(Default)]
pub struct PipelineSet {
    pub tracers: BTreeMap<String, Arc<TracerProvider>>,
    pub meters: BTreeMap<String, Arc<SdkMeterProvider>>,
    pub loggers: BTreeMap<String, Arc<LoggerProvider>>,
}

impl PipelineSet {
    /// Build all providers from the configured pipelines.
    pub fn build(
        config: &PluginConfig,
    ) -> Result<Arc<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let mut tracers = BTreeMap::new();
        for p in &config.tracer_pipelines {
            tracers.insert(p.name.clone(), tracer::build(p)?);
        }
        let mut meters = BTreeMap::new();
        for p in &config.meter_pipelines {
            meters.insert(p.name.clone(), meter::build(p)?);
        }
        let mut loggers = BTreeMap::new();
        for p in &config.logger_pipelines {
            loggers.insert(p.name.clone(), logger::build(p)?);
        }
        Ok(Arc::new(Self {
            tracers,
            meters,
            loggers,
        }))
    }

    /// Resolve a tracer pipeline by name, falling back to "default" when
    /// the spec does not explicitly name one.
    pub fn tracer_or_default(&self, name: Option<&str>) -> Option<&Arc<TracerProvider>> {
        let key = name.unwrap_or("default");
        self.tracers
            .get(key)
            .or_else(|| self.tracers.get("default"))
            .or_else(|| self.tracers.values().next())
    }

    /// Resolve a meter pipeline by name with the same fallback policy.
    pub fn meter_or_default(&self, name: Option<&str>) -> Option<&Arc<SdkMeterProvider>> {
        let key = name.unwrap_or("default");
        self.meters
            .get(key)
            .or_else(|| self.meters.get("default"))
            .or_else(|| self.meters.values().next())
    }

    /// Resolve a logger pipeline by name with the same fallback policy.
    pub fn logger_or_default(&self, name: Option<&str>) -> Option<&Arc<LoggerProvider>> {
        let key = name.unwrap_or("default");
        self.loggers
            .get(key)
            .or_else(|| self.loggers.get("default"))
            .or_else(|| self.loggers.values().next())
    }

    pub fn tracer(&self, name: &str) -> Option<&Arc<TracerProvider>> {
        self.tracers.get(name)
    }
    pub fn meter(&self, name: &str) -> Option<&Arc<SdkMeterProvider>> {
        self.meters.get(name)
    }
    pub fn logger(&self, name: &str) -> Option<&Arc<LoggerProvider>> {
        self.loggers.get(name)
    }

    /// Drain all providers within a timeout. Called by `shutdown()` and
    /// by the reload path when an old `PipelineSet` is no longer
    /// referenced.
    pub fn drain(&self, _timeout: std::time::Duration) {
        for p in self.tracers.values() {
            let _ = p.shutdown();
        }
        for p in self.meters.values() {
            let _ = p.shutdown();
        }
        for p in self.loggers.values() {
            let _ = p.shutdown();
        }
    }
}
