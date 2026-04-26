# Addon Directive ↔ TOML Key Path Mapping

**Feature**: [../spec.md](../spec.md) | **Plan**: [../plan.md](../plan.md)

This document defines the canonical, one-to-one translation between
HAProxy's built-in OpenTelemetry addon directives and the plugin's
TOML key paths inside `[plugins.params]`. Per spec FR-016 and
clarification Q2, the plugin only accepts TOML; the operator (or a
documented translation tool) converts the addon's `.cfg` and `.yml`
files to this layout.

The mapping is grouped by addon section. Anchors:

- The addon's `.cfg` instrumentation/scope/group sections map under
  `[plugins.params.instrumentation]`, `[[plugins.params.scopes]]`, and
  `[[plugins.params.groups]]`.
- The addon's `.yml` SDK pipeline sections map under
  `[[plugins.params.tracer_pipelines]]`, `[[plugins.params.meter_pipelines]]`,
  `[[plugins.params.logger_pipelines]]`.

## 1. `otel-instrumentation <name>`

| Addon directive | TOML key path | Notes |
| --- | --- | --- |
| `otel-instrumentation foo` | `[plugins.params.instrumentation]` `name = "foo"` | Section is singular; only one allowed. |
| `config <file>` | `[plugins.params.instrumentation]` `config_pipeline = "<pipeline-name>"` | The addon's `config <file>` references the YAML pipeline file; in TOML, this is the name of the tracer/meter/logger pipeline this instrumentation activates by default. |
| `acl <name> <criterion> ...` | `[[plugins.params.instrumentation.acls]]` `name = "..."`, `criterion = "..."`, `args = [...]` | |
| `log <args>` | `[plugins.params.instrumentation.log]` `<args>` | |
| `groups <g1> <g2> ...` | `[plugins.params.instrumentation]` `groups = ["g1", "g2", ...]` | |
| `scopes <s1> <s2> ...` | `[plugins.params.instrumentation]` `scopes = ["s1", "s2", ...]` | |
| `rate-limit <n> <window>` | `[plugins.params.instrumentation.rate_limit]` `count = <n>`, `window = "<window>"` | |
| `debug-level <0..4>` | `[plugins.params.instrumentation]` `debug_level = <0..4>` | |
| `option disabled` | `[plugins.params.instrumentation.option]` `disabled = true` | |
| `option dontlog-normal` | `[plugins.params.instrumentation.option]` `dontlog_normal = true` | |
| `option hard-errors` | `[plugins.params.instrumentation.option]` `hard_errors = true` | |

## 2. `otel-scope <name>`

| Addon directive | TOML key path | Notes |
| --- | --- | --- |
| `otel-scope foo` | `[[plugins.params.scopes]]` `name = "foo"` | Array-of-tables; one entry per scope. |
| `acl ...` | `[[plugins.params.scopes.acls]]` | |
| `idle-timeout <t>` | `[plugins.params.scopes]` `idle_timeout = "<t>"` | |
| `otel-event <name> [{ if \| unless } <cond>]` | `[plugins.params.scopes]` `event = "<name>"` and optionally `event_condition = { if \| unless = "..." }` | |
| `span <name> [parent <ref>] [link <ref>] [root]` | `[[plugins.params.scopes.spans]]` `name = "..."`, optional `parent = "..."`, `link = ["..."]`, `root = true` | |
| `attribute <key> <sample> [... conds]` | `[[plugins.params.scopes.attributes]]` `key = "..."`, `source = "$arg.<name>" \| literal`, `condition = "..."` | See Section 6 on value-source syntax. |
| `event <name> <key> <sample> ...` | `[[plugins.params.scopes.events]]` `name = "..."`, `attributes = [{ key = "...", source = "..." }]` | |
| `baggage <key> <sample> ...` | `[[plugins.params.scopes.baggage]]` `key = "..."`, `source = "..."` | |
| `status <code> [<sample> ...]` | `[plugins.params.scopes.status]` `code = "ok"\|"error"\|"unset"`, optional `condition = "..."` | |
| `link <span> ...` | `[[plugins.params.scopes.links]]` `span = "..."` | |
| `inject <name-prefix> [use-vars] [use-headers]` | `[plugins.params.scopes.inject]` `prefix = "..."`, `use_vars = true\|false`, `use_headers = true\|false` | |
| `extract <name-prefix> [use-vars \| use-headers]` | `[plugins.params.scopes.extract]` same shape | |
| `finish <name> ...` | `[[plugins.params.scopes.finish]]` `name = "..."` (wildcards `*`, `*req*`, `*res*` allowed as in addon) | |
| `instrument <type> <name> [aggr <a>] [desc <d>] [unit <u>] value <sample> [bounds <b>]` | `[[plugins.params.scopes.instruments]]` `kind = "cnt_int"\|"hist_int"\|"udcnt_int"\|"gauge_int"`, `name = "..."`, optional `aggr`, `description`, `unit`, `value`, `bounds = [...]` | |
| `instrument update <name> [attr <key> <sample> ...]` | `[[plugins.params.scopes.instrument_updates]]` `name = "..."`, `attributes = [...]` | |
| `log-record <severity> [id <i>] [event <n>] [span <s>] [attr <k> <s>] ... <sample> ...` | `[[plugins.params.scopes.log_records]]` `severity = "..."`, optional `id`, `event_name`, `span`, `attributes = [...]`, `body = "..."` | |

## 3. `otel-group <name>`

| Addon directive | TOML key path | Notes |
| --- | --- | --- |
| `otel-group foo` | `[[plugins.params.groups]]` `name = "foo"` | |
| `scopes <s1> <s2> ...` | `[[plugins.params.groups]]` `scopes = ["s1", "s2", ...]` | |

