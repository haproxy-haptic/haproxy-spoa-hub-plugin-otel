# Quickstart: OpenTelemetry SPOA Hub Plugin

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-04-26

This walkthrough takes you from a vanilla HAProxy + haproxy-spoa-hub
deployment to your first sampled distributed trace in under 30 minutes
(SC-001). It covers User Story 1 (P1 — distributed tracing).

## Prerequisites

- HAProxy ≥ 2.8 with SPOE support compiled in (the haproxytech images
  satisfy this)
- `haproxy-spoa-hub` already running (see its README for setup)
- An OpenTelemetry-compatible collector reachable from the hub host
  (any of: OTel Collector, Jaeger 2.x, Tempo, Honeycomb, Datadog OTLP
  endpoint, etc.). Default in this guide: `http://otel-collector:4317`
- Rust toolchain ≥ 1.86 if building the plugin from source

## 1. Build (or download) the plugin

```bash
git clone https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel.git
cd haproxy-spoa-hub-plugin-otel
cargo build --release
cp target/release/libotel_plugin.so /opt/spoa-plugins/
```

(Pre-built binaries for `amd64`, `arm64`, and `armv7` will be available
on the project's releases page once v1 ships, matching the sibling
plugins' distribution pattern.)

## 2. Configure the hub

Add the OTEL plugin to your hub config (`/etc/haproxy-spoa-hub/config.toml`):

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
scopes = ["http-request", "http-response"]

[[plugins.params.tracer_pipelines]]
name = "default"
sampler = { kind = "parent_based", root = { kind = "trace_id_ratio", ratio = 1.0 } }

[plugins.params.tracer_pipelines.exporter]
endpoint = "http://otel-collector:4317"
protocol = "grpc"
timeout  = "10s"

[plugins.params.tracer_pipelines.resource]
"service.name"      = "haproxy-edge"
"service.version"   = "1.0.0"
"deployment.environment" = "production"

[[plugins.params.scopes]]
name  = "http-request"
event = "otel-on-request"

[[plugins.params.scopes.spans]]
name = "haproxy.request"
root = false  # the plugin makes it root only if no traceparent was supplied

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
use_headers = true   # read traceparent / tracestate / baggage from $arg.traceparent etc.

[plugins.params.scopes.inject]
prefix      = ""
use_headers = true   # write traceparent_out / tracestate_out / baggage_out as session vars

[[plugins.params.scopes]]
name  = "http-response"
event = "otel-on-response"

[[plugins.params.scopes.attributes]]
key    = "http.response.status_code"
source = "$arg.status"

[plugins.params.scopes.status]
code      = "error"
condition = "$arg.status >= 500"

[[plugins.params.scopes.finish]]
name = "haproxy.request"   # finish the span declared in the http-request scope
```

Reload the hub (`SIGHUP haproxy-spoa-hub` or `systemctl reload`).
The hub's logs should show:

```
plugin loaded plugin=otel version=...
listening transport=unix address=/var/run/haproxy-spoa-hub.sock
```

## 3. Configure HAProxy

### SPOE agent file (`/etc/haproxy/spoe-otel.conf`)

```
[spoa-otel]
spoe-agent otel-agent
    messages otel-on-request otel-on-response
    option var-prefix otel
    timeout hello      2s
    timeout idle       120s
    timeout processing 50ms
    use-backend spoa-hub-backend

spoe-message otel-on-request
    args _event=str(otel-on-request) method=method path=path host=req.hdr(host) src=src \
         traceparent=req.hdr(traceparent) tracestate=req.hdr(tracestate) baggage=req.hdr(baggage)
    event on-frontend-http-request

spoe-message otel-on-response
    args _event=str(otel-on-response) correlation=var(sess.otel.otel.correlation) \
         status=status
    event on-http-response
```

### Frontend (`/etc/haproxy/haproxy.cfg`)

```haproxy
frontend http-in
    bind *:8080
    filter spoe engine spoa-otel config /etc/haproxy/spoe-otel.conf

    # Inject the new trace context into the request to the backend.
    http-request set-header traceparent %[var(sess.otel.otel.traceparent_out)] \
        if { var(sess.otel.otel.traceparent_out) -m found }
    http-request set-header tracestate %[var(sess.otel.otel.tracestate_out)] \
        if { var(sess.otel.otel.tracestate_out) -m found }
    http-request set-header baggage %[var(sess.otel.otel.baggage_out)] \
        if { var(sess.otel.otel.baggage_out) -m found }

    default_backend app

backend spoa-hub-backend
    mode spop
    server hub /var/run/haproxy-spoa-hub.sock

backend app
    server app1 127.0.0.1:9000
```

Reload HAProxy (`systemctl reload haproxy`).

## 4. Verify

### Without an upstream trace

```bash
curl -i http://localhost:8080/hello
```

In your collector you should see one span:

- `name`: `haproxy.request`
- attributes: `http.request.method=GET`, `url.path=/hello`,
  `server.address=localhost:8080`, `client.address=127.0.0.1`,
  `http.response.status_code=200`
- `status`: `OK`

### With an upstream trace

```bash
curl -i \
  -H 'traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01' \
  http://localhost:8080/hello
```

In your collector you should see one span sharing the trace ID
`0af7651916cd43dd8448eb211c80319c` and pointing at parent
`b7ad6b7169203331`. The backend (`app1`) sees the request with a
new `traceparent` header whose span ID equals the HAProxy span's ID,
so any further-instrumented service appears as a child.

## 5. Trace context lifecycle

Three operator-facing behaviours that the wiring above implements but
that aren't obvious from the config alone:

### 5.1 No inbound `traceparent` → plugin starts a new trace

The plugin's `extract` rule reads `traceparent` (and `tracestate`,
`baggage`) from the SPOE arguments. If those arguments are missing —
either because the client didn't send a `traceparent` header, or
because HAProxy wasn't told to pass it via `req.hdr(traceparent)` —
the propagator returns an empty parent context. The plugin then
builds a **root span**: fresh 128-bit `trace_id`, fresh 64-bit
`span_id`. The W3C TraceContext propagator serialises that into
`traceparent_out`, which the operator's `http-request set-header
traceparent ...` rule applies to the request that goes to the backend.

The operator does not need to do anything different for this case —
the same config covers both. There is one edge condition:
**malformed inbound `traceparent`** (wrong shape, invalid hex, etc.)
is treated as if the header were absent: a new root trace starts.
This is the integration test
`malformed_traceparent_starts_new_root_trace` in `tests/us1_tracing.rs`.

### 5.2 Inbound `traceparent` exists → plugin builds a child span

When `traceparent` arrives intact, the propagator extracts a
`SpanContext` from it. The new HAProxy span is built as a **child**:
same `trace_id`, **new** `span_id`, parent_span_id = the inbound
span_id. Sampling decisions are inherited via the configured sampler
(default `parent_based(trace_id_ratio_based(1.0))` — honour upstream,
otherwise sample everything).

`traceparent_out` carries the same `trace_id` with the *new* span_id,
so when HAProxy applies `http-request set-header traceparent`, the
backend service sees a properly-chained context. Same for
`tracestate_out` and `baggage_out`. The integration test
`child_span_with_traceparent_round_trip` in `tests/us1_tracing.rs`
asserts this end-to-end.

`tracestate_out` is also truncated to the W3C-mandated 512-octet cap
(rightmost list-members dropped first); see
`src/propagation/injector.rs::truncate_tracestate`.

### 5.3 Returning the trace ID to the client

By default the plugin only sets `traceparent` on the **request to the
backend**, not on the **response back to the client**. Operators who
want the client to see the trace ID — for log correlation,
support-ticket workflows, or distributed-tracing UIs — do this with
HAProxy's `http-response set-header`. Three common patterns:

**Echo the full W3C traceparent** (idiomatic; lets clients chain
their own spans against HAProxy):

```haproxy
http-response set-header traceparent \
    %[var(sess.otel.otel.traceparent_out)] \
    if { var(sess.otel.otel.traceparent_out) -m found }
```

**Expose just the trace ID under a custom name** (operator-friendly,
easy to grep in logs):

```haproxy
# `traceparent_out` is `00-<trace_id>-<span_id>-<flags>`; field(2,-)
# extracts the trace_id substring.
http-response set-header X-Trace-Id \
    %[var(sess.otel.otel.traceparent_out),field(2,-)] \
    if { var(sess.otel.otel.traceparent_out) -m found }
```

**Use the W3C Server-Timing convention** (so browsers can correlate
in DevTools without a custom header):

```haproxy
http-response set-header Server-Timing \
    "traceparent;desc=\"%[var(sess.otel.otel.traceparent_out)]\"" \
    if { var(sess.otel.otel.traceparent_out) -m found }
```

All three rely on the same `traceparent_out` session variable that
the request-side plugin already sets — no plugin-side change needed,
just an extra HAProxy rule on the response phase. The `if { ... -m
found }` guard keeps the rule a no-op for requests that bypassed the
SPOE filter.

## 6. Plugin health and metrics



The plugin exposes its own health surface (FR-020) on the hub's metrics
endpoint:

```bash
curl http://localhost:9090/metrics | grep '^otel_'
```

Expected counters/gauges:

- `otel_in_flight_transactions` — current correlation-map size
- `otel_pipeline_queue_depth{pipeline="default",signal="trace"}` — gauge
- `otel_pipeline_dropped_total{pipeline="default",signal="trace"}` — counter
- `otel_pipeline_export_attempts_total{pipeline="default",signal="trace"}`
- `otel_pipeline_export_failures_total{pipeline="default",signal="trace"}`
- `otel_pipeline_last_export_success_seconds{pipeline="default",signal="trace"}`
- `otel_correlation_evictions_total{reason="ttl"|"capacity"}`

If `dropped_total` grows or `last_export_success_seconds` falls behind,
the plugin is the cause of an incident; that is the diagnostic workflow
SC-008 prescribes.

## 7. Adding metrics and logs

Once tracing is working, extend the same `[[plugins.params.scopes]]`
blocks with `instruments` and `log_records` arrays. See
[`contracts/addon-directive-mapping.md`](./contracts/addon-directive-mapping.md)
for the full TOML vocabulary.

A complete example for User Story 2 (HTTP request counter and latency
histogram) and User Story 3 (log records on rejection):

```toml
# Add a meter pipeline
[[plugins.params.meter_pipelines]]
name = "default"

[plugins.params.meter_pipelines.exporter]
endpoint = "http://otel-collector:4317"
protocol = "grpc"

[plugins.params.meter_pipelines.reader]
interval = "60s"

# Add a counter and a histogram to the response scope
[[plugins.params.scopes.instruments]]
kind  = "cnt_int"
name  = "http.server.requests"
value = 1

  [[plugins.params.scopes.instruments.attributes]]
  key    = "status_class"
  source = "$arg.status_class"

[[plugins.params.scopes.instruments]]
kind  = "hist_int"
name  = "http.server.duration"
unit  = "ms"
value = "$arg.duration_ms"
bounds = [5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000]
```

## 8. Migrating to the built-in addon when it ships

Once HAProxy's built-in OTEL addon is generally available you can
switch by:

1. Translating your TOML configuration to the addon's `.cfg` and
   `.yml` syntax using the rules in
   [`contracts/addon-directive-mapping.md`](./contracts/addon-directive-mapping.md)
   (a translation tool is planned post-v1).
2. Replacing the SPOE filter line in `haproxy.cfg` with the addon's
   `filter opentelemetry config <file>` directive.
3. Removing the SPOE agent file and the `spoa-hub-backend` if no other
   plugin needs it.

The translation rules are mechanical and round-trip per spec
clarification Q2 — your telemetry semantics carry over unchanged.

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `plugin failed to load: validation error at path '...'` | TOML key path not recognised by the plugin | Cross-reference [`contracts/addon-directive-mapping.md`](./contracts/addon-directive-mapping.md) — the directive may be unsupported in v1. |
| Spans appear without parent context even though `traceparent` is sent | The SPOE message doesn't include `traceparent` as a SPOE arg | Confirm `traceparent=req.hdr(traceparent)` is in your `spoe-message otel-on-request` definition. |
| Spans never close | The response-side message lacks `correlation` | Confirm `correlation=var(sess.otel.otel.correlation)` is in your `spoe-message otel-on-response` and the `var-prefix` matches. |
| Hub log shows `correlation_missing_total` rising | Same as above, on a subset of requests | Check ACLs and route-specific filters for paths that may bypass the request-side message. |
| `otel_pipeline_dropped_total` rising under load | Collector unable to keep up; queue overflowing | Increase `[plugins.params.tracer_pipelines.exporter.queue].max_size`, lower `scheduled_delay`, or reduce sampling ratio. |
| `last_export_success_seconds` lagging | OTLP collector unreachable or rejecting payloads | Inspect the plugin's structured log; the failure reason is logged on every retry. |
