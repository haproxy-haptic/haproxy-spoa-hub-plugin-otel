# Feature Specification: OpenTelemetry SPOA Hub Plugin

**Feature Branch**: `001-otel-plugin`
**Created**: 2026-04-26
**Status**: Draft
**Input**: User description: "Look at all repos in ../haproxy-spoa-hub* and learn how it and its plugins are built. Then continue to look at https://github.com/haproxy/haproxy/issues/1640#issuecomment-4214779853 and github.com/haproxytech/opentelemetry-c-wrapper and how opentelemetry is implented in haproxy (see the source in /home/phil/Quellcode/GitHub/haproxy). It can take some time until the built-in OTEL support is really GA so I want you to build a SPOA hub plugin with the same features and API."

## Clarifications

### Session 2026-04-26

- Q: How does the plugin correlate request-side and response-side SPOE messages for a single transaction? → A: Plugin emits an opaque transaction-scoped correlation variable (HAProxy `txn` scope) on the request message; the operator wires HAProxy to pass it back as a SPOE argument on the response message. Transaction scope (not session) is required so HTTP/2 streams multiplexed on a single TCP session cannot share or overwrite each other's correlation slot. The plugin keeps span state in an in-process map keyed by that handle (hub-instance-local).
- Q: What format does the plugin accept its OpenTelemetry configuration in? → A: TOML only. The operator expresses every built-in-addon directive as a key path inside the plugin's `[plugins.params]` block, using a documented one-to-one mapping between addon directive names and TOML key paths. The plugin does not parse the addon's `.cfg` or `.yml` files. Migration to or from the built-in addon is a mechanical translation between TOML and addon syntax.
- Q: What sustained throughput per hub instance anchors the plugin's documented performance budget? → A: 10,000 requests per second per hub instance. This is the operating point at which the constitution's 1 ms p95 / 5 ms p99 added-latency ceiling and the bounded-memory soak-test guarantee MUST hold; benchmarks and queue-sizing defaults are calibrated to this number.
- Q: Does the plugin support multiple pipelines per signal type in v1, or only one? → A: Multi-pipeline per signal from day one. Operators MAY configure any number of tracer, meter, and logger pipelines, each targeting a different OTLP endpoint with its own sampler, exporter protocol, TLS settings, headers, retry policy, and queue. Health surface, drop counters, and queue-depth metrics are reported per pipeline.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Distributed tracing for HTTP traffic (Priority: P1)

An HAProxy operator runs a fleet of HAProxy instances that load-balance HTTP services and already operates an OpenTelemetry collector. They want every HTTP request that traverses HAProxy to be represented as a span in their tracing backend, with correct parent/child relationships when an upstream caller has already started a trace, so they can follow a request end-to-end across services without waiting for HAProxy's built-in OTEL support to reach general availability.

**Why this priority**: Distributed tracing is the highest-value, most-requested observability capability and the primary motivation for the user to want this plugin at all. Without it, the plugin has no reason to exist. Tracing alone is a usable product even before metrics or logs are added.

**Independent Test**: Configure a single HAProxy frontend with the plugin attached as a SPOE filter, send a synthetic request that carries a `traceparent` HTTP header, and verify in the collector that (a) the resulting span has the same trace ID as the incoming `traceparent`, (b) the span's parent ID matches the incoming span ID, (c) the span carries operator-configured attributes derived from request data (method, host, path, status), and (d) downstream backends receive an updated `traceparent` header pointing to the HAProxy span.

**Acceptance Scenarios**:

