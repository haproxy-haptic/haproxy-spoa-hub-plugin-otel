//! In-memory correlation map for in-flight transactions (task T011).
//!
//! `DashMap` for sharded concurrent access (research item R4). TTL
//! eviction runs on a background tokio task; capacity-based eviction
//! happens synchronously inside `insert` when the cap is reached.

use super::handle::CorrelationHandle;
use crate::health::HealthRegistry;
use dashmap::DashMap;
use opentelemetry::Context;
use opentelemetry::trace::TraceContextExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per-transaction state held while the request-side message has been
/// processed but the response-side message has not yet arrived.
pub struct InFlightSpan {
    pub opened_at: Instant,
    /// Captured OpenTelemetry context (carries the active span and its
    /// parent context). Held so the response-side handler can attach
    /// child events / set the status / end the span correctly.
    pub context: Context,
    /// Pre-computed metric attributes captured at request-side, for
    /// instrument emission on response-side.
    pub meter_attributes: Vec<opentelemetry::KeyValue>,
}

impl InFlightSpan {
    pub fn new(context: Context) -> Self {
        Self {
            opened_at: Instant::now(),
            context,
            meter_attributes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EvictionReason {
    Ttl,
    Capacity,
    /// Span was evicted because the response-side message arrived and
    /// closed it. (Not really an "eviction" but uses the same path.)
    Completed,
}

pub struct CorrelationStore {
    map: DashMap<CorrelationHandle, InFlightSpan>,
    cap: usize,
    ttl: Duration,
    health: Arc<HealthRegistry>,
}

impl CorrelationStore {
    #[must_use]
    pub fn new(cap: usize, ttl: Duration, health: Arc<HealthRegistry>) -> Arc<Self> {
        Arc::new(Self {
            map: DashMap::new(),
            cap,
            ttl,
            health,
        })
    }

    /// Insert an entry, evicting the oldest if the cap is exceeded.
    /// Returns the handle as a convenience.
    pub fn insert(&self, handle: CorrelationHandle, span: InFlightSpan) -> CorrelationHandle {
        if self.map.len() >= self.cap {
            self.evict_oldest();
        }
        self.map.insert(handle, span);
        self.health.in_flight_set(self.map.len());
        handle
    }

    /// Remove and return the entry. The caller is responsible for
    /// finalizing the span.
    pub fn remove(&self, handle: &CorrelationHandle) -> Option<InFlightSpan> {
        let removed = self.map.remove(handle).map(|(_, v)| v);
        self.health.in_flight_set(self.map.len());
        removed
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Run a single sweep of TTL-based eviction. Each evicted span is
    /// finished with status `ERROR` and counted.
    pub fn sweep_ttl(&self) {
        let now = Instant::now();
        let ttl = self.ttl;
        let stale: Vec<_> = self
            .map
            .iter()
            .filter_map(|e| {
                if now.duration_since(e.value().opened_at) > ttl {
                    Some(*e.key())
                } else {
                    None
                }
            })
            .collect();

        for handle in stale {
            if let Some((_, span)) = self.map.remove(&handle) {
                Self::end_with_error(&span, "abandoned");
                self.health.eviction_inc(EvictionReason::Ttl);
            }
        }
        self.health.in_flight_set(self.map.len());
    }

    fn evict_oldest(&self) {
        // Linear scan for the oldest entry. Acceptable because eviction
        // by capacity is the unhappy path and the cap is bounded.
        let oldest = self
            .map
            .iter()
            .min_by_key(|e| e.value().opened_at)
            .map(|e| *e.key());
        if let Some(handle) = oldest {
            if let Some((_, span)) = self.map.remove(&handle) {
                Self::end_with_error(&span, "evicted");
                self.health.eviction_inc(EvictionReason::Capacity);
            }
        }
    }

    fn end_with_error(span: &InFlightSpan, reason: &'static str) {
        let s = span.context.span();
        s.set_attribute(opentelemetry::KeyValue::new(
            "otel.spoa.eviction_reason",
            reason,
        ));
        s.set_status(opentelemetry::trace::Status::error(reason));
        s.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn dummy_context() -> Context {
        // Empty context — no live span; sufficient for store tests.
        Context::new()
    }

    fn store_with_cap(cap: usize) -> Arc<CorrelationStore> {
        let health = HealthRegistry::new();
        CorrelationStore::new(cap, Duration::from_secs(60), health)
    }

    #[test]
    fn insert_and_remove() {
        let store = store_with_cap(10);
        let h = CorrelationHandle::generate();
        store.insert(h, InFlightSpan::new(dummy_context()));
        assert_eq!(store.len(), 1);
        assert!(store.remove(&h).is_some());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let store = store_with_cap(2);
        let h1 = CorrelationHandle::generate();
        store.insert(h1, InFlightSpan::new(dummy_context()));
        sleep(Duration::from_millis(2));
        let h2 = CorrelationHandle::generate();
        store.insert(h2, InFlightSpan::new(dummy_context()));
        sleep(Duration::from_millis(2));
        let h3 = CorrelationHandle::generate();
        store.insert(h3, InFlightSpan::new(dummy_context()));
        assert_eq!(store.len(), 2);
        assert!(
            store.remove(&h1).is_none(),
            "oldest should have been evicted"
        );
    }
}
