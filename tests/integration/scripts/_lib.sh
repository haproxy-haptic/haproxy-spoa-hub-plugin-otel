# Shared helpers for the e2e test scripts. Source from each test.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

DOCKER_HOST_ADDR=$(echo "${DOCKER_HOST:-}" | sed -n 's|tcp://\([^:]*\):.*|\1|p')
DOCKER_HOST_ADDR=${DOCKER_HOST_ADDR:-localhost}

# Bring services up. PROJECT must be unique per test invocation so
# parallel runs don't collide.
e2e_up() {
    : "${PROJECT:?PROJECT must be set}"
    : "${HAPROXY_PORT:?HAPROXY_PORT must be set}"
    docker compose -p "$PROJECT" -f "$COMPOSE_DIR/docker-compose.yml" up -d

    echo "Waiting for HAProxy on port $HAPROXY_PORT..."
    for i in $(seq 1 60); do
        if curl -sf --max-time 2 "http://$DOCKER_HOST_ADDR:$HAPROXY_PORT/" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    echo "FAIL: HAProxy did not become ready"
    docker compose -p "$PROJECT" -f "$COMPOSE_DIR/docker-compose.yml" logs
    return 1
}

e2e_down() {
    docker compose -p "${PROJECT:-}" -f "$COMPOSE_DIR/docker-compose.yml" down -v 2>/dev/null || true
}

# Read the OTel collector's exported telemetry file via a one-shot
# busybox sidecar that mounts the same named volume the collector
# writes into. Each line is one OTLP
# ExportTraceServiceRequest / ExportMetricsServiceRequest /
# ExportLogsServiceRequest as JSON.
e2e_telemetry() {
    local volume="${PROJECT}_otel-data"
    docker run --rm -v "${volume}:/data:ro" busybox:1.37 \
        sh -c 'cat /data/telemetry.jsonl 2>/dev/null || true' 2>/dev/null
}

# Wait until at least one record matching `jq_filter` (e.g. `.resourceSpans`)
# is present in the telemetry file. Times out after $2 seconds (default 15).
# Parse the `Traceparent:` header out of a whoami response body.
# whoami returns either plain text (`Header: value` lines) or JSON
# (`{"headers":{"Traceparent":["..."]}}`) depending on path / accept;
# this helper handles both shapes.
e2e_extract_traceparent() {
    local body_file="$1"
    if jq -e . "$body_file" >/dev/null 2>&1; then
        jq -r '.headers.Traceparent[0] // .headers.traceparent[0] // empty' "$body_file"
    else
        awk -F': *' 'tolower($1)=="traceparent"{print $2; exit}' "$body_file" | tr -d '\r'
    fi
}

e2e_wait_for_telemetry() {
    local jq_filter="${1:-.resourceSpans}"
    local timeout="${2:-15}"
    for i in $(seq 1 "$timeout"); do
        local count
        count=$(e2e_telemetry \
            | jq -s "[.[] | $jq_filter] | flatten | length" 2>/dev/null \
            || echo 0)
        if [ "${count:-0}" -gt 0 ]; then
            return 0
        fi
        sleep 1
    done
    return 1
}