1. **Given** a request with no incoming trace context, **When** it passes through HAProxy with the plugin enabled, **Then** the operator sees a single root span in their OTLP backend whose attributes include the request's method, host, path, response status, and HAProxy server name.
2. **Given** a request with a valid W3C `traceparent` header, **When** it passes through HAProxy, **Then** the emitted span shares the trace ID and references the inbound span as its parent, and the request forwarded to the backend carries an updated `traceparent` whose span ID equals the HAProxy span's ID.
3. **Given** the operator configures probabilistic sampling at 10%, **When** 10,000 requests pass through, **Then** the count of spans received by the collector is within statistical tolerance of 1,000, and parent-based sampling decisions are honoured for requests carrying an upstream sampling flag.
4. **Given** the OTLP collector becomes unreachable, **When** new requests arrive, **Then** HAProxy continues to serve traffic without added latency or errors, span data is buffered up to the configured limit, drop counts increase, and once the collector returns the plugin resumes export.
5. **Given** the operator updates the plugin configuration and signals the hub to reload, **When** the reload completes, **Then** in-flight spans for already-started requests still finish and export, and new requests use the new configuration without dropped connections.

---

### User Story 2 - HAProxy-flow metrics via OTLP (Priority: P2)

The same operator wants to see counters, latency histograms, gauges and up-down counters that describe HAProxy's request flow (request rate, response status distribution, in-flight requests, response-size histogram, etc.) in their OpenTelemetry-compatible metrics backend, so they can build dashboards and alerts using the same pipeline as their tracing data and stop maintaining a parallel Prometheus-only path.

**Why this priority**: Metrics are second to tracing in operator demand and deliver value independently of traces. Dashboards and alerts are the most common day-2 use of observability data. This story can be tested and shipped without User Story 3 or 4.

**Independent Test**: Configure the plugin with a counter, a histogram, and a gauge instrument bound to operator-chosen sample expressions; drive synthetic load through HAProxy; verify the OTLP metrics endpoint receives the expected aggregated data points, with the configured attribute dimensions, at the configured export interval.

**Acceptance Scenarios**:

1. **Given** the operator declares a counter `http.requests` keyed on `status_class` and `frontend`, **When** 500 successful and 50 failed requests pass through, **Then** the collector receives counter sums of 500 and 50 against the corresponding attribute combinations.
2. **Given** the operator declares a histogram `http.server.duration` with explicit bucket bounds, **When** requests with mixed latency are served, **Then** the histogram's bucket counts and sum reflect the served distribution within one export interval.
3. **Given** the operator declares an up-down counter for in-flight requests, **When** requests are served concurrently, **Then** the gauge value tracks the count of currently in-flight requests and returns to zero when traffic stops.

---

### User Story 3 - Structured log records via OTLP (Priority: P3)

The same operator wants to emit OTLP log records for selected events (e.g., authentication failures, rate-limit hits, sticky-session re-elections) with structured attributes and, when applicable, a trace context that links the log line to its span, so they can pivot from a trace to the logs of that exact request in their unified observability backend.

**Why this priority**: Logs are the third pillar but typically already covered by HAProxy's text logs in many deployments. OTLP logs add value for shops moving everything to a single pipeline, but most operators can adopt the plugin successfully without this story.

**Independent Test**: Configure a log-record rule on a selected event, drive a request that triggers the event, and verify the collector receives a log record with the configured severity, body, attributes, and (when the event happens during a sampled trace) a non-zero trace and span ID linking it to the live span.

**Acceptance Scenarios**:

1. **Given** a log rule with severity `WARN` bound to a `tcp-request` rejection, **When** a connection is rejected, **Then** the collector receives one log record at severity `WARN` with the configured body and attributes.
2. **Given** the same event occurs during a sampled HTTP request, **When** the log record is emitted, **Then** its trace and span IDs match those of the active HAProxy span for the request.

---

### User Story 4 - Mechanical configuration parity with the built-in OTEL addon (Priority: P4)

The same operator wants the plugin's configuration vocabulary to mirror, one-to-one, the directive names and nesting structure of the built-in HAProxy OTEL addon — expressed as TOML key paths inside the plugin's `[plugins.params]` block — so that when the built-in addon reaches general availability they can mechanically translate their TOML configuration into the addon's `.cfg`/`.yml` files (and vice versa for rollback) by applying a documented mapping, without redesigning their telemetry semantics.

