# HAProxy SPOA Hub — OpenTelemetry plugin

A [haproxy-spoa-hub](https://gitlab.com/haproxy-haptic/haproxy-spoa-hub)
plugin that emits OpenTelemetry traces, metrics, and logs from HAProxy
traffic over OTLP. Mirrors the directive vocabulary of HAProxy's built-in
OpenTelemetry addon so operators can adopt OTel today and migrate to the
built-in addon mechanically when it reaches general availability.

## Overview

The plugin runs inside `haproxy-spoa-hub` and is wired to HAProxy via
the SPOE protocol. HAProxy sends SPOE messages at request lifecycle
points the operator chooses; the plugin builds spans, records metric
data points, and emits log records, then exports them via OTLP/gRPC
(or OTLP/HTTP) to an OpenTelemetry-compatible backend.

**Key features**:

- Distributed tracing with W3C TraceContext + Baggage propagation, span
  attributes, events, links, status, and four sampler types
  (always-on, always-off, ratio-based, parent-based).
- Metrics across counter, up-down counter, histogram (explicit bounds
  and exponential), and gauge instrument types, with attribute
  dimensions sourced from SPOE arguments.
- OTLP log records optionally linked to the active in-flight span.
- Multiple pipelines per signal — operators can target different OTLP
  endpoints with their own samplers, exporter protocols, TLS settings,
  headers, and retry policies.
- Configuration vocabulary that mirrors HAProxy's built-in OTEL addon
  one-to-one. See [the migration mapping](./specs/001-otel-plugin/contracts/addon-directive-mapping.md).
- Fail-open semantics: telemetry export errors never block, slow, or
  fail an HAProxy request.
- Self-observability: per-pipeline queue depth, drop count, export
  attempts/failures, last successful export timestamp, plus
  correlation map health, exposed through the hub's existing
  Prometheus endpoint.

## Quick start

See [`specs/001-otel-plugin/quickstart.md`](./specs/001-otel-plugin/quickstart.md)
for a walkthrough that takes a vanilla HAProxy + hub deployment to a
first sampled distributed trace in under 30 minutes.

A minimal `[plugins.params]` block:

```toml
[[plugins]]
name = "otel"
library = "libotel_plugin.so"
messages = ["otel-on-request", "otel-on-response"]

[plugins.params.instrumentation]
name = "haproxy-edge"
scopes = ["request", "response"]

[[plugins.params.tracer_pipelines]]
name = "default"
[plugins.params.tracer_pipelines.sampler]
kind = "parent_based"
[plugins.params.tracer_pipelines.sampler.root]
kind = "trace_id_ratio"
ratio = 1.0
[plugins.params.tracer_pipelines.exporter]
endpoint = "http://otel-collector:4317"
protocol = "grpc"
[plugins.params.tracer_pipelines.resource]
"service.name" = "haproxy-edge"

[[plugins.params.scopes]]
name  = "request"
event = "otel-on-request"
kind  = "request"
[[plugins.params.scopes.spans]]
name = "haproxy.request"
[[plugins.params.scopes.attributes]]
key    = "http.request.method"
source = "$arg.method"

[[plugins.params.scopes]]
name  = "response"
event = "otel-on-response"
kind  = "response"
[[plugins.params.scopes.attributes]]
key    = "http.response.status_code"
source = "$arg.status"
[plugins.params.scopes.status]
code      = "error"
condition = "$arg.status >= 500"
```

## Trace context lifecycle

The plugin handles all three W3C TraceContext cases automatically once
the SPOE wiring in the quickstart is in place:

- **No inbound `traceparent`** → the plugin starts a new root trace and
  emits a fresh `traceparent_out` for the request to the backend.
  Malformed inbound `traceparent` is treated the same way (new root).
- **Inbound `traceparent`** → the plugin builds a child span (same
  trace_id, new span_id) and emits the updated context as
  `traceparent_out` / `tracestate_out` / `baggage_out`. `tracestate` is
  truncated to W3C's 512-octet cap.
- **Returning the trace ID to the client** → set a response header
  with HAProxy's `http-response set-header`. The plugin doesn't do
  this by default; the operator picks the convention they want
  (echoed `traceparent`, custom `X-Trace-Id`, or `Server-Timing`).

See [`specs/001-otel-plugin/quickstart.md`](./specs/001-otel-plugin/quickstart.md#5-trace-context-lifecycle)
§5 for the full HAProxy snippets.

## TLS for OTLP exporters

Each pipeline's exporter supports TLS with a per-pipeline trust store:

```toml
[plugins.params.tracer_pipelines.exporter.tls]
ca           = "/etc/haproxy/otlp-ca.pem"
client_cert  = "/etc/haproxy/otlp-client.pem"
client_key   = "/etc/haproxy/otlp-client.key"
server_name  = "otel-collector.internal"
insecure_skip_verify = false  # default — must be set explicitly to opt out
```

Behaviour:

- If `ca` is provided, it replaces the system trust store for that
  exporter; otherwise platform CA roots are used (via `rustls-native-certs`).
- `client_cert` + `client_key` together enable mTLS.
- `server_name` overrides SNI / certificate verification hostname.
- `insecure_skip_verify = true` disables certificate validation entirely.
  It is **not** the default and the plugin logs a warning at startup
  when an operator opts in.

For HTTP-based OTLP (`protocol = "http/protobuf"` or `"http/json"`), the
same TLS knobs apply.

## Migration to / from the built-in OTEL addon

The plugin's TOML key paths mirror the directive names and nesting
structure of HAProxy's built-in OTEL addon one-to-one. To migrate:

1. Identify the addon directive you'd write in `.cfg` / `.yml`.
2. Look up the corresponding TOML key path in
   [`contracts/addon-directive-mapping.md`](./specs/001-otel-plugin/contracts/addon-directive-mapping.md).
3. Translate. Sample-expression sources map to `$arg.<name>` references
   to SPOE arguments — the operator must wire the corresponding
   `spoe-message ... args ...` fetches in `haproxy.cfg`.

A migration tool that performs the translation automatically is planned
for a post-v1 release. Today the translation is mechanical and the
plugin's error messages name the addon directive equivalent of any
TOML key path that fails validation.

## Build

```bash
cargo build --release
cp target/release/libotel_plugin.so /opt/spoa-plugins/
```

Pre-built binaries for `amd64`, `arm64`, and `armv7` (glibc ≥ 2.36, Debian
bookworm baseline) will be available on the project's release page once
v1 ships.

## Self-observability

The plugin exposes its own health surface on the hub's existing
metrics endpoint (default `:9090/metrics`):

| Metric | Type | Description |
| --- | --- | --- |
| `otel_in_flight_transactions` | gauge | Current correlation map size |
| `otel_pipeline_queue_depth{pipeline,signal}` | gauge | Per-pipeline export queue depth |
| `otel_pipeline_dropped_total{pipeline,signal}` | counter | Cumulative drops on overflow |
| `otel_pipeline_export_attempts_total{pipeline,signal}` | counter | Cumulative export attempts |
| `otel_pipeline_export_failures_total{pipeline,signal}` | counter | Cumulative export failures |
| `otel_pipeline_last_export_success_seconds{pipeline,signal}` | gauge | Unix timestamp of last success |
| `otel_correlation_evictions_total{reason}` | counter | TTL or capacity evictions |
| `otel_correlation_missing_total` | counter | Response-side messages without correlation |
| `otel_config_reload_total` | counter | Successful reloads |
| `otel_config_reload_failures_total` | counter | Failed reloads |

Operators determine the plugin is the cause of an incident from these
metrics alone; reading source code is not required.

## Documents

- [Feature specification](./specs/001-otel-plugin/spec.md)
- [Implementation plan](./specs/001-otel-plugin/plan.md)
- [Data model](./specs/001-otel-plugin/data-model.md)
- [Quickstart](./specs/001-otel-plugin/quickstart.md)
- [SPOE message contract](./specs/001-otel-plugin/contracts/spoe-message-contract.md)
- [Addon directive ↔ TOML mapping](./specs/001-otel-plugin/contracts/addon-directive-mapping.md)
- [JSON Schema for `[plugins.params]`](./specs/001-otel-plugin/contracts/plugin-config-schema.json)
- [Soak test procedure](./docs/SOAK.md)

## Constitution

This project follows the principles in `.specify/memory/constitution.md`
(code quality, testing standards, UX consistency, performance
requirements). Every functional requirement maps to at least one
automated test; every change to the hot path must include benchmark
results.

## License

Apache-2.0.
