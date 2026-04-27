# HAProxy SPOA Hub — OpenTelemetry plugin

A [haproxy-spoa-hub](https://gitlab.com/haproxy-haptic/haproxy-spoa-hub)
plugin that emits OpenTelemetry traces, metrics, and logs from HAProxy
traffic over OTLP. Mirrors the directive vocabulary of HAProxy's built-in
OpenTelemetry addon so operators can adopt OTel today and migrate to the
built-in addon mechanically when it reaches general availability.

## Overview

The plugin runs inside `haproxy-spoa-hub` and is wired to HAProxy via
the SPOE protocol. HAProxy fires SPOE messages at the request lifecycle
points the operator chooses; the plugin builds spans, records metric
data points, and emits log records, then exports them via OTLP/gRPC
or OTLP/HTTP (Protobuf or JSON) to an OpenTelemetry-compatible backend.

**Key features**:

- Distributed tracing with W3C TraceContext + Baggage propagation,
  span attributes / events / links / status, and four sampler types
  (`always_on`, `always_off`, `trace_id_ratio`, `parent_based`).
- Metrics: counter, up-down counter, histogram (explicit bounds and
  exponential), and gauge — with attribute dimensions sourced from
  SPOE arguments.
- OTLP log records, optionally linked to the active span for the
  current transaction.
- Multiple pipelines per signal — each can target a different OTLP
  endpoint with its own sampler, exporter protocol, TLS settings,
  headers, and retry policy.
- Configuration vocabulary mirrors HAProxy's built-in OTEL addon
  one-to-one (see [migration](#migrating-to-or-from-the-built-in-otel-addon)).
- Fail-open semantics: telemetry export errors never block, slow,
  or fail an HAProxy request.
- Self-observability: per-pipeline queue depth, drop counts, export
  attempts/failures, last-success timestamps, and correlation-map
  health, all on the hub's Prometheus endpoint.

## Prerequisites

- HAProxy ≥ 2.8 with SPOE support compiled in (the haproxytech images
  satisfy this).
- `haproxy-spoa-hub` already running. See its
  [README](https://gitlab.com/haproxy-haptic/haproxy-spoa-hub) for
  setup.
- An OpenTelemetry-compatible backend (OTel Collector, Tempo, Jaeger
  2.x, Honeycomb, Datadog OTLP endpoint, etc.).
- Rust toolchain ≥ 1.86 if building the plugin from source.

## Install

Pre-built binaries for `amd64`, `arm64`, and `armv7` (glibc ≥ 2.36,
Debian bookworm baseline) are published on the project's
[release page](https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/-/releases).

To build from source:

```bash
git clone https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel.git
cd haproxy-spoa-hub-plugin-otel
cargo build --release
cp target/release/libotel_plugin.so /opt/spoa-plugins/
```

## Configure the hub

Add the plugin to your hub config (e.g. `/etc/haproxy-spoa-hub/config.toml`):

```toml
plugin_dir = "/opt/spoa-plugins"
default_timeout_ms = 100
log_level = "info"
metrics_addr = "0.0.0.0:9090"

[[listeners]]
type = "unix"
address = "/var/run/haproxy-spoa-hub.sock"

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
timeout  = "10s"
[plugins.params.tracer_pipelines.resource]
"service.name"          = "haproxy-edge"
"service.version"       = "1.0.0"
"deployment.environment" = "production"

[[plugins.params.scopes]]
name  = "request"
event = "otel-on-request"
kind  = "request"

[[plugins.params.scopes.spans]]
name = "haproxy.request"

[[plugins.params.scopes.attributes]]
key    = "http.request.method"
source = "$arg.method"

[[plugins.params.scopes.attributes]]
key    = "url.path"
source = "$arg.path"

[[plugins.params.scopes.attributes]]
key    = "server.address"
source = "$arg.host"

[[plugins.params.scopes.attributes]]
key    = "client.address"
source = "$arg.src"

[plugins.params.scopes.extract]
prefix      = ""
use_headers = true

[plugins.params.scopes.inject]
prefix      = ""
use_headers = true

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

Reload the hub (`systemctl reload haproxy-spoa-hub` or `SIGHUP`).
The hub log should show `plugin loaded plugin=otel` and
`listening transport=unix address=…`.

## Configure HAProxy

### SPOE agent file (`/etc/haproxy/spoe-otel.conf`)

```
[spoa-otel]
spoe-agent otel-agent
    messages otel-on-request otel-on-response
    option var-prefix otel
    option continue-on-error
    timeout hello      2s
    timeout idle       2m
    timeout processing 100ms
    use-backend spoa-hub-backend

spoe-message otel-on-request
    args _event=str(otel-on-request) method=method path=path host=req.hdr(host) src=src \
         traceparent=req.hdr(traceparent) tracestate=req.hdr(tracestate) baggage=req.hdr(baggage)
    event on-frontend-http-request

spoe-message otel-on-response
    args _event=str(otel-on-response) correlation=var(txn.otel.otel.correlation) status=status
    event on-http-response
```

### Frontend (`/etc/haproxy/haproxy.cfg`)

```haproxy
frontend http-in
    bind *:8080
    filter spoe engine spoa-otel config /etc/haproxy/spoe-otel.conf

    # Apply the plugin's outbound TraceContext to the request to the backend.
    http-request set-header traceparent %[var(txn.otel.otel.traceparent_out)] \
        if { var(txn.otel.otel.traceparent_out) -m found }
    http-request set-header tracestate  %[var(txn.otel.otel.tracestate_out)] \
        if { var(txn.otel.otel.tracestate_out) -m found }
    http-request set-header baggage     %[var(txn.otel.otel.baggage_out)] \
        if { var(txn.otel.otel.baggage_out) -m found }

    default_backend app

backend spoa-hub-backend
    mode tcp
    server hub /var/run/haproxy-spoa-hub.sock maxconn 256

backend app
    server app1 127.0.0.1:9000
```

Reload HAProxy (`systemctl reload haproxy`).

### Why `txn` (transaction) scope and not `sess` (session)?

All return variables the plugin emits use HAProxy's **transaction**
scope. Under HTTP/2 a single TCP session carries multiple concurrent
streams; a session-scoped slot would let one stream's correlation
handle or `traceparent_out` overwrite another's, ending the wrong
span. Transaction scope gives each HTTP/2 stream its own slot.

## Verify

Send a request and confirm a span lands in your collector:

```bash
curl -i http://localhost:8080/hello
```

Expected span:

- `name`: `haproxy.request`
- attributes: `http.request.method=GET`, `url.path=/hello`,
  `server.address=...`, `client.address=...`, `http.response.status_code=200`
- `status`: `OK`

With an inbound `traceparent`, the plugin builds a child span with
the same trace_id:

```bash
curl -i \
  -H 'traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01' \
  http://localhost:8080/hello
```

The backend (`app1`) receives a *new* `traceparent` whose span ID
matches the HAProxy span — so any further-instrumented service
appears as the next hop in the same trace.

## Trace context lifecycle

The plugin handles all three W3C TraceContext cases automatically:

### No inbound `traceparent` → plugin starts a new trace

If no `traceparent` arrives — either the client didn't send one or
HAProxy isn't passing it via `req.hdr(traceparent)` — the propagator
returns an empty parent context. The plugin builds a **root span**
(fresh 128-bit trace_id, fresh span_id) and writes a brand-new
`traceparent_out`. The same `http-request set-header traceparent`
rule applies. **Malformed inbound `traceparent`** (wrong shape,
invalid hex) is treated identically to "no traceparent": the plugin
falls back to a new root.

### Inbound `traceparent` exists → plugin builds a child span

The propagator extracts a `SpanContext`. The new HAProxy span is a
**child** (same trace_id, new span_id, parent_span_id = inbound
span_id). Sampling is inherited via the configured sampler (default
`parent_based(trace_id_ratio_based(1.0))` — honour upstream,
otherwise sample everything). The plugin writes the new context as
`traceparent_out` / `tracestate_out` / `baggage_out`. `tracestate`
is truncated to the W3C-mandated 512-octet cap (rightmost
list-members dropped first).

### Returning the trace ID to the client

Not done by default — the plugin only sets `traceparent` on the
**request to the backend**. If you want the client to see the trace
ID (for log correlation, support tickets, browser DevTools), add an
`http-response set-header` rule. Three patterns:

```haproxy
# Echo the full W3C traceparent so the client can chain its own spans.
http-response set-header traceparent \
    %[var(txn.otel.otel.traceparent_out)] \
    if { var(txn.otel.otel.traceparent_out) -m found }
```

```haproxy
# Operator-friendly trace_id under a custom name. `field(2,-)` extracts
# the trace_id substring from `00-<trace_id>-<span_id>-<flags>`.
http-response set-header X-Trace-Id \
    %[var(txn.otel.otel.traceparent_out),field(2,-)] \
    if { var(txn.otel.otel.traceparent_out) -m found }
```

```haproxy
# W3C Server-Timing — visible in browser DevTools without extra tooling.
http-response set-header Server-Timing \
    "traceparent;desc=\"%[var(txn.otel.otel.traceparent_out)]\"" \
    if { var(txn.otel.otel.traceparent_out) -m found }
```

## TLS for OTLP exporters

Each pipeline's exporter supports TLS with a per-pipeline trust
store and optional mTLS:

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
- `server_name` overrides SNI / certificate-verification hostname.
- `insecure_skip_verify = true` disables certificate validation entirely.
  Not the default; the plugin logs a warning at startup when an
  operator opts in.

For HTTP-based OTLP (`protocol = "http/protobuf"` or `"http/json"`)
the same TLS knobs apply.

## Self-observability

The plugin exposes its own health surface on the hub's existing
metrics endpoint (default `:9090/metrics`):

| Metric | Type | Description |
| --- | --- | --- |
| `otel_in_flight_transactions` | gauge | Current correlation-map size |
| `otel_pipeline_queue_depth{pipeline,signal}` | gauge | Per-pipeline export queue depth |
| `otel_pipeline_dropped_total{pipeline,signal}` | counter | Cumulative drops on queue overflow |
| `otel_pipeline_export_attempts_total{pipeline,signal}` | counter | Cumulative export attempts |
| `otel_pipeline_export_failures_total{pipeline,signal}` | counter | Cumulative export failures |
| `otel_pipeline_last_export_success_seconds{pipeline,signal}` | gauge | Unix timestamp of last successful export |
| `otel_correlation_evictions_total{reason}` | counter | TTL or capacity evictions |
| `otel_correlation_missing_total` | counter | Response-side messages without correlation |
| `otel_config_reload_total` | counter | Successful config reloads |
| `otel_config_reload_failures_total` | counter | Failed config reloads |

These are sufficient to determine that the plugin is the cause of
an incident without reading source code: if `dropped_total` rises,
the queue is overflowing; if `last_export_success_seconds` lags,
the OTLP endpoint is unreachable; if `correlation_missing_total`
rises, a SPOE message wiring problem.

## Migrating to or from the built-in OTEL addon

The plugin's TOML key paths mirror the directive names and nesting
structure of HAProxy's built-in OTEL addon one-to-one. To migrate:

1. Identify the addon directive you'd write in `.cfg` / `.yml`.
2. Look up the corresponding TOML key path in
   [`contracts/addon-directive-mapping.md`](./specs/001-otel-plugin/contracts/addon-directive-mapping.md).
3. Translate. Sample-expression sources map to `$arg.<name>` references
   to SPOE arguments — operators must wire the corresponding
   `spoe-message ... args ...` fetches in `haproxy.cfg`.

A migration tool that performs the translation automatically is
planned post-v1. Today the translation is mechanical and the plugin's
configuration errors name the addon directive equivalent of any TOML
key path that fails validation.

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `plugin failed to load: validation error at path '...'` | TOML key path not recognised by the plugin | Cross-reference [`contracts/addon-directive-mapping.md`](./specs/001-otel-plugin/contracts/addon-directive-mapping.md) — the directive may be unsupported in v1. |
| Spans appear without parent context even though `traceparent` is sent | `spoe-message` doesn't include `traceparent` as an arg | Confirm `traceparent=req.hdr(traceparent)` is in your `spoe-message otel-on-request` definition. |
| Spans never close | Response-side message lacks `correlation` | Confirm `correlation=var(txn.otel.otel.correlation)` is in `spoe-message otel-on-response` and the `var-prefix` matches. |
| Hub log shows `correlation_missing_total` rising | Same as above for some routes | Check ACLs / route-specific filters for paths bypassing the request-side message. |
| `otel_pipeline_dropped_total` rising under load | Collector unable to keep up; queue overflowing | Increase `[plugins.params.tracer_pipelines.exporter.queue].max_size`, lower `scheduled_delay`, or reduce sampling ratio. |
| `last_export_success_seconds` lagging | OTLP collector unreachable or rejecting payloads | Inspect plugin's structured log; failure reason logged on every retry. |
| HTTP/2 streams seeing each other's traces | Vars set at `sess` scope (must be `txn`) | If you've customised the haproxy config, confirm every `var(...otel...)` reference uses `txn.` not `sess.`. |

## Project documents

For implementation details, design rationale, and contributor reference:

- [Feature specification](./specs/001-otel-plugin/spec.md)
- [Implementation plan](./specs/001-otel-plugin/plan.md)
- [Data model](./specs/001-otel-plugin/data-model.md)
- [Quickstart walkthrough](./specs/001-otel-plugin/quickstart.md)
- [SPOE message contract](./specs/001-otel-plugin/contracts/spoe-message-contract.md)
- [Addon directive ↔ TOML mapping](./specs/001-otel-plugin/contracts/addon-directive-mapping.md)
- [JSON Schema for `[plugins.params]`](./specs/001-otel-plugin/contracts/plugin-config-schema.json)
- [Soak test procedure](./docs/SOAK.md)
- [Hot-path benchmarks](./docs/BENCHMARKS.md)

## Constitution

This project follows the principles in
[`.specify/memory/constitution.md`](./.specify/memory/constitution.md):
code quality, testing standards, UX consistency, performance
requirements. Every functional requirement maps to at least one
automated test; every change to the hot path includes benchmark
results.

## License

Apache-2.0.