**Why this priority**: Migration friction is the operator's biggest hidden cost. This story has lower immediate value than 1–3 but is highly valuable when the built-in addon ships. It can be delivered incrementally on top of stories 1–3.

**Independent Test**: Take a complete configuration that the built-in HAProxy OTEL addon would accept (instrumentation file, scope file, optional pipeline file). Mechanically translate it to the plugin's TOML key paths using the published one-to-one mapping. Hand the resulting TOML to the plugin. Verify the operator can produce the same spans, metrics, and log records as the built-in addon would for the same HAProxy configuration.

**Acceptance Scenarios**:

1. **Given** a TOML configuration whose key paths correspond one-to-one to a valid built-in-addon configuration via the published mapping, **When** the plugin loads it, **Then** the plugin starts cleanly, exposes the equivalent scopes and groups, and the resulting telemetry matches the addon's output for equivalent traffic.
2. **Given** a TOML configuration that uses a key path corresponding to an addon directive the plugin does not yet support, **When** the plugin loads it, **Then** the plugin refuses to start, names the unsupported directive in both its addon-vocabulary form and its TOML key path, and points the operator to the migration notes.
3. **Given** the operator switches from the plugin to the built-in addon, **When** they apply the published TOML-to-addon translation to their plugin configuration, remove the SPOE filter, and add the native filter pointing to the resulting files, **Then** the resulting telemetry is functionally equivalent for the supported feature set.

---

### Edge Cases

- An incoming `traceparent` header is malformed or contains an invalid trace ID. The plugin MUST treat this as no incoming context and start a new root trace; it MUST NOT reject the request and MUST NOT export the malformed value.
- A `tracestate` header is too long. The plugin MUST truncate per W3C rules rather than dropping the request or the trace context.
- The OTLP exporter cannot keep up with span/metric/log production. The plugin MUST drop the oldest data, increment a drop counter exposed to operators, log a warning at most once per configurable period, and never block HAProxy traffic.
- A single SPOE message arrives without its expected counterpart (e.g. an `on-http-response` without an `on-frontend-http-request`). The plugin MUST emit a span with whatever data is available, mark it with an explicit status indicating the missing data, and continue.
- The operator misconfigures a sample expression that fails to evaluate at run time. The plugin MUST treat the attribute as absent, increment a per-rule error counter, and continue.
- The configured export endpoint uses a TLS certificate the plugin cannot validate. The plugin MUST fail to start with a clear error naming the file and the validation problem rather than starting and silently dropping all telemetry.
- HAProxy fires a SPOE message on a request that arrives during plugin reload. The hub-level guarantee that in-flight messages drain MUST hold; the plugin MUST NOT lose state for an in-flight transaction across reload.
- The OTLP backend rejects a payload (oversized batch, invalid attributes). The plugin MUST split or drop the payload according to documented rules, log the failure, and continue exporting subsequent batches.

## Requirements *(mandatory)*

### Functional Requirements

#### Tracing (User Story 1)