The addon's HAProxy-rule-side activation (`http-request otel-group <filter-id> <group>`)
maps to a SPOE message with `_event = "group:<group>"`. See
[`spoe-message-contract.md`](./spoe-message-contract.md) Section C.

## 4. SDK pipelines (addon's `.yml`)

The addon's YAML pipeline file is parsed by the OpenTelemetry C SDK.
For the plugin, each tracer / meter / logger pipeline is a TOML
array-of-tables entry.

### 4.1 Tracer pipeline

| Addon YAML path | TOML key path |
| --- | --- |
| `tracer_provider.processors[].batch.exporter.otlp.endpoint` | `[[plugins.params.tracer_pipelines]]` `exporter.endpoint = "..."` |
| `tracer_provider.processors[].batch.exporter.otlp.protocol` | `[[plugins.params.tracer_pipelines]]` `exporter.protocol = "grpc"\|"http/protobuf"\|"http/json"` |
| `tracer_provider.processors[].batch.exporter.otlp.compression` | `exporter.compression = "gzip"\|"zstd"\|"none"` |
| `tracer_provider.processors[].batch.exporter.otlp.timeout` | `exporter.timeout = "10s"` |
| `tracer_provider.processors[].batch.exporter.otlp.headers` | `exporter.headers = { "x-foo" = "bar" }` |
| `tracer_provider.processors[].batch.exporter.otlp.certificate` | `exporter.tls.ca = "/path"` |
| `tracer_provider.processors[].batch.exporter.otlp.client_certificate` | `exporter.tls.client_cert = "/path"` |
| `tracer_provider.processors[].batch.exporter.otlp.client_key` | `exporter.tls.client_key = "/path"` |
| `tracer_provider.processors[].batch.max_queue_size` | `exporter.queue.max_size = 2048` |
| `tracer_provider.processors[].batch.max_export_batch_size` | `exporter.queue.max_export_batch_size = 512` |
| `tracer_provider.processors[].batch.scheduled_delay` | `exporter.queue.scheduled_delay = "5s"` |
| `tracer_provider.sampler.always_on` | `sampler.kind = "always_on"` |
| `tracer_provider.sampler.always_off` | `sampler.kind = "always_off"` |
| `tracer_provider.sampler.trace_id_ratio_based.ratio` | `sampler.kind = "trace_id_ratio"`, `sampler.ratio = 0.1` |
| `tracer_provider.sampler.parent_based.root` | `sampler.kind = "parent_based"`, `sampler.root = { kind = "trace_id_ratio", ratio = 1.0 }` |
| `resource.attributes[]` | `resource = { "service.name" = "..." }` |

### 4.2 Meter pipeline

| Addon YAML path | TOML key path |
| --- | --- |
| `meter_provider.readers[].periodic.exporter.otlp.*` | `[[plugins.params.meter_pipelines]]` `exporter.*` (same shape as tracer) |
| `meter_provider.readers[].periodic.interval` | `reader.interval = "60s"` |
| `meter_provider.readers[].periodic.timeout` | `reader.timeout = "10s"` |
| `meter_provider.views[].name + aggregation` | `[[plugins.params.meter_pipelines.aggregations]]` `instrument_name = "..."`, `aggregation = "sum"\|"last_value"\|"histogram"\|"exp_histogram"\|"drop"\|"default"` |

### 4.3 Logger pipeline

| Addon YAML path | TOML key path |
| --- | --- |
| `logger_provider.processors[].batch.exporter.otlp.*` | `[[plugins.params.logger_pipelines]]` `exporter.*` |

## 5. Resource attributes

| Addon | TOML |
| --- | --- |
| `resource.attributes` (YAML map) | per-pipeline `resource = { ... }` table; OR `[plugins.params.resource]` for plugin-wide defaults that apply to every pipeline that does not override. |

## 6. Sample-expression value sources

The addon evaluates HAProxy sample expressions inline (`attribute key fetch_name(args)`).
The plugin receives values via SPOE arguments. Mapping:

| Addon sample expression form | TOML value-source form | Operator action |
| --- | --- | --- |
| `<sample-fetch-name>` | `"$arg.<name>"` | Operator MUST add `<name>=<sample-fetch>` to the SPOE message `args`. |
| `var(<scope>.<name>)` | `"$arg.<arg-name>"` (after operator passes the variable as a SPOE arg) | Same: round-trip via SPOE arg. |
| `str("literal")` | `"literal"` (TOML literal) | Direct. |
| `int(42)` | `42` (TOML int) | Direct. |
| `bool(true)` | `true` (TOML bool) | Direct. |

A future v2 may add `"$var.<scope>.<name>"` to read HAProxy variables
that the SPOE peer carries automatically; v1 requires the operator to
pass them as args.

## 7. Out-of-scope addon features

The following addon features are **not** representable in v1 TOML and
cause `init()` to fail with a clear error pointing to this document:

- HAProxy lifecycle events that cannot be fired as SPOE messages (e.g.
  `on-stream-start`, `on-stream-stop`, `on-idle-timeout`,
  `on-client-session-start`, `on-server-session-start`, etc.). The
  plugin documents the supported subset in the quickstart.
- Observable (asynchronous) instruments — same limitation as the addon.
- Double-precision metric values — same limitation as the addon
  (HAProxy fetches return integers).

## 8. Migration notes

A migration tool (out of scope for this feature, planned post-v1) will
mechanically translate between an addon-format `.cfg + .yml` pair and
the TOML format described above using exclusively the rules in this
document. The plugin and the tool share the same canonical mapping;
the plugin's error messages name both the addon directive and the
TOML key path so an operator can hand-translate without the tool.
