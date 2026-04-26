# Hot-path benchmarks

Measures the plugin's per-message dispatch overhead, anchoring SC-002
(≤ 1 ms added p95, ≤ 5 ms added p99 at 10 000 req/s sustained per hub
instance).

The `hot_path` benchmark is in `benches/hot_path.rs` and is run with:

```bash
cargo bench --bench hot_path
```

Criterion writes detailed reports to `target/criterion/`.

## Reference run

Captured 2026-04-26 on a 16-core x86_64 / Linux 7 host, `cargo bench`
on Rust 1.95.0, release profile (LTO, codegen-units = 1).

| Benchmark | min | median | max | iterations |
| --- | --- | --- | --- | --- |
| `dispatch::request_response_round_trip` | 8.98 µs | **9.42 µs** | 9.80 µs | 540 000 |
| `dispatch::request_side_steady_state`   | 9.26 µs | **9.63 µs** | 9.96 µs | 505 000 |

Both benchmarks measure the plugin's dispatch hot path against an
unreachable OTLP endpoint (sampler `always_off`, exporter pointed at
`127.0.0.1:65535` so the network never participates). The figures are
the per-call overhead of:

- argument parsing
- correlation-handle generation (request) / lookup (response)
- W3C TraceContext extraction (request) / injection (request)
- attribute-rule evaluation
- span construction (request) / span completion (response)
- in-flight-store insert / remove

## SC-002 margin

At 10 000 req/s per hub instance, the plugin's CPU contribution at the
median measured cost is:

```
9.42 µs/req × 10 000 req/s = 94.2 ms/s ≈ 9.4% of one CPU
```

Per-request added p95 from the plugin is bounded by a ceiling roughly
two orders of magnitude below SC-002's 1 ms p95. That margin absorbs:

- Allocator jitter at higher load (the SDK's batch processor allocates
  a Vec-of-spans periodically).
- Lock contention on the correlation map under multi-shard concurrent
  inserts (DashMap shards by hash; this benchmark uses a single thread).
- HAProxy SPOE serialization and the unix-socket round-trip — which
  the operator-visible end-to-end latency includes but the plugin's
  hot-path measurement does not.

## What's NOT measured here

The benchmark covers the plugin's in-process dispatch only. It does
not cover:

- HAProxy SPOE serialization / deserialization (HAProxy side).
- unix-socket transport between HAProxy and the hub.
- the hub's own dispatch path (queueing across plugins, etc.).
- network I/O of OTLP export (the SDK's batch processor handles this
  off the hot path; export is asynchronous and bounded by the per-
  pipeline queue cap).

For sustained-load memory behaviour, see `docs/SOAK.md` (5-minute
soak, slope ≤ 2 MB/min at 95% CI). The soak runs at 2 000 req/s on a
single-host localhost stack — see SOAK.md for why that rate validates
the leak property regardless of the SC-002 throughput anchor.

## Reproducing

```bash
cargo bench --bench hot_path 2>&1 | tee benchmark-output.txt
```

CI captures these as artifacts under `target/criterion/` (one HTML
report per benchmark group). Each PR that touches the hot path is
expected to compare its run against the previous run; criterion
auto-detects regressions vs. the saved baseline.