- **FR-001**: The plugin MUST emit one span per HTTP request seen via SPOE, with a configurable name and a configurable set of attributes derived from operator-chosen HAProxy sample expressions.
- **FR-002**: The plugin MUST extract incoming W3C TraceContext (`traceparent`, `tracestate`) and Baggage (`baggage`) from request headers passed by HAProxy as SPOE arguments, and use them as the parent context for the emitted span.
- **FR-003**: The plugin MUST inject updated W3C TraceContext into a designated set of SPOE return variables so the operator can apply them to outgoing headers on the request to the backend, preserving the trace across hops.
- **FR-004**: The plugin MUST support sampler configurations equivalent to the built-in addon: always-on, always-off, trace-id ratio, and parent-based with a configurable fallback sampler.
- **FR-005**: The plugin MUST set span status (OK, ERROR, UNSET) based on operator-configured rules referencing sample expressions (typically response status code).
- **FR-006**: The plugin MUST support span events (timestamped key/value annotations) and span links (references to non-parent spans), each with attributes built from sample expressions.
- **FR-007**: The plugin MUST correlate request-side and response-side SPOE messages for a single transaction by emitting an opaque, transaction-scoped (HAProxy `txn` scope) correlation handle as a return variable on the request-side message and accepting that same value back as a SPOE argument on the response-side message. Transaction scope is required (not session): under HTTP/2 multiple streams share a TCP session, so a session-scoped slot would let stream A overwrite stream B's correlation handle and end the wrong span. The plugin's outbound TraceContext variables (`traceparent_out`, `tracestate_out`, `baggage_out`) MUST also be transaction-scoped for the same reason. Span state for an in-flight transaction is keyed by the handle inside the plugin's in-process map and is local to a single hub instance; the operator's HAProxy configuration MUST pass the handle back via standard `set-var` round-tripping.

#### Metrics (User Story 2)

- **FR-008**: The plugin MUST support metric instrument types equivalent to the built-in addon: counter, up-down counter, histogram (explicit bounds and exponential), and gauge.
- **FR-009**: The plugin MUST allow each instrument's value and dimensional attributes to be derived from operator-chosen sample expressions, and MUST allow the operator to update an instrument's recorded value at any configured trigger point.
- **FR-010**: The plugin MUST aggregate metric data points client-side and export them on a configurable interval; the default export interval MUST be safe for typical OTLP collectors and configurable per deployment.

#### Logs (User Story 3)

- **FR-011**: The plugin MUST emit OTLP log records with operator-configured severity (matching OpenTelemetry severity numbers and names), body, and attributes built from sample expressions, optionally associated with the active HAProxy span for the same transaction.

#### Export and transport

- **FR-012**: The plugin MUST export telemetry via OTLP/gRPC and OTLP/HTTP (with both Protobuf and JSON encoding for the HTTP variant), selectable per pipeline.
- **FR-013**: The plugin MUST support TLS for OTLP exporters, including operator-supplied CA certificates, mTLS client certificates, and toggling of server-name verification, with safe defaults that require explicit opt-out for each unsafe choice.
- **FR-014**: The plugin MUST fail open with respect to HAProxy traffic at all times: telemetry export errors, backpressure, or remote unavailability MUST never block, slow, or fail a request being served.
- **FR-015**: The plugin MUST bound the in-memory queue for each pipeline (per pipeline, per signal type), drop oldest data on overflow, and expose drop counters and queue depths per pipeline through the hub's metrics endpoint and through structured logs.

#### Configuration and migration parity

- **FR-016**: The plugin MUST accept its OpenTelemetry configuration as TOML inside the hub's standard `[plugins.params]` block. The TOML key paths MUST mirror, one-to-one, the directive names and nesting structure of the built-in HAProxy OTEL addon for instrumentation, scopes, groups, spans, attributes, events, baggage, instruments, log records, samplers, and exporter pipelines. The plugin MUST publish a documented mapping between addon directives and TOML key paths so translation in either direction is mechanical. The plugin MUST NOT parse the addon's native `.cfg` or `.yml` files.
- **FR-017**: The plugin MUST refuse to start if the operator's configuration uses a TOML key path that maps to a built-in-addon directive the plugin does not implement, naming the unsupported directive in both its addon-vocabulary form and its TOML key path together with the line at which it appeared, rather than silently ignoring it.
- **FR-018**: The plugin MUST validate the operator's TOML configuration at startup and at reload, surfacing errors with file, line, key path, and a human-readable explanation; an invalid reload MUST leave the previous configuration running, not a partial mix.

#### Lifecycle and operability

