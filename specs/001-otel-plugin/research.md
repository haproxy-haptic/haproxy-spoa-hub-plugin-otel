# Phase 0 Research: OpenTelemetry SPOA Hub Plugin

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-04-26

This document records the technology and design decisions taken before
Phase 1. Every Technical Context unknown is resolved here. Each entry is
recorded as **Decision / Rationale / Alternatives considered**.

---

## R1. OpenTelemetry SDK distribution

**Decision**: Use the upstream Rust crates `opentelemetry`, `opentelemetry-sdk`,
`opentelemetry-otlp`, and `opentelemetry-semantic-conventions` from
`open-telemetry/opentelemetry-rust`. Pin minor versions in `Cargo.toml`
(major.minor.*) and bump deliberately.

**Rationale**: This is the canonical Rust implementation of the OpenTelemetry
specification, maintained by the OpenTelemetry SIG. The API shape (`SdkTracerProvider`,
`SdkMeterProvider`, `SdkLoggerProvider` with `with_batch_exporter` /
`with_periodic_exporter`) is exactly what the plugin needs and supports
multiple independent provider instances, which directly delivers the multi-pipeline
requirement (spec clarification Q4).

**Alternatives considered**:

- `tracing` + `tracing-opentelemetry` bridge — unsuitable: the plugin must
  emit OTel telemetry directly, not bridge from `tracing` spans, and it
  doesn't expose enough control over span lifecycle (the addon-mirror
  semantics need explicit `span_builder().start_with_context()` /
  `end_with_timestamp()` calls).
- Hand-rolled OTLP encoder over `prost` + `tonic`/`reqwest` — rejected on
  the Code Quality principle (duplication of a maintained crate) and on
  testing burden.

---

## R2. OTLP exporter feature-flag selection

**Decision**: Enable `grpc-tonic`, `http-proto`, `http-json`, `tls`,
`gzip-tonic` in `opentelemetry-otlp`. Expose all three protocol variants
to operators per pipeline:

- `grpc` (OTLP/gRPC, Protobuf over HTTP/2 via tonic)
- `http/protobuf` (OTLP/HTTP, Protobuf body)
- `http/json` (OTLP/HTTP, JSON body)

