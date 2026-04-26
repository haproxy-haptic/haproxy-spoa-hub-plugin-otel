# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial implementation of the OpenTelemetry plugin for `haproxy-spoa-hub`:
  distributed tracing, metrics, and logs over OTLP/gRPC, with W3C
  TraceContext + Baggage propagation across SPOE messages.
- Multi-arch release artifacts: `amd64`, `arm64`, `armv7` (glibc 2.36).

[Unreleased]: https://gitlab.com/haproxy-haptic/haproxy-spoa-hub-plugin-otel/-/compare/v0.0.0...HEAD
