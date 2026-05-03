//! SPOE-message dispatcher skeleton (task T016).
//!
//! Looks up the active scope for an incoming SPOE message based on the
//! reserved `_event` argument, strips other reserved arguments, and
//! routes to the request-side or response-side handler. Phase 3 (US1)
//! fills in the actual dispatch logic.

use crate::config::PluginConfig;
use crate::correlation::CorrelationStore;
use crate::health::HealthRegistry;
use crate::pipelines::PipelineSet;
use haproxy_spoa_hub_plugin_api::{ProcessingResult, SpoeMessage};
use std::sync::Arc;

pub mod overrides;
mod request;
mod response;
mod scope;

/// Reserved SPOE argument names (see contracts/spoe-message-contract.md).
pub const ARG_EVENT: &str = "_event";
pub const ARG_CORRELATION: &str = "correlation";

pub use overrides::{ARG_ROOT, ARG_SAMPLE, OverrideResolution, SampleDecision};

/// Active runtime state. `ArcSwap` holds the current value; reload
/// installs a new state without disturbing in-flight spans (research R7).
pub struct RuntimeState {
    pub config: Arc<PluginConfig>,
    pub pipelines: Arc<PipelineSet>,
    pub correlation: Arc<CorrelationStore>,
    pub health: Arc<HealthRegistry>,
}

impl RuntimeState {
    pub fn build(config: PluginConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let health = HealthRegistry::new();
        let pipelines = PipelineSet::build(&config)?;
        let correlation = CorrelationStore::new(
            config.runtime.max_in_flight_transactions,
            config.runtime.correlation_handle_ttl,
            health.clone(),
        );
        Ok(Self {
            config: Arc::new(config),
            pipelines,
            correlation,
            health,
        })
    }
}

/// Process a SPOE message. Phase 2 returns no variables; Phase 3 onwards
/// dispatches to the request-side or response-side handler based on the
/// scope matched by `_event`.
pub fn dispatch(state: &RuntimeState, message: &SpoeMessage) -> ProcessingResult {
    let event = match message.get_string(ARG_EVENT) {
        Some(e) => e,
        None => {
            log::debug!("otel plugin: SPOE message lacks `_event` argument; ignoring");
            return ProcessingResult::from_vars(Vec::new());
        }
    };

    let scope = state.config.scopes.iter().find(|s| s.event == event);
    match scope {
        Some(scope) => scope::dispatch_scope(state, scope, message),
        None => {
            log::debug!("otel plugin: no scope matches event `{event}`; ignoring");
            ProcessingResult::from_vars(Vec::new())
        }
    }
}
