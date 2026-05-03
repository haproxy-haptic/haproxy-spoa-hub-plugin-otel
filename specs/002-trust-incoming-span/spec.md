# Feature Specification: Per-Message Override of `trust_incoming_span` and Sample Decision

**Feature Branch**: `002-trust-incoming-span`
**Created**: 2026-05-03
**Status**: Draft
**Input**: User description: "Per-message override of trust-incoming-span and sample decision via SPOE arguments"

## Background

The plugin's tracing behaviour today is fixed at hub configuration time and applies uniformly to every request that enters a given scope:

- Whether to start a fresh root span or use the inbound `traceparent` as the parent is decided per-scope by `Scope.spans[].root` (see `src/dispatch/request.rs:38-42`). The default is "use the inbound parent if present"; flipping it to fresh-root requires editing the TOML and reloading the hub.
- The sampling decision is taken by the sampler attached to the active tracer pipeline (`src/dispatch/request.rs:43`, `src/pipelines/...`). One sampler per pipeline; no per-request override.

Downstream consumers — primarily the HAProxy Template Ingress Controller (HAPTIC) — need to expose per-Ingress annotations that map onto these decisions. The two concrete annotations driving this work are:

- `nginx.ingress.kubernetes.io/opentelemetry-trust-incoming-span` — when `false`, the controller wants HAProxy to ignore upstream `traceparent` for that route and start a fresh trace, even though the operator's hub-wide configuration trusts inbound trace context everywhere else.
- (Forward-looking) per-route hard sampling decisions ("never sample this route", "always sample this route") that ingress-nginx exposes via configmap-level toggles and that HAPTIC users will eventually want to drive per-Ingress.

Without a per-message override, HAPTIC must either (a) maintain a separate hub instance with `root = true` and route Ingresses to it via `service-upstream` tricks, or (b) ship a hub patch that hard-codes the per-Ingress map lookup. Both push complexity and operational fragility into the chart layer to compensate for a missing plugin capability.

## Clarifications

### Session 2026-05-03

- Q: Where in the SPOE message contract do per-message overrides belong, alongside `_event` (reserved) and operator-defined arguments? → A: A small set of underscore-prefixed reserved arguments (`_root`, `_sample`). Underscore-prefix already signals "reserved by the plugin" via `_event`; consumers can rely on the rule that a name beginning with `_` is plugin-controlled.
- Q: What happens when both a per-scope default (`Scope.spans[].root = true`) AND a per-message override (`_root = false`) are present? → A: The per-message argument wins. The operator's per-scope default is the policy; the SPOE argument is an exception requested by the route. This matches the "ingress annotation overrides controller default" expectation in HAPTIC.
- Q: What value space does `_sample` accept and how does it interact with the configured sampler? → A: Three string values, all short-circuiting the sampler. `record` always emits the span and exports it; `drop` never emits; `default` is the same as the argument being absent (use the configured sampler). Per-route sampling RATES (e.g., 10%) are NOT in scope for this feature and require a separate design — they need plumbing into the sampler's decision interface, not a per-message short-circuit.
- Q: How is a malformed override value handled (e.g., `_root = "yes"` instead of `"true"`)? → A: Log at WARN, ignore the override, fall back to the per-scope default for this transaction. The plugin's contract is fail-open with respect to HAProxy traffic; rejecting a request because of an operator typo in a SPOE argument would violate that. The malformed-override rate is exposed as a health counter so operators notice.
- Q: Are these overrides also accepted on response-side messages or only request-side? → A: Request-side only. Both decisions (start-as-root, sample/drop) are made when the span is opened; the response-side message just closes whatever was opened. Sending `_root` or `_sample` on a response-side message is a no-op (silently ignored, not an error, to keep the HAProxy config simple — operators may symmetrically wire both sides without harm).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Per-route fresh-root trace (Priority: P1)

A platform operator runs HAPTIC. The platform team trusts upstream services to send valid `traceparent` headers and configures the otel hub with `root = false` (the default — HAPTIC propagates the inbound trace). One application team has a noisy upstream that emits malformed or low-quality traces, and they want HAProxy to drop that history at the ingress for their route specifically: every request to `service-foo.example.com` should start a new root trace, regardless of whether `traceparent` arrived. They put `nginx.ingress.kubernetes.io/opentelemetry-trust-incoming-span: "false"` on their Ingress. HAPTIC translates that annotation into a per-host+path SPOE argument that the plugin honours.

**Why this priority**: This is the immediate gap blocking HAPTIC from closing one of the gap-report annotations. Without it, the only workaround is operator hub-fork or topology gymnastics.

**Independent Test**: Configure a single tracer pipeline with `Scope.spans[0].root = false` (default-trust). Send two synthetic requests through the plugin's request-side dispatch with the same upstream `traceparent` header — once without `_root`, once with `_root str(true)`. The first emits a child span sharing the upstream trace ID; the second emits a fresh root span with a new trace ID and no parent reference. Both export through the same pipeline.

**Acceptance Scenarios**:

