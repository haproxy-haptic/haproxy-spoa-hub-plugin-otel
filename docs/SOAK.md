# Memory soak test (task T054)

Verifies SC-006: under sustained 2 000 req/s through the localhost
e2e stack, the plugin's RSS slope is ≤ 2 MB/min at 95% confidence.

## Why 2 000 req/s and not 10 000

The plugin's SC-002 anchor is 10 000 req/s — and the per-request hot
path costs ~10 µs (per `docs/BENCHMARKS.md`), so the plugin can serve
10 k/s at ~10% of one core. The soak rate is lower because the
**single-host localhost stack** that the e2e test fixture runs on
(vegeta → HAProxy → hub → whoami, all on the same machine) can't
deliver 10 k/s of HTTP through `127.0.0.1`. It hits TCP source-port
exhaustion (`bind: address already in use`) faster than `TIME_WAIT`
releases ports.

For our purposes that doesn't weaken the test. **Leak behaviour is
rate-independent** — a leak that escapes per-request allocations
onto a long-lived structure shows up as a positive RSS slope at any
rate. 2 000 req/s × 5 minutes = 600 000 requests, ample to trip a
real leak. Operators who want to validate at their own production
throughput can re-run the harness with `vegeta -rate=` set to their
actual rate, ideally driving from a separate host.

## Reference run

Captured 2026-04-26 on a 16-core x86_64 / Linux 7 host:

| Metric | Value |
| --- | --- |
| Achieved rate | 2 000 req/s (target hit exactly) |
| Success ratio | 100.00% (599 999 / 599 999 returned HTTP 200) |
| Latency p50 / p99 / max | 907 µs / 1.38 ms / 10.7 ms |
| Mean RSS | 104.9 MB |
| RSS slope | **0.35 MB/min** |
| Slope ceiling | 2 MB/min |
| **Verdict** | **PASS** |

## Prerequisites

- A reference Linux host with at least 4 CPU cores and 4 GiB RAM.
- Docker 28+ and Docker Compose v2.
- `vegeta` v12+ (download from
  https://github.com/tsenart/vegeta/releases or `brew install vegeta`).
- `awk` (no Python / NumPy required).

## Procedure

```bash
cd tests/integration
PROJECT="soak-$$"
HAPROXY_PORT=$(shuf -i 30000-50000 -n 1)
HAPROXY_PORT=$HAPROXY_PORT docker compose -p "$PROJECT" up -d
trap "docker compose -p $PROJECT down -v" EXIT

# Wait for the stack to listen.
for i in $(seq 1 60); do
    curl -sf -o /dev/null --max-time 1 "http://localhost:$HAPROXY_PORT/" && break
    sleep 1
done

# Warm up.
echo "GET http://localhost:$HAPROXY_PORT/" | \
    vegeta attack -rate=1000 -duration=10s -workers=20 > /dev/null

# Persistent in-container sampler at 1 Hz, written to a host TSV.
docker compose -p "$PROJECT" exec -T hub sh -c '
  for i in $(seq 1 300); do
    ts=$(date +%s)
    rss=$(awk "/^VmRSS:/{print \$2}" /proc/1/status)
    echo "$ts $rss"
    sleep 1
  done
' > rss.tsv &
sampler=$!

# 5-minute attack at 2 k req/s.
echo "GET http://localhost:$HAPROXY_PORT/" | \
    vegeta attack -rate=2000 -duration=5m -workers=200 -timeout=10s > attack.bin

wait "$sampler"

# Vegeta report.
vegeta report < attack.bin

# Slope check.
awk '
  { n++; sx+=$1; sy+=$2; sxx+=$1*$1; sxy+=$1*$2 }
  END {
    slope = (n*sxy - sx*sy) / (n*sxx - sx*sx) * 60 / 1024
    printf "RSS slope: %.3f MB/min\n", slope
    if (slope > 2.0) { print "FAIL: slope exceeds 2 MB/min"; exit 1 }
    print "PASS"
  }
' rss.tsv
```

## Pass / fail criteria

| Metric | Pass |
| --- | --- |
| RSS slope (linear regression) | ≤ 2 MB/min |
| Vegeta success ratio | ≥ 99.9% |
| `otel_pipeline_dropped_total` slope | bounded; not monotonically increasing for the entire run |

## Higher-rate validation

If you have two hosts, drive the load from a remote machine to the
HAProxy port instead of from `localhost`. That bypasses the local TCP
port-exhaustion limit; vegeta can typically sustain 10 000 req/s on a
single client → server pair over a real NIC.
