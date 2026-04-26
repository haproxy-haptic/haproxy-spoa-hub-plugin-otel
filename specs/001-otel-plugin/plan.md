# Implementation Plan: OpenTelemetry SPOA Hub Plugin

**Branch**: `001-otel-plugin` | **Date**: 2026-04-26 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-otel-plugin/spec.md`

## Summary

Build a Rust `cdylib` plugin for `haproxy-spoa-hub` that emits OpenTelemetry traces, metrics, and logs from SPOE messages received from HAProxy. The plugin's TOML configuration vocabulary mirrors, one-to-one, the directive names and nesting structure of HAProxy's built-in OTEL addon, supports any number of tracer/meter/logger pipelines per signal, exports via OTLP/gRPC and OTLP/HTTP (Protobuf and JSON), and propagates W3C TraceContext + Baggage. Performance budget: ≤ 1 ms added p95 / ≤ 5 ms added p99 at 10,000 req/s sustained per hub instance, fail-open with respect to HAProxy traffic, per-pipeline bounded queues with drop-oldest on overflow.

## Technical Context

**Language/Version**: Rust stable ≥ 1.86 (the haproxy-spoa-hub baseline; sibling plugins set this same MSRV).

**Primary Dependencies**:

- `abi_stable` — FFI plugin contract required by the hub (`spoa_plugin` root-module symbol, `define_plugin!` macro from the hub SDK)
- `tokio` — async runtime, version-aligned with the hub
- `opentelemetry`, `opentelemetry-sdk` — API + SDK (current stable)
- `opentelemetry-otlp` with features `grpc-tonic`, `http-proto`, `http-json`, `tls`, `gzip-tonic`
- `opentelemetry-semantic-conventions` — canonical attribute name constants
- `serde`, `toml` — TOML config (per hub conventions)
- `jsonschema` — config schema validation (matching the fingerprinting plugin's pattern)
- `dashmap` — concurrent correlation map (request-side handle → in-flight span state)
- `log` — emission (the hub installs a tracing-subscriber bridge)
- `thiserror`, `anyhow` — error types
- Dev: `criterion` (benchmarks), in-process fake OTLP collector built on `tonic`/`hyper` for integration tests

**Storage**: N/A. In-memory, hub-instance-local correlation map only. Bounded by configurable `max_in_flight_transactions` with oldest-evicting LRU policy and a per-handle TTL to bound memory under any traffic pattern.

**Testing**: `cargo test` for unit and integration; `criterion` for benchmarks. Integration tests run an in-process OTLP/gRPC and OTLP/HTTP collector that records every received export and exposes assertions; no network, deterministic per Constitution Principle II.

**Target Platform**: Linux x86_64, aarch64, armv7. glibc ≥ 2.36 (matches sibling plugin binary matrix on Debian bookworm).

**Project Type**: Rust dynamic library (`cdylib`); single crate. Loaded by `haproxy-spoa-hub` at runtime via `abi_stable` from the produced `.so`.

**Performance Goals**: 10,000 sustained requests per second per hub instance (spec clarification Q3). At that operating point:

- Added per-request latency ≤ 1 ms p95, ≤ 5 ms p99 (SC-002; constitution Performance Standards default)
- Steady-state RSS growth ≤ 2 MB/min at 95% CI over a 5-minute soak at the harness rate (default 2,000 req/s on single-host localhost; SC-006). Bounded-by-construction tests (correlation map cap + TTL eviction, per-pipeline export queues with drop-oldest, deterministic provider drop on reload) cover the structural leak paths; the soak catches slow per-request escapees. Per-request overhead at the SC-002 anchor (10,000 req/s) is verified separately by criterion benchmarks (~10 µs/request).
- Resumes export within one export interval after a 10-minute collector outage (SC-003)

**Constraints**:

- Fail-open with respect to HAProxy traffic at all times (FR-014). Telemetry production from the SPOE handler is non-blocking; export I/O happens on tokio worker threads behind bounded channels.
- Per-pipeline bounded queues with drop-oldest semantics; queue depth and drop count exposed per pipeline (FR-015, FR-020).
- No `.unwrap()` / `.expect()` on configuration paths. Configuration errors MUST name file, line, TOML key path, and explanation (FR-018, SC-007).
- TLS for OTLP exporters with safe defaults; explicit opt-out required for each unsafe choice (FR-013).
- The hub's `process()` is synchronous on the blocking thread pool. The plugin's hot path produces telemetry items synchronously and hands them to async exporters via channels.
- Hub-instance-local correlation map. Multi-hub deployments require operator-configured session affinity (spec Assumptions).

**Scale/Scope**: 4 user stories (P1 → P4), 22 functional requirements, 8 success criteria, 8 edge cases. v1 covers tracing + metrics + logs + multi-pipeline + W3C TraceContext/Baggage propagation + TOML mirror of the addon's directive vocabulary. HAProxy lifecycle events that cannot be fired as SPOE messages are explicitly out of scope for v1 (spec Assumption).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Checked against `.specify/memory/constitution.md` v1.0.0.

| Principle / Section | Gate | Status |
| --- | --- | --- |
| I. Code Quality (NON-NEGOTIABLE) | clippy + rustfmt zero warnings; documented public API; no dead code; deduplicated helpers; complexity thresholds not raised | **PASS** — standard Rust toolchain. Plugin is one crate; helpers stay shared. CI inherits from sibling-plugin templates. |
| II. Testing Standards (NON-NEGOTIABLE) | every FR maps to a test; each user story has an integration test; no flake tolerance; hermetic external dependencies | **PASS** — research item R5 defines an in-process fake OTLP collector; integration test plan covers all four user stories; FR-to-test traceability is enumerated in `tasks.md`. |
| III. User Experience Consistency | mirrored directive names; structured error messages; OTel semantic conventions; consistent log levels | **PASS** — FR-016 forces directive-vocabulary parity; FR-018 forces structured errors; `opentelemetry-semantic-conventions` provides canonical attribute names; log-level guidance follows constitution. |
| IV. Performance Requirements | declared per-feature budget; benchmarks for hot-path changes; bounded resources; async telemetry; safe defaults, no unbounded defaults | **PASS** — budget declared above; criterion benchmarks for the SPOE-message hot path; per-pipeline bounded queues with drop-oldest; export is async behind a channel; defaults documented and capped. |
| Performance Standards & Reliability | ≤ 1 ms p95 / ≤ 5 ms p99 added latency at target throughput; bounded RSS; fail-open; self-observability | **PASS** — same as above; FR-020 specifies the self-observability surface (queue depths, drop counts, export attempts/failures, last-success timestamps per pipeline). |
| Development Workflow & Quality Gates | spec-driven flow; Constitution Check gate; pre-merge gate; commit hygiene | **PASS** — flow is being followed; merge gates inherit from the sibling plugins' CI configuration. |

No violations. Complexity Tracking section below is intentionally empty.

**Post-Phase 1 re-evaluation (2026-04-26)**: Re-checked after `research.md`,
`data-model.md`, `contracts/`, and `quickstart.md` were produced. No new
violations. The Phase 1 artifacts strengthen the gates rather than weakening them:
research item R3 commits to a non-blocking hot path, R4 bounds the correlation
map with TTL + LRU, the JSON Schema's `additionalProperties: false` makes FR-017
enforceable mechanically, and the SPOE-message contract pins the public API
surface explicitly. Gates remain PASS.

## Project Structure

### Documentation (this feature)

```text
specs/001-otel-plugin/
├── plan.md                                # This file
├── research.md                            # Phase 0 output
├── data-model.md                          # Phase 1 output
├── quickstart.md                          # Phase 1 output
├── contracts/
│   ├── plugin-config-schema.json          # JSON Schema for [plugins.params]
│   ├── spoe-message-contract.md           # SPOE args read + return variables written
│   └── addon-directive-mapping.md         # one-to-one map: addon directive → TOML key path
├── checklists/
│   └── requirements.md                    # already exists
├── spec.md                                # already exists
└── tasks.md                               # produced by /speckit-tasks
```

### Source Code (repository root)

```text
Cargo.toml                      # crate manifest; [lib] crate-type = ["cdylib", "rlib"]
src/
├── lib.rs                      # define_plugin! invocation; plugin entry point
├── plugin.rs                   # SpoePlugin trait impl: init, process, shutdown, config_schema
├── config/
│   ├── mod.rs
│   ├── schema.rs               # JSON Schema string returned from config_schema()
│   ├── instrumentation.rs      # otel-instrumentation block
│   ├── scope.rs                # otel-scope block (spans, attrs, events, instruments, log-records)
│   ├── group.rs                # otel-group block
│   ├── pipeline.rs             # exporter pipelines (tracer/meter/logger arrays)
│   └── sampler.rs              # sampler config
├── correlation/
│   ├── mod.rs
│   ├── handle.rs               # opaque CorrelationHandle (random 128-bit, hex-encoded)
│   └── store.rs                # DashMap<CorrelationHandle, InFlightSpan> with TTL/LRU
├── propagation/
│   ├── mod.rs
│   ├── extractor.rs            # SpoeArgsExtractor implements opentelemetry::propagation::Extractor
│   └── injector.rs             # SpoeVarsInjector implements opentelemetry::propagation::Injector
├── pipelines/
│   ├── mod.rs                  # PipelineRegistry: maps pipeline name → SDK provider
│   ├── tracer.rs               # SdkTracerProvider construction per pipeline
│   ├── meter.rs                # SdkMeterProvider construction per pipeline
│   └── logger.rs               # SdkLoggerProvider construction per pipeline
├── dispatch/
│   ├── mod.rs
│   ├── request.rs              # request-side handler: open span, emit correlation handle
│   ├── response.rs             # response-side handler: close span, emit telemetry
│   └── scope.rs                # apply scope rules (attrs, events, instruments, log records)
├── attribute/
│   └── mod.rs                  # SpoeValue → opentelemetry::KeyValue conversion
└── health/
    └── mod.rs                  # self-observability surface, exposed via plugin metrics

tests/
├── integration/
│   ├── common/
│   │   └── otlp_collector.rs   # in-process fake OTLP/gRPC + OTLP/HTTP collector
│   ├── us1_tracing.rs          # User Story 1: distributed tracing
│   ├── us2_metrics.rs          # User Story 2: metrics
│   ├── us3_logs.rs             # User Story 3: logs
│   └── us4_addon_parity.rs     # User Story 4: directive-vocabulary parity
├── contract/
│   └── spoe_messages.rs        # SPOE message contract (args read, vars written)
└── golden/
    └── *.toml                  # golden TOML configs derived from real addon configs

benches/
└── hot_path.rs                 # criterion benches: request-side, response-side, end-to-end
```

**Structure Decision**: Single Rust crate following the haproxy-spoa-hub plugin convention (sibling: `haproxy-spoa-hub-plugin-fingerprinting`). The crate produces a `cdylib` consumed by the hub via `abi_stable`. Source modules are organised by responsibility — configuration, correlation, propagation, signal pipelines, dispatch, attribute conversion, health. Integration tests stand up an in-process OTLP collector; no network is touched in any test.

## Complexity Tracking

(intentionally empty — Constitution Check passed without violations)