1. **Given** a request with a valid `traceparent` and no `_root` SPOE argument, **When** it passes through the plugin, **Then** the emitted span shares the inbound trace ID and references the inbound span as its parent (current behaviour; regression check).
2. **Given** the same request with `_root = "true"`, **When** it passes through the plugin, **Then** the emitted span has a freshly generated trace ID, no parent reference, and `traceparent_out` carries the new trace ID (not the inbound one).
3. **Given** a request with `_root = "false"` and an inbound `traceparent`, **When** it passes through, **Then** the emitted span behaves as in scenario 1 (parent extracted) — `_root = "false"` is the explicit form of the default and is a no-op when the scope's `root` is already false.
4. **Given** a scope configured with `root = true` (operator chose fresh-root globally) and a request with `_root = "false"`, **When** it passes through, **Then** the emitted span uses the inbound `traceparent` as parent — per-message override wins over per-scope default.
5. **Given** a request with `_root = "yes"` (malformed), **When** it passes through, **Then** the override is ignored, the per-scope default applies, the request is served as if `_root` were absent, the malformed-override health counter increments by one, and a WARN log line records the malformed value (truncated for safety) and the scope name.

---

### User Story 2 - Per-route hard sampling decision (Priority: P2)

The same operator wants two simple boolean overrides for sampling: "always sample this route" (e.g., for a low-volume admin endpoint where every request should be traced for forensics) and "never sample this route" (e.g., for a noisy health-check endpoint that would dominate the trace budget). Both are achievable via a per-message argument that short-circuits the configured sampler.

**Why this priority**: Sampling RATES per route are out of scope for this feature; the boolean force-record / force-drop is a strict subset and is the part most directly translatable to existing ingress-controller annotations. P2 because the immediate HAPTIC need (Story 1) ships independently.

**Independent Test**: Configure a tracer pipeline with `parent_based(trace_id_ratio(0.0))` (drop everything by default unless inbound is sampled). Send three requests through the plugin: one without `_sample`, one with `_sample = "record"`, one with `_sample = "drop"`. Verify the collector receives exactly one span (the `record` one); the others are dropped.

**Acceptance Scenarios**:

1. **Given** a tracer pipeline with `always_off` sampling and no `_sample` argument, **When** a request passes through, **Then** no span is exported.
2. **Given** the same pipeline and `_sample = "record"`, **When** a request passes through, **Then** the span is created and exported regardless of the sampler.
3. **Given** an `always_on` sampling pipeline and `_sample = "drop"`, **When** a request passes through, **Then** no span data reaches the OTLP collector, but a non-recording span is constructed: the correlation handle is allocated, `traceparent_out` is emitted with the unsampled flag (`flags = 00`), and the response-side handler closes the in-flight entry without inflating the `correlation_missing_total` counter.
4. **Given** `_sample = "default"` (explicit), **When** a request passes through, **Then** behaviour is identical to the argument being absent — sampler decides.
5. **Given** `_sample = "maybe"` (malformed), **When** a request passes through, **Then** the override is ignored, the sampler's decision applies, the malformed-override counter increments, and a WARN log line records the malformed value.

---

### Edge Cases

