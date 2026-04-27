# Data Model: OpenTelemetry SPOA Hub Plugin

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-04-26

The plugin has no persistent storage. The "data model" is therefore the
in-memory configuration tree (constructed once at `init()` and again at
every reload) plus the in-memory transactional state (one entry per
in-flight HAProxy transaction, evicted on response or by TTL).

All types below are described conceptually. The `src/config/`,
`src/correlation/`, and `src/dispatch/` modules implement them.

---

## 1. Configuration tree (parsed from TOML at `init()`)

### `PluginConfig`

Top-level configuration. One instance per loaded plugin invocation.

| Field | Type | Notes |
| --- | --- | --- |
| `instrumentation` | `Instrumentation` | The single `otel-instrumentation` block, mandatory. |
| `tracer_pipelines` | `Vec<TracerPipeline>` | One or more tracer pipelines (multi-pipeline per spec Q4). At least one if any scope emits spans. |
| `meter_pipelines` | `Vec<MeterPipeline>` | One or more meter pipelines. Optional. |
| `logger_pipelines` | `Vec<LoggerPipeline>` | One or more logger pipelines. Optional. |
| `scopes` | `Vec<Scope>` | All `otel-scope` blocks. |
| `groups` | `Vec<Group>` | All `otel-group` blocks. |
| `runtime` | `RuntimeKnobs` | Hub-local non-addon settings (queue caps, drain timeout, log level). |

Validation rules:

- At least one `Scope` MUST be defined for the plugin to do anything observable.
- Every `Scope` MUST reference a tracer/meter/logger pipeline that exists in the corresponding `*_pipelines` list, or the implicit default if the operator did not name a pipeline.
- Every `Group` MUST reference scopes that exist.

### `Instrumentation`

Mirrors the addon's `otel-instrumentation` block.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Operator-chosen identifier. |
| `acl` | `Vec<AclRule>` | Optional pre-evaluation filters. |
| `log` | `Option<LogPolicy>` | Per-instrumentation log policy. |
| `groups` | `Vec<String>` | Names of groups this instrumentation activates. |
| `scopes` | `Vec<String>` | Names of scopes this instrumentation activates. |
| `rate_limit` | `Option<RateLimit>` | Telemetry production rate cap (orthogonal to sampling). |
| `debug_level` | `u8` | 0–4. |
| `option` | `Vec<InstrumentationOption>` | Set of: `Disabled`, `DontlogNormal`, `HardErrors`. |

### `Scope`

Mirrors `otel-scope`. Bound to one or more SPOE message events by the
operator's HAProxy configuration. The plugin matches by event-name
attribute on incoming SPOE messages.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Required. |
| `acl` | `Vec<AclRule>` | Optional. |
| `event` | `String` | The SPOE message name this scope reacts to. |
| `idle_timeout` | `Option<Duration>` | Required if `event` is the idle-timeout event. |
| `spans` | `Vec<SpanRule>` | Span declarations. |
| `attributes` | `Vec<AttributeRule>` | Attribute rules. |
| `events` | `Vec<EventRule>` | Span events. |
| `baggage` | `Vec<BaggageRule>` | Baggage propagation rules. |
| `instruments` | `Vec<InstrumentRule>` | Counter / up-down counter / histogram / gauge rules. |
| `log_records` | `Vec<LogRecordRule>` | Log record rules. |
| `inject` | `Option<InjectRule>` | Where to put outgoing trace context. |
| `extract` | `Option<ExtractRule>` | Where to read incoming trace context. |
| `links` | `Vec<LinkRule>` | Inter-span links. |
| `finish` | `Vec<FinishRule>` | Span-finish rules. |
| `status` | `Option<StatusRule>` | OK / ERROR / UNSET decision rule. |

### `Group`

Mirrors `otel-group`. Triggered explicitly by an HAProxy rule action.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Required. |
| `scopes` | `Vec<String>` | Names of scopes this group activates. |

### `TracerPipeline`, `MeterPipeline`, `LoggerPipeline`

Each pipeline owns one SDK provider plus its exporter. All three share
a common `Exporter` config block.

`TracerPipeline`:

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Pipeline identifier. Default: `default`. |
| `sampler` | `Sampler` | One of `AlwaysOn`, `AlwaysOff`, `TraceIdRatio(f64)`, `ParentBased(Box<Sampler>)`. |
| `processors` | `Vec<SpanProcessor>` | Currently always one batch processor (config knobs only). |
| `exporter` | `Exporter` | Endpoint + protocol + TLS + headers + retry. |
| `resource` | `Resource` | OTel resource attributes (`service.name`, etc.). |

`MeterPipeline`:

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Default: `default`. |
| `reader` | `MetricReader` | Periodic reader with `interval`, `timeout`. |
| `aggregations` | `AggregationPolicy` | Per-instrument-name policy (sum, last_value, histogram, exp_histogram, drop, default). |
| `exporter` | `Exporter` | |
| `resource` | `Resource` | |

`LoggerPipeline`:

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Default: `default`. |
| `processor` | `LogProcessor` | Batch processor. |
| `exporter` | `Exporter` | |
| `resource` | `Resource` | |

### `Exporter`

