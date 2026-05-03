# SPOE Message Contract — Per-Message Override Delta

**Feature**: [../spec.md](../spec.md) | **Base contract**: [`/specs/001-otel-plugin/contracts/spoe-message-contract.md`](../../001-otel-plugin/contracts/spoe-message-contract.md)

This delta extends the request-side message contract with two new reserved arguments. All other parts of the contract (response-side message, group-trigger message, return variables, scoping rules) are unchanged.

## Reserved arguments — request-side message

The base contract reserves `_event` for scope dispatch. This feature adds two more reserved arguments under the same `_`-prefix discipline.

| SPOE arg name | Type | Value space | Semantics |
| --- | --- | --- | --- |
| `_event` | string | operator-defined event name | (unchanged — base contract) Scope dispatch. |
| `_root` | string | `"true"` \| `"false"` | Per-message override of the active scope's `Scope.spans[].root` setting. Present-and-parseable wins over the per-scope default for this transaction only. |
| `_sample` | string | `"default"` \| `"record"` \| `"drop"` | Per-message override of the configured sampler's decision. `record` forces span emission and export regardless of the sampler. `drop` skips span emission and correlation allocation entirely. `default` (or absent) defers to the configured sampler. |

### Rules common to `_root` and `_sample`

- **Request-side only.** Both arguments are recognised on request-side messages. Their presence on a response-side or group-trigger message is silently ignored (not an error). Operators MAY symmetrically wire the same SPOE-argument set on both messages without conditional HAProxy config; the plugin treats them as no-ops on the response side.
- **Reserved prefix.** The leading underscore signals that the argument is plugin-controlled. Operators MUST NOT use these names for any other purpose. Operator-defined attribute or instrument rules that reference `$arg._root` or `$arg._sample` will see the literal SPOE-argument string value (e.g., the string `"true"`) — matching the existing `_event` behaviour, but discouraged.
- **Malformed values fail open.** A value outside the documented set, or the wrong SPOE type, is treated as if the argument were absent. The plugin emits one WARN log line per malformed override (truncated value for safety) and increments the `malformed_override_total` health counter labelled by argument name and scope name. The malformed value itself is NOT used as a metric label (high-cardinality / attacker-controlled).
- **Override precedence.** Per-message argument > per-scope TOML default. Reasoning: scope defaults express the operator's policy; per-message arguments express per-route exceptions. This matches the "ingress annotation overrides controller default" expectation common in Kubernetes ingress controllers.

### Interaction matrix

| `_root` | `_sample` | Effect |
| --- | --- | --- |
| absent | absent | (current behaviour) Scope's `root` decides parent context; configured sampler decides export. |
| `"true"` | absent / `"default"` | Fresh root context; sampler decides export. `traceparent_out` carries the new root trace ID. |
| `"false"` | absent / `"default"` | Inbound `traceparent` extracted as parent (explicit form of default; no-op when scope's `root` is already false). |
| absent | `"record"` | Scope's `root` decides parent context; span is created and exported regardless of sampler. |
| absent | `"drop"` | No span created, no correlation handle allocated. Instruments and log records still fire. `traceparent_out`, `tracestate_out`, `baggage_out` are NOT emitted (no span context to inject). |
| `"true"` | `"drop"` | Fresh root context computed; no span exported. `traceparent_out` carries the synthetic new root trace ID so downstream services see a coherent trace identity for this request even though HAProxy's local span was dropped. |
| `"true"` | `"record"` | Fresh root context; span created and exported regardless of sampler. |
| malformed | * | Override ignored, scope default applies; metrics/logs per the resolved `_sample` value. Counter increments. |
| * | malformed | Override ignored, sampler decides; metrics/logs as configured. Counter increments. |

The "force fresh-root + drop span + inject context" combination is intentional: it lets HAPTIC operators say "for this Ingress, sever the inbound trace at the edge but still give downstreams a fresh trace ID so their own spans link together". Removing the synthetic context injection in that combination would force the operator to use an alternative propagator (or accept un-traced downstreams), which defeats the point of running otel at the ingress.

### Backward compatibility

- Existing operator configurations without `_root` / `_sample` arguments take the same code path as today. The arguments are strictly additive.
- Existing `tests/us1_tracing.rs` cases (root, child-with-traceparent, malformed-traceparent, etc.) continue to pass without modification.
- The reserved-name list grows by two entries. Operators currently using SPOE arguments named `_root` or `_sample` (unlikely, given the underscore-prefix convention is already documented as plugin-reserved) will see those values consumed by the new override logic. The base contract already explicitly states "Operators MUST NOT use the name `_event` for any other purpose"; the same rule extends to `_root` and `_sample`.

## Health-counter additions

This feature adds one health counter to the plugin's existing `/metrics` surface.

| Counter | Type | Labels | Increment trigger |
| --- | --- | --- | --- |
| `malformed_override_total` | counter | `arg_name` (`_root` \| `_sample`), `scope_name` | Once per malformed value seen on a request-side message. |

The counter is reported alongside the existing `correlation_missing_total`, queue-depth, and drop counters.