- **Overrides on response-side messages.** `_root` and `_sample` on a response-side message are silently ignored (not an error). This permits operators to wire the same SPOE-argument set on both messages symmetrically without conditional HAProxy config.
- **`_sample = "drop"` interaction with metrics and logs.** Dropping a span MUST NOT silence the scope's metric and log emission. Instruments and log records are independent telemetry types and continue to fire (matches current `request.rs:106-107` semantics).
- **`_root = "true"` interaction with `_sample = "drop"`.** Both honoured: a fresh, non-recording root span is constructed. `traceparent_out` carries the new trace ID with the unsampled flag, so downstream services see a fresh, non-sampled trace context. Justification: the request still has a trace identity for downstream propagation; the local sampling decision is independent of trace continuity.
- **`_root = "true"` with no inbound `traceparent`.** Idempotent — both with and without override, the result is a fresh root. No-op override is silently honoured.
- **Concurrent overrides under HTTP/2.** Each SPOE message corresponds to one HAProxy transaction; arguments are message-scoped, not session-scoped. No cross-stream interference.
- **Malformed value high cardinality.** The malformed-override counter MUST NOT be labelled by the malformed value itself (that's an attacker-controlled high-cardinality vector). It MAY be labelled by argument name (`_root` vs `_sample`) and scope name only.
- **Reload during in-flight transaction.** A scope's `root` setting may change at reload while an in-flight transaction's request-side has already been processed. The override semantics are evaluated at request-side handling time only; no post-hoc reapplication on reload.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The plugin MUST accept a request-side SPOE argument named `_root` of string type with the values `"true"` or `"false"`. When present and parseable, it overrides the active scope's `Scope.spans[].root` setting for this transaction only.
- **FR-002**: The plugin MUST accept a request-side SPOE argument named `_sample` of string type with the values `"default"`, `"record"`, or `"drop"`. When `"record"`, the plugin MUST emit and export the span regardless of the configured sampler. When `"drop"`, the plugin MUST construct a non-recording span via the SDK's `SamplingDecision::Drop` path so the span has a valid `SpanContext` (`trace_id`, `span_id`) for downstream propagation but is not exported; the correlation handle is allocated as for any other span so the response-side handler closes it normally. When `"default"` or absent, the configured sampler decides.
- **FR-003**: The plugin MUST treat `_root` and `_sample` as request-side-only. Their presence on a response-side or group-trigger message MUST be silently ignored.
- **FR-004**: The plugin MUST treat malformed `_root` or `_sample` values (anything outside the documented value space, or the wrong SPOE type) as if the argument were absent. The plugin MUST emit a single WARN log line per malformed override, naming the argument and scope, and MUST increment a `malformed_override_total` health counter labelled by argument name (`_root` or `_sample`) and scope name.
- **FR-005**: When `_sample = "drop"` is honoured, the plugin MUST still emit instruments and log records configured on the active scope. Sampling and metric/log emission are independent.
- **FR-006**: The plugin MUST emit `traceparent_out`, `tracestate_out`, and `baggage_out` whenever a span is constructed (sampled or not), carrying the local span's `SpanContext`. When `_sample = "drop"` is honoured, `traceparent_out` carries the W3C-correct unsampled flag (`flags = 00`) so downstream services receive a coherent trace identity but understand it is not sampled at this hop. When `_root = "true"` is honoured, the trace identity in these variables reflects the freshly generated root, not the inbound `traceparent`.
- **FR-007**: The plugin MUST NOT change behaviour for any request that does not carry `_root` or `_sample`. The two added arguments are strictly additive; existing operator configurations and existing acceptance tests in `tests/us1_tracing.rs` MUST continue to pass without modification.
- **FR-008**: The plugin's documented SPOE message contract (this feature's `contracts/spoe-message-contract.md` and the consolidated reference under `specs/001-otel-plugin/contracts/spoe-message-contract.md` once stabilised) MUST list `_root` and `_sample` under the request-side message's reserved arguments table, alongside `_event`.

### Non-Functional Requirements

- **NFR-001**: Per-message override evaluation MUST NOT add measurable latency at the SC-002 anchor (10,000 req/s, ≤ 1 ms p95 / ≤ 5 ms p99). Operationally this means: argument lookup happens once per request via the existing `SpoeMessage::get_string` path; no allocation is required when the argument is absent (the common case).
- **NFR-002**: The malformed-override counter MUST be exposed on the hub's existing health surface alongside the plugin's current `correlation_missing_total`, queue-depth, and drop counters.

### Key Entities

- **Override resolution state** — a tiny per-request value computed once at request-side handling: `(effective_root: bool, sample_decision: SampleDecision)`. Computed by combining the active scope's defaults with parsed `_root` / `_sample` arguments. Lives only on the request-side stack frame; not stored in the correlation map.
- **`SampleDecision`** — an internal enum: `UseSampler | ForceRecord | ForceDrop`. Mapped from `_sample` argument values.

## Success Criteria *(mandatory)*

- **SC-001**: HAPTIC's `nginx.ingress.kubernetes.io/opentelemetry-trust-incoming-span` annotation can be implemented end-to-end (annotation on Ingress → map file → SPOE argument → plugin override → fresh root span emitted) using only operator-side templating; no plugin patch required.
- **SC-002**: A request without `_root` or `_sample` arguments takes the same code path as today, with no additional allocation, branch mispredict beyond a single hot-cached argument lookup, or measurable latency change vs the v0.1.1 baseline at 10,000 req/s. Verified by `criterion` benchmark comparing baseline vs feature builds; delta MUST be within ±2% of the baseline mean.
- **SC-003**: All five acceptance scenarios in User Story 1 and all five in User Story 2 are covered by integration tests under `tests/us1_tracing.rs` and a new `tests/us2_per_message_override.rs` (or equivalent), running against an in-process OTLP collector following the existing test pattern. Tests are deterministic and complete in under 5 seconds per file.
- **SC-004**: The malformed-override counter is observable on the hub's metrics endpoint and increments under the malformed-value scenarios in User Stories 1.5 and 2.5.

## Out of Scope

- Per-route sampling RATES (e.g., "sample this route at 10%, that route at 50%"). This requires a richer per-message sampler interface that selects a sampling rate at decision time; the boolean record/drop short-circuit in this spec is a strict subset and ships independently.
- Per-route metric or log filtering. Instruments and log records continue to fire regardless of `_sample`; that decision is part of `_sample`'s semantics, not an additional knob.
- Operator-side TOML schema additions. This feature is a SPOE-message-contract change only. The existing `Scope.spans[].root` and pipeline sampler configuration are unchanged.
- Backwards-compatible behaviour for misnamed arguments (e.g., `root` without underscore prefix). The reserved-prefix discipline applies; only `_root` and `_sample` are recognised.
- HAPTIC integration. HAPTIC's chart-side annotation library and SPOE arg wiring is a separate change in `charts/haptic/libraries/spoa-hub.yaml` and the relevant annotation library; tracked in HAPTIC's repository, not here.
