//! Self-observability surface (task T014).
//!
//! Exposes counters and gauges that satisfy spec FR-020 and SC-008. The
//! plugin holds one `HealthRegistry` per running instance; the values
//! are scraped through the hub's existing Prometheus endpoint via
//! callbacks the hub will wire when it queries the plugin.

use crate::correlation::EvictionReason;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// One key per (signal, pipeline) tuple. Keeps the surface flat and
/// cheap to iterate at scrape time.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PipelineKey {
    pub signal: Signal,
    pub pipeline: String,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Signal {
    Trace,
    Metric,
    Log,
}

impl Signal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Signal::Trace => "trace",
            Signal::Metric => "metric",
            Signal::Log => "log",
        }
    }
}

/// All counters/gauges; built once at plugin init and shared via `Arc`.
pub struct HealthRegistry {
    in_flight_transactions: AtomicI64,
    correlation_evictions_ttl: AtomicU64,
    correlation_evictions_capacity: AtomicU64,
    correlation_completions: AtomicU64,
    correlation_missing_total: AtomicU64,
    config_reload_total: AtomicU64,
    config_reload_failures_total: AtomicU64,
    pipelines: dashmap::DashMap<PipelineKey, PipelineHealth>,
}

#[derive(Default)]
pub struct PipelineHealth {
    pub queue_depth: AtomicI64,
    pub dropped_total: AtomicU64,
    pub export_attempts_total: AtomicU64,
    pub export_failures_total: AtomicU64,
    pub last_export_success_unix_seconds: AtomicI64,
}

impl HealthRegistry {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_flight_transactions: AtomicI64::new(0),
            correlation_evictions_ttl: AtomicU64::new(0),
            correlation_evictions_capacity: AtomicU64::new(0),
            correlation_completions: AtomicU64::new(0),
            correlation_missing_total: AtomicU64::new(0),
            config_reload_total: AtomicU64::new(0),
            config_reload_failures_total: AtomicU64::new(0),
            pipelines: dashmap::DashMap::new(),
        })
    }

    pub fn in_flight_set(&self, n: usize) {
        self.in_flight_transactions
            .store(n as i64, Ordering::Relaxed);
    }

    pub fn eviction_inc(&self, reason: EvictionReason) {
        let counter = match reason {
            EvictionReason::Ttl => &self.correlation_evictions_ttl,
            EvictionReason::Capacity => &self.correlation_evictions_capacity,
            EvictionReason::Completed => &self.correlation_completions,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn correlation_missing_inc(&self) {
        self.correlation_missing_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_reload(&self, success: bool) {
        if success {
            self.config_reload_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.config_reload_failures_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn pipeline(
        &self,
        key: &PipelineKey,
    ) -> dashmap::mapref::one::RefMut<'_, PipelineKey, PipelineHealth> {
        self.pipelines.entry(key.clone()).or_default()
    }

    /// Snapshot of the current state, intended for export to the hub's
    /// metrics endpoint.
    pub fn snapshot(&self) -> HealthSnapshot {
        let pipelines = self
            .pipelines
            .iter()
            .map(|e| {
                let h = e.value();
                (
                    e.key().clone(),
                    PipelineSnapshot {
                        queue_depth: h.queue_depth.load(Ordering::Relaxed),
                        dropped_total: h.dropped_total.load(Ordering::Relaxed),
                        export_attempts_total: h.export_attempts_total.load(Ordering::Relaxed),
                        export_failures_total: h.export_failures_total.load(Ordering::Relaxed),
                        last_export_success_unix_seconds: h
                            .last_export_success_unix_seconds
                            .load(Ordering::Relaxed),
                    },
                )
            })
            .collect();
        HealthSnapshot {
            in_flight_transactions: self.in_flight_transactions.load(Ordering::Relaxed),
            correlation_evictions_ttl: self.correlation_evictions_ttl.load(Ordering::Relaxed),
            correlation_evictions_capacity: self
                .correlation_evictions_capacity
                .load(Ordering::Relaxed),
            correlation_completions: self.correlation_completions.load(Ordering::Relaxed),
            correlation_missing_total: self.correlation_missing_total.load(Ordering::Relaxed),
            config_reload_total: self.config_reload_total.load(Ordering::Relaxed),
            config_reload_failures_total: self.config_reload_failures_total.load(Ordering::Relaxed),
            pipelines,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    pub in_flight_transactions: i64,
    pub correlation_evictions_ttl: u64,
    pub correlation_evictions_capacity: u64,
    pub correlation_completions: u64,
    pub correlation_missing_total: u64,
    pub config_reload_total: u64,
    pub config_reload_failures_total: u64,
    pub pipelines: std::collections::HashMap<PipelineKey, PipelineSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PipelineSnapshot {
    pub queue_depth: i64,
    pub dropped_total: u64,
    pub export_attempts_total: u64,
    pub export_failures_total: u64,
    pub last_export_success_unix_seconds: i64,
}