- **FR-019**: The plugin MUST drain pending telemetry on shutdown, up to a configurable timeout, before exiting; data not flushed within the timeout MUST be counted and logged.
- **FR-020**: The plugin MUST expose its own health surface (queue depth, drop count, export attempts and failures, last successful export timestamp — reported per pipeline) so operators can determine that the plugin is the cause of an incident without reading source code.
- **FR-021**: The plugin MUST tolerate hub reload (SIGHUP) without losing in-flight transaction state and without restarting OTLP connections that are still healthy unless their configuration changed.

#### Sample-expression coverage

- **FR-022**: The plugin MUST be able to use any HAProxy sample expression value that HAProxy can deliver as a SPOE argument as a source for span attributes, metric attributes, log attributes, instrument values, and span status conditions.

### Key Entities

- **Tracer pipeline**: A configured pipeline that owns a sampler, a set of span processors, a batch exporter, and an OTLP endpoint. Operators MAY configure any number of tracer pipelines, each targeting a different endpoint with its own sampler and exporter settings.
- **Meter pipeline**: A configured pipeline that owns metric readers, aggregation policies, and a batch exporter targeting an OTLP endpoint. Operators MAY configure any number of meter pipelines.
- **Logger pipeline**: A configured pipeline that owns a log record processor and a batch exporter targeting an OTLP endpoint. Operators MAY configure any number of logger pipelines.
- **Scope**: A named grouping of telemetry rules (spans, attributes, events, baggage, instruments, log records) that the operator binds to one or more SPOE message events. Mirrors the built-in addon's `otel-scope`.
- **Group**: A named collection of scopes triggered together by a single SPOE-driven action. Mirrors the built-in addon's `otel-group`.
- **Span attribute / metric attribute / log attribute**: A key plus a sample-expression source plus an optional condition.
- **Instrument**: A counter, up-down counter, histogram, or gauge with a name, unit, description, attribute set, value source, and aggregation policy.
- **Sampler configuration**: One of always-on, always-off, trace-id ratio, or parent-based with a fallback; applied per tracer pipeline.
- **Exporter endpoint**: A destination OTLP endpoint with protocol, encoding, TLS settings, headers, timeout, and retry policy.
- **Transaction correlation handle**: An opaque value emitted by the plugin as a transaction-scoped (HAProxy `txn`) return variable on the request-side SPOE message and supplied back to the plugin as a SPOE argument on the response-side message. Transaction scope is mandatory under HTTP/2 (one TCP session can carry many concurrent streams). Used by the plugin as the key in a hub-instance-local map of in-flight span state. The operator wires the round-trip in `haproxy.cfg` via standard variable round-tripping; no shared state across hub instances is required.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator who already uses HAProxy and an OTLP-compatible backend can install the plugin and produce their first sampled distributed trace in under 30 minutes, starting from a vanilla HAProxy configuration and following the quickstart in the plugin's documentation.
- **SC-002**: An operator running the plugin at 10,000 requests per second sustained per hub instance observes no more than 1 ms of added p95 per-request latency and no more than 5 ms of added p99 per-request latency attributable to the plugin, measured against the same HAProxy without the plugin.
- **SC-003**: When the OTLP collector is unreachable for up to ten minutes while the plugin is sustaining 10,000 requests per second per hub instance, no HAProxy request is rejected, slowed, or stalled by the plugin, and the plugin resumes exporting within one export interval after the collector returns.
- **SC-004**: For a configuration that is also accepted by the built-in HAProxy OTEL addon and uses only features the plugin supports, the spans, metrics, and log records produced by the plugin and by the built-in addon for the same traffic are functionally equivalent — same names, same attribute sets, same parent/child relationships, same counter/histogram values within rounding.
- **SC-005**: At least 95% of operators who attempt the migration described in User Story 4 produce a working built-in-addon configuration by applying the plugin's documented one-to-one translation between TOML key paths and addon directives, without redesigning their telemetry semantics, measured by a survey of operators who run the plugin in production for at least one month.
- **SC-006**: The plugin's resident memory footprint is bounded under sustained traffic, with steady-state RSS growth no greater than 2 MB per minute at 95% confidence over a 5-minute soak at the documented harness rate (2,000 req/s on a single-host localhost setup; per-deployment soaks at higher rates are an operator concern). Leak behaviour is rate-independent — it concerns whether per-request allocations escape onto a long-lived structure — so the soak rate verifies the same property regardless. The 10,000 req/s anchor in SC-002 / SC-003 is unchanged and is verified by the criterion benchmarks (per-request overhead ≤ 10 µs).
- **SC-007**: When the plugin is misconfigured, 100% of the configuration errors surfaced to the operator name the offending file, a line or path within the file, and a human-readable explanation; no configuration error is reported as an unattributed stack trace alone.
- **SC-008**: For every operator-actionable failure mode (collector unreachable, queue overflow, TLS validation failure, export rejection), the operator can determine from the plugin's exposed health surface alone that the plugin is the cause, without reading the plugin's source code.