The default protocol is `grpc` (matches OpenTelemetry specification's
default and the addon's de-facto default).

**Rationale**: FR-012 requires all three. `opentelemetry-otlp` ships them
behind feature flags; enabling all three gives the operator a per-pipeline
choice without introducing additional dependencies.

**Alternatives considered**:

- gRPC-only — rejected, fails FR-012.
- HTTP-only — rejected, gRPC is the OpenTelemetry default and many
  collectors expose only the gRPC port in production.

---

## R3. Bridging the synchronous SPOE handler to the asynchronous OTLP exporters

**Decision**: The plugin's `process()` runs on the hub's blocking thread
pool (per the SDK contract). It performs all telemetry production
(span builds, metric records, log emissions) synchronously without
blocking I/O, and hands finished telemetry items to the SDK's batch
processors via the SDK's bounded channels. The SDK's batch processors
run on the tokio runtime that the hub already provides; the OTLP exporter
performs the network I/O on that runtime.

The plugin owns one `tokio::sync::mpsc` per pipeline only when the SDK's
default channel sizing is unsuitable; otherwise it relies on the SDK's
own bounded internal queues, configured via
`SdkTracerProvider::builder().with_batch_exporter()` and the corresponding
batch-processor configuration knobs (max queue size, scheduled delay,
max export batch size).

**Rationale**: FR-014 (fail-open) requires that the hot path NEVER waits
on network I/O. The OpenTelemetry SDK's batch span/log processors and
periodic metric reader already implement bounded async export.
Wrapping them with our own channel adds complexity without changing the
ownership of the network I/O — they own it either way. Constitution
Principle IV ("Telemetry MUST NOT block traffic") is satisfied by the
SDK's batch model plus drop-oldest policy on overflow.

**Alternatives considered**:

- Plugin-owned mpsc channels with a custom exporter task — adds layer,
  loses the SDK's tested batching/retry semantics.
- Synchronous export inline — rejected; would violate FR-014 immediately.

---

## R4. Correlation map data structure

**Decision**: `dashmap::DashMap<CorrelationHandle, InFlightSpan>` plus a
companion `tokio::time` interval task that evicts entries whose
`opened_at` is older than a configurable TTL (default 60 s). Cap the
total entry count at a configurable `max_in_flight_transactions`
(default 100,000); on overflow, evict the oldest by `opened_at`.

**Rationale**: A `DashMap` provides lock-free reads and sharded writes,
which is the right shape for the SPOE hot path (one insert on the
request-side message, one remove on the response-side message, both
on the blocking thread pool with high concurrency). Bounding the map
both by TTL and by count protects against the response-side message
never arriving (FR-015 spirit applied to in-flight state, not just
queues). Constitution Principle IV: no unbounded defaults.

The `CorrelationHandle` is a 128-bit random value, hex-encoded as a
32-character string, returned to HAProxy as a session-scoped variable
on the request-side message.

**Alternatives considered**:

- `parking_lot::RwLock<HashMap<...>>` — single global lock under
  contention; rejected at 10k req/s.
- HAProxy stream/transaction-id-based correlation — would have removed
  the need for a handle entirely, but spec clarification Q1 explicitly
  chose Option A (plugin emits its own opaque handle), so this is now
  out of scope.

---

## R5. Test fake for the OTLP collector

**Decision**: Hand-rolled in-process collector fixture in
`tests/integration/common/otlp_collector.rs` that exposes both
OTLP/gRPC (via `tonic`) and OTLP/HTTP (via `axum` or `hyper`)
endpoints on `127.0.0.1:0` (ephemeral ports). It records every received
`ExportTraceServiceRequest`, `ExportMetricsServiceRequest`, and
`ExportLogsServiceRequest` and exposes them through assertion helpers
(`assert_received_span`, `assert_received_metric`, …). The fixture also
supports configurable injected failures — refuse N requests, drop the
connection, return a partial-success response.

**Rationale**: Constitution Principle II requires deterministic, hermetic
tests. A real collector binary would introduce process-spawn cost,
external dependency, and non-determinism. `wiremock` is a reasonable
HTTP-only option but doesn't speak gRPC; rolling a small `tonic` server
is straightforward (≤ 200 lines) and gives full control over failure
injection.

**Alternatives considered**:

- `wiremock` (HTTP-only) + a separate gRPC fake — extra surface, two
  fixtures to maintain.
- Booting a real OTel Collector in CI — rejected: external binary,
  flake risk, slow.

---

## R6. Sample-expression sourcing — addon evaluates HAProxy fetches at filter callback time; the plugin receives values as SPOE arguments

**Decision**: Where the addon writes
`attribute http.method method` (HAProxy `method` fetch evaluated by the
filter at callback time), the plugin's TOML writes
`attribute http.method = "$arg.method"` and the operator's HAProxy SPOE
message MUST include `method=method` (or whatever fetch they want) in
its `args`. The TOML attribute value uses a `$arg.<name>` reference
syntax to a SPOE argument the plugin received in the current message.
A `$var.<scope>.<name>` syntax (typed via the SPOE arg pipeline) is
reserved for future use; v1 supports only `$arg.<name>` and TOML
literals.

**Rationale**: A SPOA plugin cannot evaluate HAProxy sample fetches
itself — by design, fetches are passed in by HAProxy as SPOE message
arguments. The addon-vocabulary mapping must therefore translate
"sample expression at directive evaluation time" into "named SPOE
argument the operator wired up". This is documented explicitly in the
addon-directive-mapping contract.

**Alternatives considered**:

- Implement a HAProxy-fetch interpreter inside the plugin — requires
  re-implementing dozens of fetches and constantly chasing HAProxy.
  Rejected.
- Allow arbitrary TOML expressions evaluated at message time — over-
  scoped for v1; will be reconsidered if operators ask for it.

---

## R7. Reload semantics for in-flight transactions

**Decision**: On hub reload (SIGHUP), the plugin's previous configuration
remains the owner of in-flight `InFlightSpan` entries until they finish
or expire by TTL. New transactions started after the reload use the new
configuration. Each `InFlightSpan` carries an `Arc` reference to the
SDK provider set under which it was opened, so the providers stay alive
until the last span that references them finishes; only then are the
old providers dropped (and shutdown drained).

If the new configuration removes a pipeline that an in-flight span was
exporting through, the span is exported via the old pipeline (its
references are still live). If the new configuration adds pipelines,
new transactions see them; in-flight ones do not.

**Rationale**: FR-021 requires reload tolerance without losing
in-flight transaction state. Constitution Principle IV requires no
loss of telemetry under normal operation. Holding `Arc<Provider>` in
each `InFlightSpan` is the canonical Rust pattern for this guarantee.

**Alternatives considered**:

- Drain all in-flight spans on reload — rejected: would violate FR-021
  on a busy hub (unbounded drain time at high concurrency).
- Migrate in-flight spans to new providers — rejected: SDK does not
  support span migration; would mean re-wrapping context and dropping
  data.

---

## R8. TLS implementation choice for OTLP exporters

**Decision**: Use `rustls` via the `tls-roots` feature on
`opentelemetry-otlp` (loads platform CA roots through `rustls-native-certs`
or similar) and the `reqwest-rustls-tls` / `tonic`-rustls path for the
HTTP and gRPC transports respectively. Operator may optionally provide
a custom CA bundle, a client certificate (mTLS), and a server-name
override via TOML keys mirroring the addon's TLS knobs.

**Rationale**: rustls is the Rust ecosystem default, requires no system
OpenSSL build-time dependency (matching the sibling plugins' clean
build profile), and integrates with both `tonic` and `reqwest`. The
sibling fingerprinting plugin uses rustls; staying consistent reduces
operator surprise and shared toolchain footprint.

**Alternatives considered**:

- `native-tls` (OpenSSL via system) — adds a system dependency the
  hub does not currently have. Rejected.
- Mixing rustls for HTTP and native-tls for gRPC — operationally
  confusing and doubles the trust-store maintenance.

---

## R9. JSON Schema for TOML configuration

**Decision**: Author a JSON Schema (Draft 2020-12) covering every TOML
key path the plugin recognises, derived mechanically from the
addon-directive-mapping contract. The schema is returned from
`config_schema()` (per the hub plugin SDK contract); the hub validates
the operator's TOML against it before `init()`. The schema sets
`additionalProperties: false` at every object level to make
FR-017 (refuse to start on unsupported directive) trivially enforceable.

**Rationale**: The fingerprinting plugin already demonstrates this
pattern (config.rs:50-64 in that crate). Reusing it costs nothing
and gets us FR-017 plus FR-018 (file/line/key-path errors) for free
because `jsonschema`'s error reporting includes the offending path.

**Alternatives considered**:

- Hand-rolled validation in `init()` — rejected: every directive
  becomes a custom code path, more code to maintain, lossier error
  messages.
- No validation, parse-as-you-go — rejected: would violate FR-017
  and SC-007.

---

## R10. SPOE argument typing → OpenTelemetry attribute typing

**Decision**: `SpoeValue` (the hub SDK's typed enum for SPOE values)
maps to `opentelemetry::Value` as follows:

| `SpoeValue` variant | OTel attribute type |
| --- | --- |
| `Bool` | `Bool` |
| `Int32` / `Int64` / `UInt32` / `UInt64` | `I64` (range-checked; values exceeding `i64::MAX` are dropped with a per-rule error counter) |
| `Float` (if added) | `F64` |
| `String` | `String` |
| `Binary` | hex-encoded `String` |
| `Ipv4` / `Ipv6` | textual `String` |
| `Null` | attribute omitted (treated as "missing") |

The operator MAY specify a desired output type via a TOML
`type = "string" | "int" | "double" | "bool"` per attribute rule;
when present, the plugin coerces the SPOE value or emits a per-rule
error counter and omits the attribute.

**Rationale**: OTel attributes are typed and a wire-level mismatch in
the collector is harder to debug than at the source. Explicit coercion
with a per-rule error counter satisfies FR-022 (every SPOE-deliverable
value usable as a source) and the spec's edge case for evaluation
failures.

**Alternatives considered**:

- Always emit as string — loses type fidelity in the backend; rejected.
- Refuse the request on type mismatch — rejected, violates FR-014
  (fail-open).

---

## R11. Default sampler

**Decision**: `parent_based(trace_id_ratio_based(1.0))` — i.e., honour
upstream sampling decisions, otherwise sample every trace.

**Rationale**: Matches the OpenTelemetry specification's recommended
default and the spec's stated default (Assumptions). Operators running
high-volume deployments are expected to override this in their TOML;
the default is safe for development and onboarding (SC-001 — first
trace in 30 minutes).

**Alternatives considered**:

- `always_on` as the unconditional default — drops parent-based
  inheritance, surprising for operators familiar with the addon.
- `trace_id_ratio_based(0.1)` — would silently drop 90% of traces in
  a fresh setup, contradicting SC-001.

---

## R12. Default propagators

**Decision**: W3C TraceContext (`traceparent`, `tracestate`) and W3C
Baggage. No B3 / Jaeger / OT-Trace by default. Operators can extend
the propagator composite via TOML if needed (deferred — not in v1
unless an operator presents a concrete need).

**Rationale**: Spec Assumptions name W3C TraceContext + Baggage as the
defaults, matching the addon's defaults and the OpenTelemetry
specification. B3 / Jaeger are legacy and add surface without operator-
visible benefit at first ship.

**Alternatives considered**:

- Include B3 by default — rejected: the addon doesn't, parity costs
  nothing here.

---
