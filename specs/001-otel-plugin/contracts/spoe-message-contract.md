# SPOE Message Contract

**Feature**: [../spec.md](../spec.md) | **Plan**: [../plan.md](../plan.md)

The plugin reacts to SPOE messages dispatched by the hub. This contract
defines, per message kind, which arguments the plugin reads and which
return variables it writes. HAProxy operators configure the SPOE
agent file to deliver these arguments and to round-trip the plugin's
return variables.

## Message kinds

The plugin recognises **three** kinds of incoming SPOE message,
distinguished by the operator-chosen `name` they bind to in `spoe-message`
declarations and by which arguments are present.

### A. `request-side` message — opens a span (and may emit metrics, logs)

Operator names this whatever they like (e.g., `otel-on-request`); the
plugin matches by looking up the message name in the `event` field
of configured `Scope` blocks.

**Required arguments** (operator MUST include in `spoe-message`):

| SPOE arg name | Type | Source (HAProxy fetch) | Used by |
| --- | --- | --- | --- |
| `_event` | string | Literal of the operator-chosen event name | Scope dispatch |

**Recommended arguments** (any subset; consumed by attribute / instrument / log rules that reference them as `$arg.<name>`):

| SPOE arg name | Type | Common HAProxy fetch | Used by |
| --- | --- | --- | --- |
| `method` | string | `method` | `http.request.method` attribute |
| `url` | string | `url` | `url.full` |
| `path` | string | `path` | `url.path` |
| `host` | string | `req.hdr(host)` | `server.address` |
| `scheme` | string | `var(txn.scheme)` or literal | `url.scheme` |
| `src` | string (IP) | `src` | `client.address` |
| `src_port` | int | `src_port` | `client.port` |
| `proto` | string | `req.proto` | `network.protocol.version` |
| `traceparent` | string | `req.hdr(traceparent)` | TraceContext extraction |
| `tracestate` | string | `req.hdr(tracestate)` | TraceContext extraction |
| `baggage` | string | `req.hdr(baggage)` | Baggage extraction |

Operators MAY include any other SPOE argument; rules in the active
scope can reference it as `$arg.<name>`.

**Return variables** (the plugin writes these for HAProxy to consume):

| Return variable name | Scope | Description |
| --- | --- | --- |
| `<plugin>.correlation` | session | Opaque 32-char hex correlation handle. Operator MUST round-trip this back as the `correlation` SPOE argument on the corresponding response-side message. |
| `<plugin>.traceparent_out` | session | New W3C `traceparent` carrying the HAProxy span as the new parent; operator applies it to the request to the backend via `http-request set-header traceparent %[var(sess.<plugin>.traceparent_out)]`. |
| `<plugin>.tracestate_out` | session | Updated `tracestate` to forward. |
| `<plugin>.baggage_out` | session | Updated `baggage` to forward. |

(`<plugin>` is the hub-assigned plugin name from `[[plugins]] name = "..."`.
The hub auto-prefixes return-variable names with the plugin name.)

### B. `response-side` message — closes the span (and may emit metrics, logs)

**Required arguments**:

| SPOE arg name | Type | Source | Used by |
| --- | --- | --- | --- |
| `_event` | string | Literal of the response-side event name | Scope dispatch |
| `correlation` | string | `var(sess.<plugin>.correlation)` | Look up `InFlightSpan` |

**Recommended arguments** (typical):

| SPOE arg name | Type | Common HAProxy fetch | Used by |
| --- | --- | --- | --- |
| `status` | int | `status` | `http.response.status_code`, status rule |
| `bytes_in` | int | `bytes_in` | metric instruments |
| `bytes_out` | int | `bytes_out` | metric instruments |
| `duration_ms` | int | `Td` (total transaction duration in ms) | histogram instrument |
| `srv_name` | string | `be_server` | `server.address` of selected backend |

**Return variables**: none required. The plugin MAY write a
`<plugin>.last_status` session variable for operator visibility of the
exported span's final status (`ok` / `error` / `unset`).

### C. `group-trigger` message — runs a named otel-group

Used when the operator binds an `otel-group` to a specific HAProxy
rule action (`http-request otel-group <filter-id> <group>` in addon
parlance, translated to a SPOE message with `_event` set to the
group's name in plugin parlance).

**Required arguments**:

| SPOE arg name | Type | Notes |
| --- | --- | --- |
| `_event` | string | `group:<name>`, where `<name>` is the group identifier. |
| `correlation` | string (optional) | If present, the group runs in the context of the named in-flight transaction. |

The group's scopes are evaluated in order; each may emit telemetry
following its own rules.

## Rules common to all message kinds

- The `_event` argument is reserved by the plugin; operators MUST NOT use the name `_event` for any other purpose.
- The `correlation` argument is reserved on response-side and group-trigger messages.
- Unknown SPOE argument names are NOT errors; rules reference what they need by name and ignore the rest.
- A missing `correlation` on a response-side message causes the plugin to emit a span with `status=ERROR` and attribute `otel.spoa.correlation_missing=true`, then increment the `correlation_missing_total` health counter.

## Out-of-scope events

HAProxy lifecycle events for which HAProxy cannot send a SPOE message
(e.g., the addon's `on-stream-start` hooked via filter callback) are
out of scope for v1. The plugin REJECTS configuration that references
such an event with a clear error pointing to the migration notes.
