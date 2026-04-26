#!/usr/bin/env bash
# E2E: send an HTTP request through HAProxy → SPOA hub → OTEL plugin →
# whoami backend → OTel collector. Verify:
#   1. The plugin emits a correlation handle (X-Correlation response header)
#   2. The plugin sets an outbound `traceparent` on the backend request
#      (whoami echoes received headers in its response body)
#   3. A span lands at the collector with the configured attributes

source "$(dirname "$0")/_lib.sh"

PROJECT="otel-quick-$$"
export HAPROXY_PORT=$(shuf -i 30000-50000 -n 1)
export PROJECT

trap e2e_down EXIT
e2e_up

echo "Sending HTTP request..."
TMP=$(mktemp -d)
curl -sf --max-time 5 -D "$TMP/headers" -o "$TMP/body" \
    "http://$DOCKER_HOST_ADDR:$HAPROXY_PORT/hello?x=1"

echo "Response headers:"
sed 's/^/  /' < "$TMP/headers"
echo "Response body:"
sed 's/^/  /' < "$TMP/body"

CORRELATION=$(awk -F': *' '/^[Xx]-[Cc]orrelation:/{print $2}' "$TMP/headers" | tr -d '\r')
if [ -z "$CORRELATION" ] || [ "${#CORRELATION}" -ne 32 ]; then
    echo "FAIL: X-Correlation missing or wrong length: '$CORRELATION'"
    docker compose -p "$PROJECT" logs hub | tail -50
    exit 1
fi
if ! echo "$CORRELATION" | grep -qE '^[0-9a-f]{32}$'; then
    echo "FAIL: correlation is not 32-char hex: '$CORRELATION'"
    exit 1
fi

# whoami echoes back the request's `Traceparent:` header → that's what
# the plugin emitted as `traceparent_out`.
BACKEND_TP=$(awk -F': *' 'tolower($1)=="traceparent"{print $2}' "$TMP/body" | tr -d '\r')
if [ -z "$BACKEND_TP" ]; then
    echo "FAIL: backend request lacked traceparent header — plugin did not inject"
    docker compose -p "$PROJECT" logs hub | tail -50
    exit 1
fi
if ! echo "$BACKEND_TP" | grep -qE '^00-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$'; then
    echo "FAIL: backend traceparent has wrong shape: '$BACKEND_TP'"
    exit 1
fi

echo "Waiting for span to reach the collector..."
if ! e2e_wait_for_telemetry '.resourceSpans[]?.scopeSpans[]?.spans[]?' 30; then
    echo "FAIL: no spans landed at the collector"
    docker compose -p "$PROJECT" logs otel-collector | tail -30
    docker compose -p "$PROJECT" logs hub | tail -50
    exit 1
fi

# Find the span whose span_id matches the new HAProxy span (the second
# segment of the outbound traceparent).
NEW_SPAN_ID=$(echo "$BACKEND_TP" | cut -d- -f3)
SPAN=$(e2e_telemetry \
    | jq -c --arg sid "$NEW_SPAN_ID" \
        '.resourceSpans[]?.scopeSpans[]?.spans[]? | select(.spanId == $sid)' \
    | head -1)

if [ -z "$SPAN" ]; then
    echo "FAIL: no exported span with spanId $NEW_SPAN_ID"
    e2e_telemetry | jq '.resourceSpans[]?.scopeSpans[]?.spans[]? | {name, traceId, spanId}' | head -20
    exit 1
fi

NAME=$(echo "$SPAN" | jq -r '.name')
METHOD=$(echo "$SPAN" | jq -r '.attributes[]? | select(.key=="http.request.method").value.stringValue // empty')
PATH_ATTR=$(echo "$SPAN" | jq -r '.attributes[]? | select(.key=="url.path").value.stringValue // empty')

[ "$NAME" = "haproxy.request" ] || { echo "FAIL: span name '$NAME' != haproxy.request"; exit 1; }
[ "$METHOD" = "GET" ] || { echo "FAIL: http.request.method '$METHOD' != GET"; exit 1; }
[ "$PATH_ATTR" = "/hello" ] || { echo "FAIL: url.path '$PATH_ATTR' != /hello"; exit 1; }

echo "PASS: E2E quickstart"
echo "  correlation:           $CORRELATION"
echo "  outbound traceparent:  $BACKEND_TP"
echo "  exported span:         name=$NAME method=$METHOD path=$PATH_ATTR"
