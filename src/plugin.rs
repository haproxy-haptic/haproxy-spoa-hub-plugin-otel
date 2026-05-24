//! `SpoePlugin` trait implementation (task T013).
//!
//! Holds the active `RuntimeState` behind an `ArcSwap` so SIGHUP-driven
//! reload swaps the configuration atomically (research item R7). Spans
//! that are still in flight retain a clone of the previous `RuntimeState`
//! through the in-flight-span entry's captured `Arc<PipelineSet>`, so
//! the old SDK providers stay alive until the last in-flight span ends.
//!
//! The hub calls `init()` from outside a tokio runtime context, but the
//! OTLP+tonic exporter constructs a hyper Client at build time and that
//! needs a reactor. The plugin therefore owns a private multi-thread
//! tokio runtime stored in a `OnceLock`; pipeline construction enters
//! that runtime so the batch processor tasks get spawned on it, and
//! they continue running there for the plugin's lifetime.

use crate::config::PluginConfig;
use crate::dispatch::{self, RuntimeState};
use crate::propagation::default_propagator;
use arc_swap::ArcSwap;
use haproxy_spoa_hub_plugin_api::{MetricRecorder, ProcessingResult, define_plugin};

/// Plugin-scoped metric recorder; silent no-op on pre-v0.6 hubs.
static METRICS: MetricRecorder = MetricRecorder::new();
use std::sync::{Arc, OnceLock};

static STATE: OnceLock<ArcSwap<RuntimeState>> = OnceLock::new();
static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn ensure_runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("otel-plugin")
            .build()
            .expect("failed to start tokio runtime for otel plugin")
    })
}

#[derive(Debug)]
struct OtelPlugin;

define_plugin!(OtelPlugin, {
    fn new() -> Self {
        OtelPlugin
    }

    fn init(
        &mut self,
        context: &PluginContext,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Install the default text-map propagator (W3C TraceContext + Baggage)
        // exactly once per process. Subsequent reloads keep the same
        // propagator stack.
        opentelemetry::global::set_text_map_propagator(default_propagator());

        let config = PluginConfig::from_context(context).map_err(|e| {
            log::error!("otel plugin: configuration rejected: {e}");
            Box::<dyn std::error::Error + Send + Sync>::from(e.render())
        })?;

        // Enter the plugin-owned tokio runtime for the duration of pipeline
        // construction. The batch processors / metric readers / log
        // processors built here spawn long-running tasks on this runtime
        // via `tokio::spawn`; those tasks keep running after we exit the
        // guard because the runtime itself stays alive in TOKIO_RT.
        let rt = ensure_runtime();
        let _guard = rt.enter();

        let state = Arc::new(RuntimeState::build(config)?);

        match STATE.get() {
            Some(swap) => {
                let old = swap.swap(state);
                old.health.record_reload(true);
                old.pipelines
                    .drain(old.config.runtime.shutdown_drain_timeout);
            }
            None => {
                STATE
                    .set(ArcSwap::new(state))
                    .map_err(|_| "otel plugin: STATE already initialised")?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "otel"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn process(
        &self,
        message: &SpoeMessage,
    ) -> Result<ProcessingResult, Box<dyn std::error::Error + Send + Sync>> {
        let started = std::time::Instant::now();
        let Some(swap) = STATE.get() else {
            METRICS.counter("dispatches_total", &[("outcome", "uninitialised")], 1);
            return Ok(ProcessingResult::from_vars(Vec::new()));
        };
        // Enter the plugin-owned runtime so any synchronous SDK calls
        // (e.g. dropping a BoxedSpan, which schedules an export task)
        // see a tokio reactor.
        let _guard = ensure_runtime().enter();
        let state = swap.load();
        let result = dispatch::dispatch(&state, message);

        // Hub prefixes everything with `plugin_otel_`. `outcome` is
        // "processed" — there's no error path in dispatch itself; the
        // underlying OTLP exporter handles its own retry/drop with
        // separate metrics surfaced by the opentelemetry-sdk crate
        // (which we don't intercept).
        METRICS.counter("dispatches_total", &[("outcome", "processed")], 1);
        METRICS.histogram(
            "dispatch_duration_seconds",
            &[],
            started.elapsed().as_secs_f64(),
        );

        Ok(result)
    }

    fn config_schema(&self) -> Option<&str> {
        Some(crate::config::schema::SCHEMA_JSON)
    }

    fn drain(&self, timeout_ms: u64) -> bool {
        // Hub calls this between the registry swap and shutdown_all on
        // a config reload (haproxy-spoa-hub#3, vtable v3 — see
        // haproxy-spoa-hub@0.7.0). The OpenTelemetry SDK pipelines we
        // host (BatchSpanProcessor, PeriodicReader, BatchLogProcessor)
        // run on tokio tasks against the plugin-owned runtime; their
        // pending exports complete AFTER the SPOE ACK has returned.
        // Without this override the runtime would be dropped mid-flight
        // on hub reload and any queued spans / metric data points /
        // log records would be lost.
        //
        // Delegates to the existing `PipelineSet::drain`, which was
        // already wired for the in-plugin reload path (when `init()`
        // runs a config-swap on the same plugin instance). The hub-
        // initiated drain reuses the same primitive — same SDK
        // shutdown calls flush the same queues.
        //
        // Timeout semantics: this function returns `true` once the
        // SDK shutdown calls have completed. `PipelineSet::drain` does
        // not currently enforce the timeout itself (the underlying
        // SDK shutdown calls have their own internal timeouts via the
        // BatchSpanProcessorBuilder / PeriodicReaderBuilder configs),
        // so a slow OTLP backend could exceed `timeout_ms`. The hub
        // measures real-time against the deadline regardless and
        // increments `spoa_hub_config_reload_drain_forced_total{
        // plugin="otel"}` if drain runs over — but this function
        // never returns `false` itself. Follow-up work to enforce the
        // timeout via `tokio::time::timeout` around the shutdown
        // calls can refine that.
        let Some(swap) = STATE.get() else {
            // init() never ran or STATE never populated — nothing to drain.
            return true;
        };
        let state = swap.load();
        state
            .pipelines
            .drain(std::time::Duration::from_millis(timeout_ms));
        true
    }

    metrics_static = METRICS;
});
