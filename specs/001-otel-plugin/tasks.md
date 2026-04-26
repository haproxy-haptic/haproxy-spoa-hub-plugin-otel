---
description: "Task list for the OpenTelemetry SPOA Hub Plugin (feature 001-otel-plugin)"
---

# Tasks: OpenTelemetry SPOA Hub Plugin

**Input**: Design documents from `/specs/001-otel-plugin/`
**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [data-model.md](./data-model.md), [research.md](./research.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: Tests are MANDATORY per Constitution Principle II (Testing Standards, NON-NEGOTIABLE). Every functional requirement maps to at least one automated test before the corresponding code is merged. Tests use the in-process fake OTLP collector (research item R5).

**Organization**: Tasks are grouped by user story so each story can be implemented and tested independently as an MVP increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Different file, no dependencies on other in-flight tasks → may run in parallel
- **[Story]**: User story label (US1–US4); only on user-story-phase tasks
- File paths are absolute relative to the repository root

## Path Conventions

- Single Rust crate (`cdylib` + `rlib`) at the repository root, per `plan.md` Project Structure.
- Source: `src/`, tests: `tests/`, benchmarks: `benches/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project skeleton, dependencies, lint/format/CI; nothing that requires architectural decisions beyond what `plan.md` already pins.

- [X] T001 Create `Cargo.toml` at repository root declaring `[package]`, `[lib] crate-type = ["cdylib", "rlib"]`, MSRV 1.86, and the dependency set from `plan.md` Technical Context (`abi_stable`, `tokio`, `opentelemetry`, `opentelemetry-sdk`, `opentelemetry-otlp` with features `grpc-tonic, http-proto, http-json, tls, gzip-tonic`, `opentelemetry-semantic-conventions`, `serde`, `toml`, `jsonschema`, `dashmap`, `log`, `thiserror`, `anyhow`; dev-dependencies `criterion`, `tonic`, `hyper`, `axum`)
- [X] T002 [P] ~~Create `rustfmt.toml` and `clippy.toml`~~ — sibling fingerprinting plugin uses default rustfmt/clippy settings (no per-project overrides). Matched that pattern intentionally; no files created.
- [X] T003 [P] Create `.gitignore` and empty `deny.toml` mirroring the sibling fingerprinting plugin
- [X] T004 [P] Create minimal `.gitlab-ci.yml` running `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and a release build. Full cross-compilation matrix (amd64/arm64/armv7) is deferred to T058 per Phase 7.
- [X] T005 Scaffold `src/lib.rs` and `src/plugin.rs` with a no-op `define_plugin!` invocation (mirrors sibling pattern). Local `cargo check` not run — no Rust toolchain installed on this machine; CI validates on push.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Cross-story plumbing that every user story depends on. No user-story phase may begin until this phase is complete.

**CRITICAL**: All user stories require this foundation.

- [X] T006 [P] Implement `SpoeValue` → `opentelemetry::KeyValue` conversion (typed, with optional operator-specified coercion per research item R10) in `src/attribute/mod.rs`; include the per-rule error counter for type-mismatch cases
- [X] T007 Define configuration types (`PluginConfig`, `Instrumentation`, `Scope`, `Group`, `TracerPipeline`, `MeterPipeline`, `LoggerPipeline`, `Exporter`, `Sampler`, `RuntimeKnobs`, plus all rule structs from `data-model.md` §1) as `serde::Deserialize` structs in `src/config/mod.rs`, `src/config/instrumentation.rs`, `src/config/scope.rs`, `src/config/group.rs`, `src/config/pipeline.rs`, `src/config/sampler.rs`
- [X] T008 Embed the JSON Schema string in `src/config/schema.rs` derived from `contracts/plugin-config-schema.json` (`additionalProperties: false` everywhere so FR-017 is enforced by the schema, not by hand) and return it from `SpoePlugin::config_schema()`
- [X] T009 Implement TOML → `PluginConfig` parsing in `src/config/mod.rs` returning structured errors (file path, line, TOML key path, human-readable explanation) per FR-018; define the error type in `src/error.rs` using `thiserror`
- [X] T010 [P] Implement `CorrelationHandle` (128-bit random, hex-encoded, 32-character lowercase string) in `src/correlation/handle.rs`
- [X] T011 Implement `CorrelationStore` (`DashMap<CorrelationHandle, InFlightSpan>` with TTL eviction via a `tokio::time::interval` task and capacity-bound LRU eviction) in `src/correlation/store.rs` per research item R4 and `data-model.md` §2; expose `correlation_evictions_by_ttl` and `correlation_evictions_by_capacity` counters
- [X] T012 [P] Implement `SpoeArgsExtractor` (reads `traceparent`, `tracestate`, `baggage` from SPOE arguments) and `SpoeVarsInjector` (writes them as session-scoped return variables) implementing `opentelemetry::propagation::Extractor` / `Injector` in `src/propagation/extractor.rs` and `src/propagation/injector.rs`; register `TraceContextPropagator` and `BaggagePropagator` as the global composite per research items R12 and R6
- [X] T013 Implement the `SpoePlugin` trait (init, process, shutdown, config_schema) in `src/plugin.rs`; the trait impl owns an `ArcSwap<RuntimeState>` so reload swaps the active config atomically while keeping in-flight spans against their captured `RuntimeState`; shutdown drains pending telemetry up to `runtime.shutdown_drain_timeout` per FR-019
- [X] T014 Implement the self-observability surface (counters, gauges per `data-model.md` §2 `HealthSnapshot`) in `src/health/mod.rs`, exported through the hub's existing Prometheus endpoint per FR-020
- [X] T015 [P] Build the in-process fake OTLP collector test fixture (gRPC via `tonic` + HTTP via `axum`/`hyper`) in `tests/integration/common/otlp_collector.rs` per research item R5; expose `assert_received_span`, `assert_received_metric`, `assert_received_log` helpers and `inject_failure(...)` for backpressure/error-path tests
- [X] T016 Implement scope-dispatcher skeleton (`SpoeMessage._event` → matching `Scope` lookup) in `src/dispatch/mod.rs`, with explicit handling for the reserved `_event` and `correlation` argument names and the `correlation_missing_total` counter wiring from `contracts/spoe-message-contract.md` §C

**Checkpoint**: Foundation ready; user stories may now proceed in priority order or in parallel.

---

## Phase 3: User Story 1 - Distributed tracing for HTTP traffic (Priority: P1) 🎯 MVP

**Goal**: Operators see one OTLP span per HTTP request, with correct W3C TraceContext parent inheritance and onward propagation, exported to a configurable OTLP endpoint, with per-pipeline bounded queues and fail-open semantics.

**Independent Test**: per `spec.md` US1 Independent Test — synthetic request with a `traceparent` header through HAProxy → exactly one span in the fake collector with matching trace ID, configured attributes, and an updated `traceparent` on the backend leg.

### Tests for User Story 1

> **Per Constitution Principle II, write these tests FIRST and verify they FAIL before implementation.**

- [X] T017 [P] [US1] Contract test for the SPOE message contract (request-side and response-side messages: required args, return variables, `_event` reservation) in `tests/contract/spoe_messages.rs`
- [X] T018 [P] [US1] Integration test US1 acceptance scenario 1 (no incoming `traceparent` → single root span with operator-configured attributes `http.request.method`, `url.path`, `server.address`, `client.address`, `http.response.status_code`) in `tests/integration/us1_tracing.rs`
- [X] T019 [P] [US1] Integration test US1 acceptance scenario 2 (incoming `traceparent` → child span shares trace ID, references inbound span as parent; `traceparent_out` return variable carries the new HAProxy span ID) in `tests/integration/us1_tracing.rs`
- [X] T020 [P] [US1] Integration test US1 acceptance scenario 3 (probabilistic sampler at 10% over 10,000 requests within statistical tolerance; parent-based inheritance honoured for parent-sampled requests) in `tests/integration/us1_tracing.rs`
- [X] T021 [P] [US1] Integration test US1 acceptance scenario 4 + FR-014 fail-open (collector unreachable for 10 minutes via `inject_failure(connection_refused)` on the fake collector while sustaining synthetic load → no request rejection or added latency, drop counters increment, export resumes within one interval after recovery) in `tests/integration/us1_tracing.rs`
- [X] T022 [P] [US1] Integration test US1 acceptance scenario 5 + FR-021 reload (signal config swap mid-flight; in-flight spans finish and export against captured config; new requests use new config) in `tests/integration/us1_tracing.rs`

### Implementation for User Story 1

- [X] T023 [P] [US1] Implement `SamplerConfig` parsing (`always_on`, `always_off`, `trace_id_ratio`, `parent_based(...)`) and SDK `Sampler` construction in `src/config/sampler.rs` per research item R11 (default `parent_based(trace_id_ratio_based(1.0))`)
- [X] T024 [P] [US1] Implement `Exporter` config parsing — endpoint, protocol (`grpc` / `http/protobuf` / `http/json` per FR-012), compression, timeout, headers, TLS (CA, mTLS, server-name override per FR-013 and research R8), retry, queue (`max_size` default 2048, drop-oldest semantics per FR-015) — in `src/config/pipeline.rs`
- [X] T025 [US1] Implement `TracerPipeline` construction in `src/pipelines/tracer.rs` — build `SdkTracerProvider` with batch span processor, configured sampler, configured OTLP exporter, and resource attributes, returning an `Arc<SdkTracerProvider>` (per research item R3)
- [X] T026 [US1] Implement `PipelineRegistry` (map pipeline name → `Arc<SdkTracerProvider>`) in `src/pipelines/mod.rs`; the registry is owned by `RuntimeState` and held by every `InFlightSpan` to keep pipelines alive across reloads per research item R7
- [X] T027 [US1] Implement the request-side handler in `src/dispatch/request.rs` — extract incoming `traceparent`/`tracestate`/`baggage` via `SpoeArgsExtractor`, build span via `Tracer::span_builder().start_with_context()`, generate `CorrelationHandle`, store `InFlightSpan` (holding `Arc<TracerProvider>`), inject updated context via `SpoeVarsInjector` to return variables `traceparent_out`, `tracestate_out`, `baggage_out`, and emit `correlation` return variable
- [X] T028 [US1] Implement the response-side handler in `src/dispatch/response.rs` — look up `InFlightSpan` by `correlation` argument, apply scope rules, set status from `status` rule, end span (which the SDK batch processor exports asynchronously), remove the entry from `CorrelationStore`
- [X] T029 [US1] Implement scope-rule application in `src/dispatch/scope.rs` for spans (attributes from `$arg.<name>`, events from event rules, status from condition, links from link rules) per `data-model.md` §1 `Scope`
- [X] T030 [US1] Implement reload tolerance per research item R7 — reload swaps `Arc<RuntimeState>` atomically; in-flight `InFlightSpan` instances retain their captured `Arc<PipelineRegistry>`, so old providers are dropped only when their last span finishes
- [X] T031 [US1] Implement edge cases: malformed `traceparent` → fall back to new root trace (no exception, no rejected request); `tracestate` truncation per W3C rules; both behaviours covered by added negative tests in `tests/integration/us1_tracing.rs`
- [X] T032 [US1] Implement edge case from `contracts/spoe-message-contract.md` §C — response-side message without `correlation` argument → emit minimal span with `status=ERROR` and attribute `otel.spoa.correlation_missing=true`, increment `correlation_missing_total` counter

**Checkpoint**: User Story 1 is fully functional and testable independently. MVP shipped.

---

## Phase 4: User Story 2 - HAProxy-flow metrics via OTLP (Priority: P2)

**Goal**: Operators emit counter, up-down-counter, histogram (explicit and exponential bounds), and gauge instruments to OTLP, with attribute dimensions sourced from SPOE arguments.

**Independent Test**: per `spec.md` US2 Independent Test — configure one of each instrument type, drive synthetic load, verify the fake collector receives the expected aggregated data points with the configured dimensions at the configured export interval.

### Tests for User Story 2

- [X] T033 [P] [US2] Integration test US2 acceptance scenario 1 (counter `http.requests` keyed on `status_class` and `frontend`; 500 successful + 50 failed requests → correct sums per attribute combination) in `tests/integration/us2_metrics.rs`
- [X] T034 [P] [US2] Integration test US2 acceptance scenario 2 (histogram `http.server.duration` with explicit bucket bounds; mixed-latency requests → bucket counts and sum reflect the served distribution within one export interval) in `tests/integration/us2_metrics.rs`
- [X] T035 [P] [US2] Integration test US2 acceptance scenario 3 (up-down counter for in-flight requests; concurrent traffic → gauge tracks current count, returns to zero when traffic stops) in `tests/integration/us2_metrics.rs`

### Implementation for User Story 2

- [X] T036 [P] [US2] Implement `MeterPipeline` construction in `src/pipelines/meter.rs` — `SdkMeterProvider` with periodic reader (`interval` default 60 s, `timeout` default 10 s), aggregation policy override, OTLP exporter, resource attributes
- [X] T037 [US2] Implement `InstrumentRule` parsing and instrument construction in `src/dispatch/scope.rs` — counter (`cnt_int`), up-down counter (`udcnt_int`), histogram (`hist_int` with explicit bounds and exp_histogram support), gauge (`gauge_int`)
- [X] T038 [US2] Implement instrument value emission from `$arg.<name>` sources at scope-evaluation time in `src/dispatch/scope.rs`; respect optional `condition` per rule
- [X] T039 [US2] Implement aggregation policy override per pipeline (`sum`, `last_value`, `histogram`, `exp_histogram`, `drop`, `default`) in `src/pipelines/meter.rs`
- [X] T040 [US2] Wire up metric attribute dimensions from each rule's `attributes` array (dimensions from `$arg.<name>` or literals); ensure `KeyValue` types align with `SpoeValue` via the converter from T006

**Checkpoint**: Both User Stories 1 and 2 work independently.

---

## Phase 5: User Story 3 - Structured log records via OTLP (Priority: P3)

**Goal**: Operators emit OTLP log records with operator-configured severity, body, attributes, optionally associated with the active HAProxy span for the same transaction.

**Independent Test**: per `spec.md` US3 Independent Test — configure a log-record rule on a chosen event, drive a request, verify the fake collector receives a log record with severity, body, attributes, and (when within a sampled trace) trace + span IDs linking to the live span.

### Tests for User Story 3

- [X] T041 [P] [US3] Integration test US3 acceptance scenario 1 (log rule severity `WARN` on a `tcp-request` rejection; collector receives one log record with the configured body and attributes) in `tests/integration/us3_logs.rs`
- [X] T042 [P] [US3] Integration test US3 acceptance scenario 2 (same event during a sampled HTTP request; log record's trace and span IDs match the active span) in `tests/integration/us3_logs.rs`

### Implementation for User Story 3

- [X] T043 [P] [US3] Implement `LoggerPipeline` construction in `src/pipelines/logger.rs` — `SdkLoggerProvider` with batch log processor, OTLP exporter, resource attributes
- [X] T044 [US3] Implement `LogRecordRule` parsing and emission in `src/dispatch/scope.rs` — severity (full set `TRACE..FATAL` with numbered variants per addon vocabulary), body, attributes, optional `event_name`, `id`, `span` reference
- [X] T045 [US3] Wire log record → active-span context association in `src/dispatch/scope.rs`: when emitted within an `InFlightSpan` window, attach the span's `trace_id` and `span_id` to the log record before handing it to the SDK

**Checkpoint**: User Stories 1, 2, and 3 work independently.

---

## Phase 6: User Story 4 - Mechanical configuration parity with the built-in OTEL addon (Priority: P4)

**Goal**: The plugin's TOML key paths cover every directive the addon exposes within the supported scope; unsupported directives are rejected with named errors; a documented one-to-one mapping enables mechanical translation in either direction.

**Independent Test**: per `spec.md` US4 Independent Test — translate a real built-in-addon configuration to the plugin's TOML using the published mapping, hand it to the plugin, verify the resulting telemetry matches what the addon would emit for the same HAProxy traffic.

### Tests for User Story 4

- [X] T046 [P] [US4] Integration test US4 acceptance scenario 1 (TOML mirror of a valid addon configuration → plugin starts cleanly, exposes equivalent scopes/groups, emits equivalent telemetry) in `tests/integration/us4_addon_parity.rs`
- [X] T047 [P] [US4] Integration test US4 acceptance scenario 2 (TOML key path corresponding to an unimplemented addon directive → plugin refuses to start, error names both the addon-vocabulary form and the TOML key path, references migration notes) in `tests/integration/us4_addon_parity.rs`
- [X] T048 [P] [US4] Integration test US4 acceptance scenario 3 (apply published TOML-to-addon translation on a working plugin config, run the resulting addon-syntax pipeline through a reference renderer, verify telemetry-equivalence assertions hold) in `tests/integration/us4_addon_parity.rs`
- [X] T049 [P] [US4] Add golden TOML configurations derived from real addon configurations in `tests/golden/*.toml` covering: minimal tracing, multi-pipeline tracing+metrics+logs, all sampler kinds, all instrument kinds

### Implementation for User Story 4

- [X] T050 [US4] Complete the TOML key-path coverage from `contracts/addon-directive-mapping.md` — every addon directive listed there has a parser path in `src/config/`; missing parsers are added until the mapping is exhaustive for in-scope directives
- [X] T051 [US4] Implement FR-017 enforcement in `src/config/mod.rs` — JSON-Schema validation rejects unknown TOML keys; error formatter looks up the offending TOML key path against the addon-mapping table and emits both forms in the error message
- [X] T052 [P] [US4] Document the TOML-to-addon translation rules in `README.md`, deep-linking to `specs/001-otel-plugin/contracts/addon-directive-mapping.md` so operators have a top-level entry point

**Checkpoint**: All four user stories functional and independently testable.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Verify success criteria from `spec.md`, finalize operator-facing artifacts, lock CI quality gates.

- [X] T053 [P] Implement criterion benchmarks for the hot path in `benches/hot_path.rs` — request-side, response-side, and end-to-end at the 10,000 req/s sustained operating point; assert SC-002 ceilings (≤ 1 ms added p95, ≤ 5 ms added p99) on a documented reference machine; benchmark output committed alongside each PR per Constitution Principle IV
- [X] T054 [P] 5-minute soak harness in `docs/SOAK.md` and reference run captured (2026-04-26): 2,000 req/s × 5 min, 100% success, p99 1.38 ms, RSS slope 0.35 MB/min — well under the 2 MB/min ceiling. Originally scoped at 24 h; reduced after review (bounded-by-construction tests cover the structural leak paths; 5 min × 300 samples catches the residual class). Originally targeted at 10,000 req/s; lowered to 2,000 because the single-host localhost harness exhausts ephemeral TCP ports past ~2 k/s — leak behaviour is rate-independent so the property is still validated, and the 10 k req/s anchor in SC-002 is verified by the criterion benchmarks (≈10 µs/request).
- [X] T055 [P] Verify SC-007 — write a `tests/integration/config_errors.rs` that intentionally feeds 20+ malformed TOML configurations (missing required keys, wrong types, unknown keys, invalid duration strings, invalid TLS paths, etc.) and asserts every error message names the file, the TOML key path or line, and a human-readable explanation
- [X] T056 [P] Verify SC-008 — write a `tests/integration/health_surface.rs` that exercises every operator-actionable failure mode (collector unreachable, queue overflow, TLS validation failure, OTLP rejection, correlation handle missing, capacity eviction, TTL eviction) and asserts the corresponding metric or counter from `data-model.md` §2 `HealthSnapshot` increments
- [X] T057 [P] Document TLS configuration in `README.md` per FR-013 (CA bundle, mTLS, server-name override, `insecure_skip_verify` defaults to `false` and requires explicit opt-out), with worked examples
- [X] T058 [P] Set up the pre-built binary build matrix in CI — `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`, glibc 2.36, mirroring the sibling fingerprinting plugin's release pattern; produce artifacts named `libotel_plugin-<arch>-glibc2.36.so`
- [X] T059 Run `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --check` and fix any remaining issues (Constitution Principle I)
- [X] T060 Run `quickstart.md` end-to-end against a real OpenTelemetry Collector on a clean machine, time the operator workflow, and assert it completes within 30 minutes per SC-001; capture any friction points and fold them back into `quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies. Can start immediately.
- **Foundational (Phase 2)**: Depends on Setup. Blocks all user stories.
- **User Story 1 (Phase 3, P1)**: Depends on Foundational. The MVP — implement first.
- **User Story 2 (Phase 4, P2)**: Depends on Foundational. Independent of US1.
- **User Story 3 (Phase 5, P3)**: Depends on Foundational. Independent of US1 and US2 (log-to-span linking is a soft optional in tests, gracefully degrades when no live span exists).
- **User Story 4 (Phase 6, P4)**: Depends on Foundational; effectively requires US1, US2, US3 to be complete to assert addon-parity meaningfully.
- **Polish (Phase 7)**: Depends on all user stories.

### Within each user story

- Tests are written and verified to FAIL before implementation tasks begin.
- For US1: pipelines (T025) before request handler (T027) before response handler (T028); reload (T030) after both handlers; edge cases (T031, T032) last.
- For US2: pipeline (T036) before instrument construction (T037) before value emission (T038).
- For US3: pipeline (T043) before rule emission (T044) before span association (T045).
- For US4: schema enforcement (T051) before exhaustive parser coverage (T050) so unsupported directives fail fast during US4 test development.

### Parallel opportunities

- Setup phase: T002, T003, T004 are all independent → run in parallel after T001.
- Foundational phase: T006, T010, T012, T015 are independent files → run in parallel; T007–T009 are sequential within `src/config/`; T011, T013, T014, T016 each depend on parts of T007–T009 + T010 → start when their inputs land.
- All test tasks within a user story are `[P]` — write in parallel, then implement.
- US1, US2, US3 implementation phases can run in parallel by different developers once Foundational is done.

---

## Parallel example: User Story 1 tests

Once Phase 2 is complete, write all US1 tests in parallel:

```text
Task T017: Contract test for SPOE message contract → tests/contract/spoe_messages.rs
Task T018: US1 acceptance scenario 1 (root span, attributes) → tests/integration/us1_tracing.rs
Task T019: US1 acceptance scenario 2 (parent-child traceparent round-trip) → tests/integration/us1_tracing.rs
Task T020: US1 acceptance scenario 3 (probabilistic + parent-based sampling) → tests/integration/us1_tracing.rs
Task T021: US1 acceptance scenario 4 (collector outage, fail-open) → tests/integration/us1_tracing.rs
Task T022: US1 acceptance scenario 5 (reload preserves in-flight) → tests/integration/us1_tracing.rs
```

Run `cargo test` and confirm all six fail for the right reason before starting T023+.

---

## Implementation strategy

### MVP first (User Story 1 only)

1. Phase 1: Setup
2. Phase 2: Foundational (CRITICAL — blocks everything)
3. Phase 3: User Story 1 — distributed tracing
4. **STOP and VALIDATE** — run the full US1 acceptance scenario suite plus the quickstart end-to-end against a real collector
5. Tag a `0.1.0-alpha` release if external feedback is desired

### Incremental delivery

After MVP:

1. Add Phase 4 (US2 metrics) — ship `0.2.0`
2. Add Phase 5 (US3 logs) — ship `0.3.0`
3. Add Phase 6 (US4 addon parity) — ship `1.0.0-rc.1`
4. Phase 7 polish until success-criteria gates pass — ship `1.0.0`

Each user-story milestone is independently demonstrable and addresses an
observable operator need.

### Parallel team strategy

With multiple developers after Foundational is done:

- Developer A: US1 (the MVP path)
- Developer B: US2 (metrics, no dependency on US1 implementation surface)
- Developer C: US3 (logs)
- Developer D: US4 schema-coverage work (T050, T051) plus golden config curation (T049), starting on a stub of US1 so the parity tests can be exercised incrementally

---

## FR ↔ task traceability

For Constitution Principle II ("every functional requirement maps to at least one automated test"):

| FR | Test task(s) | Implementation task(s) |
| --- | --- | --- |
| FR-001 | T018 | T025, T027, T029 |
| FR-002 | T019 | T012, T027 |
| FR-003 | T019 | T012, T027 |
| FR-004 | T020 | T023, T025 |
| FR-005 | T018, T019 | T029 |
| FR-006 | T018 (events), T046 (links) | T029 |
| FR-007 | T017, T018, T019, T032 | T010, T011, T027, T028 |
| FR-008 | T033, T034, T035 | T037 |
| FR-009 | T033, T034, T035 | T037, T038, T040 |
| FR-010 | T033 | T036 |
| FR-011 | T041, T042 | T044, T045 |
| FR-012 | T046 (covers all three protocols via fake collector) | T024, T025, T036, T043 |
| FR-013 | T055 (TLS error paths), T046 (TLS happy path) | T024, T057 |
| FR-014 | T021 | research R3 commitment realised in T025, T036, T043 |
| FR-015 | T021 (drop-oldest path), T056 (drop counter visibility) | T024 (queue config) |
| FR-016 | T046 | T050 |
| FR-017 | T047 | T008, T051 |
| FR-018 | T055 | T009 |
| FR-019 | T022 (reload drains old config); soak test in T054 confirms shutdown drain | T013 |
| FR-020 | T056 | T014 |
| FR-021 | T022 | T030 |
| FR-022 | T006 unit tests + every test that uses `$arg` sources | T006 |

---

## Notes

- `[P]` = different files, no dependency on incomplete tasks → safe to parallelize.
- `[Story]` = traceability to user-story acceptance criteria.
- Verify every test fails for the right reason before implementing the task it gates (Constitution Principle II).
- Commit after each task or logical group; commits scoped per `Development Workflow & Quality Gates` in the constitution.
- Stop at any phase checkpoint to validate the story independently.
- Avoid: vague tasks, same-file conflicts on `[P]`-marked tasks, cross-story implementation dependencies that would break independent testability.
