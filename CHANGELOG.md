# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Development and releases moved to GitHub (`https://github.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel`); GitLab is a read-only mirror.

## [0.5.0] - 2026-05-24

### Added

- **`drain()` override** for the v3 plugin-API drain phase introduced in `haproxy-spoa-hub` v0.7.0 (`haproxy-spoa-hub#3`). Delegates to the existing `PipelineSet::drain` primitive, which calls `shutdown()` on every loaded tracer / meter / logger — same SDK shutdown calls the in-plugin reload path already used when `init()` runs a config-swap on the same instance. Same OTLP / OTel-SDK queues get flushed. Before this release, hub reloads dropped the plugin-owned tokio runtime mid-flight (`runtime.Drop`), so any queued spans / metric data points / log records that hadn't been exported yet were lost. With v0.5.0 on a v0.7.0+ hub the SDK pipelines get a chance to flush before the runtime is torn down.

### Changed

- Plugin-API dep `^0.6` → `^0.7` to pick up the v3 vtable. Vtable evolution is append-only — this plugin's v0.5.0 `.so` still loads on v0.6.x and v0.5.x hubs (those hubs ignore the appended `drain` field, no behavioural change for them — the runtime-drop-mid-flight bug remains on those hubs, which is their existing behaviour, not a regression introduced here).

### Known limitations

- The drain body delegates to `PipelineSet::drain`, which currently does NOT enforce the `timeout_ms` argument itself — the underlying OpenTelemetry SDK shutdown calls have their own internal timeouts via the `BatchSpanProcessorBuilder` / `PeriodicReaderBuilder` configs. If those run over the hub's budget, the hub measures real-time against the deadline and increments `spoa_hub_config_reload_drain_forced_total{plugin="otel"}`, but the plugin itself never returns `false` from its drain body. Today's behaviour is strictly better than pre-v0.7 (full loss); follow-up work to wrap the SDK shutdown calls in `tokio::time::timeout` can make drain reject early and surface forced-shutdown.

### Notes for operators

- On hubs older than v0.7.0: this release behaves identically to v0.4.0 — drain is never called. No reason to upgrade unless you're also upgrading the hub.
- On hub v0.7.0+: in-flight OTLP exports get a chance to land before the plugin is torn down on reload.

## [0.4.0] - 2026-05-15

### Added

- **Prometheus metrics** via the `haproxy-spoa-hub` v0.6.0 plugin-API v2 vtable. Surfaced under the `plugin_otel_` prefix on the hub's `/metrics`.

  | metric                     | kind      | labels              |
  |----------------------------|-----------|---------------------|
  | `dispatches_total`         | counter   | `outcome`           |
  | `dispatch_duration_seconds`| histogram | _(no labels)_       |

  `outcome` ∈ `{processed, uninitialised}`. The plugin itself never errors out of `dispatch` — the OTLP exporter's own retry/drop accounting is surfaced by the `opentelemetry-sdk` crate's internal counters; this plugin's metrics cover the SPOE-hop hot path only.

### Changed

- Plugin-API dep `^0.5` → `^0.6` to pick up `MetricRecorder` and the `metrics_static` macro arm. Vtable change is append-only — this plugin's v0.4.0 .so still loads on v0.5.x hubs (metrics silent there).

## [0.3.0] - 2026-05-04

### Changed

- **BREAKING**: Bumped `haproxy-spoa-hub-plugin-api` dependency to `0.5` (the hand-rolled `#[repr(C)]` vtable migration). This plugin's `.so` will only load on `haproxy-spoa-hub` v0.5.0 or newer. Older hubs reject the load with a "missing required symbol 'get_plugin_vtable'" error since the entry-symbol naming changed.
- Dropped the direct `abi_stable = "0.11"` dependency. Plugin-api re-exports the abi_stable types this crate uses (`RHashMap`, `RString`, `RVec`); no direct dep is required. Updated all `use abi_stable::std_types::*` imports across `src/` and `tests/` to pull these types from `haproxy_spoa_hub_plugin_api` instead.

## [0.2.0] - 2026-05-03

### Added

- Per-message override of `Scope.spans[].root` and the configured sampler
  decision via two new request-side reserved SPOE arguments. `_root`
  takes `"true"` or `"false"` and overrides the per-scope `root` setting
  for one transaction; `_sample` takes `"default"`, `"record"`, or
  `"drop"` and short-circuits the sampler. Malformed values fail open
  and are counted in a new `malformed_override_total` gauge labelled by
  `(arg_name, scope_name)`. Spec at `specs/002-trust-incoming-span/`.

## [0.1.1] - 2026-04-28

### Added

- Release artifacts (per-arch plugin `.so` files) published to the GitLab
  Generic Package Registry under `packages/generic/otel/<tag>/` with stable
  URLs.
- Per-file `<artifact>.sha256` sidecars and a combined `SHA256SUMS` for
  batch verification with `sha256sum -c SHA256SUMS`.
- Cosign keyless signatures (`<artifact>.cosign.bundle`) for every release
  artifact, signed via the GitLab OIDC id token (Sigstore Fulcio + Rekor).
- `make verify-release VERSION=vX.Y.Z` target plus `scripts/verify-release.sh`
  for end-to-end consumer verification (downloads `SHA256SUMS`, checks all
  hashes, runs `cosign verify-blob` against the project's tag identity).
- Unsigned SLSA v0.2 provenance metadata emitted next to build artifacts
  via `RUNNER_GENERATE_ARTIFACTS_METADATA` (free SLSA-L1).

### Changed

- Release builds now use `cargo auditable build --release`, embedding a
  compressed `Cargo.lock`-equivalent in the plugin so SBOM tools can
  enumerate Rust dependencies without source access.

## [0.1.0] - 2026-04-27

### Added

- Initial implementation of the OpenTelemetry plugin for `haproxy-spoa-hub`:
  distributed tracing, metrics, and logs over OTLP/gRPC, with W3C
  TraceContext + Baggage propagation across SPOE messages.
- Multi-arch release artifacts: `amd64`, `arm64`, `armv7` (glibc 2.36).

[Unreleased]: https://github.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/releases/tag/v0.1.0