| Field | Type | Notes |
| --- | --- | --- |
| `endpoint` | `Url` | Required. |
| `protocol` | `Protocol` | `Grpc` (default), `HttpProtobuf`, `HttpJson`. |
| `compression` | `Option<Compression>` | `Gzip`, `Zstd` (gRPC only), `None`. |
| `timeout` | `Duration` | Default 10 s. |
| `headers` | `BTreeMap<String, String>` | Custom outgoing headers (e.g., bearer tokens). |
| `tls` | `TlsConfig` | CA bundle path, mTLS cert/key, server-name override, insecure-skip-verify (defaults to `false`). |
| `retry` | `RetryConfig` | `enabled`, `max_attempts`, `initial_backoff`, `max_backoff`. |
| `queue` | `QueueConfig` | `max_size` (default 2048), `max_export_batch_size`, `scheduled_delay`. |

### `AttributeRule`, `EventRule`, `InstrumentRule`, `LogRecordRule`, …

Each rule has the common shape:

| Field | Type | Notes |
| --- | --- | --- |
| `key` | `String` | OTel attribute key, OR instrument name, OR event name. |
| `source` | `ValueSource` | `Literal(Value)` \| `Arg(String)` \| `Var(VarScope, String)` (v2). |
| `type` | `Option<AttributeType>` | Optional coercion target (`String`, `I64`, `F64`, `Bool`). |
| `condition` | `Option<AclRef>` | Optional ACL gating evaluation. |

`InstrumentRule` adds: `kind: InstrumentKind` (Counter, UpDownCounter, Histogram, Gauge), `unit: Option<String>`, `description: Option<String>`, `value_source: ValueSource`, `bounds: Option<Vec<f64>>` (for explicit-bounds histograms), `aggr: AggregationPolicy`, plus an `attributes: Vec<AttributeRule>` set of dimensions.

`LogRecordRule` adds: `severity: SeverityNumber`, `body: ValueSource`, `id: Option<i64>`, `event_name: Option<String>`, `span_ref: Option<String>` (link-by-name to a span declared in the same scope), `attributes: Vec<AttributeRule>`.

### `RuntimeKnobs`

Hub-local settings that are NOT part of the addon vocabulary. Live
under a TOML key like `[plugins.params.runtime]`.

| Field | Type | Default |
| --- | --- | --- |
| `max_in_flight_transactions` | `usize` | `100_000` |
| `correlation_handle_ttl` | `Duration` | `60s` |
| `shutdown_drain_timeout` | `Duration` | `5s` |
| `health_metrics_addr` | `Option<SocketAddr>` | `None` (relies on hub's metrics endpoint) |

---

## 2. Runtime / transactional state

### `CorrelationHandle`

A 128-bit opaque identifier, hex-encoded as a 32-character lowercase
string. Generated at request-side message time, returned to HAProxy as
the transaction-scoped (HAProxy `txn`) variable named per the operator's TOML, and accepted
back as a SPOE argument on the response-side message.

### `InFlightSpan`

Stored in `CorrelationStore: DashMap<CorrelationHandle, InFlightSpan>`.

| Field | Type | Notes |
| --- | --- | --- |
| `opened_at` | `Instant` | For TTL eviction. |
| `tracer_pipelines` | `Arc<PipelineSet>` | The provider set under which the span was opened. Held so reload doesn't drop the SDK while spans are still open. |
| `span` | `BoxedSpan` | The active span. |
| `context` | `OtelContext` | Current OTel context (parent, baggage). |
| `meter_attributes` | `Vec<KeyValue>` | Attributes captured at request-side for use by response-side metric instruments. |
| `pending_log_records` | `Vec<LogRecord>` | Log records queued for emission on span close (rare; usually emitted immediately). |

State transitions:

```text
                  request-side msg            response-side msg
       (none) ───────────────────────► OPEN ─────────────────────► (closed, exported)
                                          │
                                          │  TTL exceeded (no response)
                                          ▼
                                       ABANDONED (closed with status=ERROR
                                                 attribute "abandoned"=true)

                                          │  capacity exceeded (oldest evicted)
                                          ▼
                                       EVICTED (closed with status=ERROR
                                               attribute "evicted"=true,
                                               + drop counter incremented)
```

### `HealthSnapshot`

Exposed via the plugin's metrics surface (FR-020).

| Field | Notes |
| --- | --- |
| `in_flight_transactions` | Current count in `CorrelationStore`. |
| `per_pipeline.queue_depth` | One per (signal, pipeline) tuple. |
| `per_pipeline.dropped_total` | Cumulative drop count. |
| `per_pipeline.export_attempts_total` | Cumulative attempts. |
| `per_pipeline.export_failures_total` | Cumulative failures. |
| `per_pipeline.last_success_at` | Timestamp of last successful export (null if none yet). |
| `config_reload_total` | Cumulative successful reloads. |
| `config_reload_failures_total` | Cumulative failed reloads. |
| `correlation_evictions_by_ttl` | Cumulative TTL evictions. |
| `correlation_evictions_by_capacity` | Cumulative capacity evictions. |

These are exposed as Prometheus-style metrics on the hub's existing
metrics endpoint (the hub already exposes a `/metrics` endpoint per
its own README).

---

## 3. Identity, uniqueness, lifecycle invariants

- `CorrelationHandle` is unique within a single hub instance per SPOE-
  emitted handle generation; the random space (128 bits) makes
  collision negligible.
- A `CorrelationHandle` MUST NOT outlive its `InFlightSpan`. When the
  store removes an entry the corresponding span is finished and
  exported.
- An `InFlightSpan` holds an `Arc` to its provider set; the provider
  set is dropped only when its last span is finished.
- Reload swaps the active `Arc<PluginConfig>` atomically; in-flight
  spans keep operating against their captured config.