## Assumptions

- The plugin runs inside `haproxy-spoa-hub` as a dynamically loaded plugin and is wired to HAProxy via the SPOE protocol, following the existing hub plugin contract. Operators configure HAProxy to send SPOE messages at the request lifecycle points the plugin needs (typically request start and response end) and propagate the plugin's returned variables back into HAProxy headers where required for context propagation.
- Because the plugin's transaction correlation map is hub-instance-local (see Clarifications, Q1), an HAProxy transaction's request-side and response-side SPOE messages MUST reach the same hub instance. With the typical 1:1 deployment (one hub per HAProxy on the same host via a Unix socket) this is trivially satisfied. Operators who deploy multiple hub instances behind a network load balancer MUST configure session affinity so that all SPOE messages for a given HAProxy stream are routed to the same hub instance.
- The plugin's primary objective is to provide an interim path to OpenTelemetry parity for operators who cannot yet adopt HAProxy's built-in OTEL addon (for reasons including HAProxy version, build flags, or its non-GA status at the time of writing). The plugin will continue to be useful even after the built-in addon reaches GA, for operators who do not wish to recompile HAProxy with extra build flags.
- The set of HAProxy lifecycle events visible to the plugin is the set HAProxy can fire SPOE messages on. Lifecycle events that the built-in addon hooks via filter callbacks but that HAProxy cannot expose via SPOE are out of scope for the first release; the plugin documents the unsupported events explicitly. This is the principal known feature gap with the built-in addon and will be revisited when SPOE gains the necessary events or when alternative correlation mechanisms emerge.
- "Same features and API" is interpreted as: same directive names organized in the same nesting structure (expressed in the plugin as TOML key paths inside `[plugins.params]`), same set of samplers, propagators (W3C TraceContext and Baggage), and instrument types, and functionally equivalent telemetry output. It is not interpreted as a binary-compatible drop-in for the built-in addon — the plugin is loaded as a SPOA hub plugin, not as an HAProxy filter, and accepts only TOML configuration; the addon's native `.cfg`/`.yml` files are not parsed by the plugin.
- Default sampler is parent-based with `trace_id_ratio_based(1.0)` as the fallback (i.e., honour upstream sampling decisions, otherwise sample everything). Default propagators are W3C TraceContext and Baggage. Default protocol is OTLP/gRPC. These defaults match the OpenTelemetry specification's recommendations and the addon's behaviour where it specifies one.
- Performance budgets and resource caps inherit from the project constitution (Principle IV and the Performance Standards & Reliability section): bounded queues, async export, fail-open w.r.t. traffic, declared per-feature budgets, no unbounded defaults. The numeric defaults in SC-002 and SC-003 are the constitution-level ceilings; the plan is expected to declare and meet them.
- The plugin is initially targeted at HAProxy 2.8 or newer, the same baseline as other haproxy-spoa-hub plugins. Older HAProxy versions are out of scope unless and until an operator presents a concrete need.
