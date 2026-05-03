# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/-/compare/v0.1.0...HEAD
[0.1.0]: https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/-/tags/v0.1.0
