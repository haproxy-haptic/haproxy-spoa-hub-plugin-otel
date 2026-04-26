#!/usr/bin/env bash
# E2E: drive 5 requests through HAProxy and verify the
# `http.server.request_count` counter reaches the collector with sum >= 5.

source "$(dirname "$0")/_lib.sh"

PROJECT="otel-metrics-$$"
export HAPROXY_PORT=$(shuf -i 30000-50000 -n 1)
export PROJECT

trap e2e_down EXIT
e2e_up

echo "Sending 5 requests..."
for i in 1 2 3 4 5; do
    curl -sf --max-time 5 "http://$DOCKER_HOST_ADDR:$HAPROXY_PORT/path-$i" >/dev/null
done

echo "Waiting for metric to reach the collector..."
for i in $(seq 1 30); do
    SUM=$(e2e_telemetry \
        | jq -s '
            [
              .[]?.resourceMetrics[]?.scopeMetrics[]?.metrics[]?
              | select(.name == "http.server.request_count")
              | .sum.dataPoints[]?.asInt | tonumber
            ]
            | max // 0
        ' 2>/dev/null || echo 0)
    if [ "${SUM:-0}" -ge 5 ]; then
        echo "PASS: E2E metrics — counter sum reached $SUM"
        exit 0
    fi
    sleep 1
done

echo "FAIL: counter http.server.request_count did not reach 5"
echo "Telemetry tail:"
e2e_telemetry | tail -5
docker compose -p "$PROJECT" logs otel-collector | tail -30
exit 1
