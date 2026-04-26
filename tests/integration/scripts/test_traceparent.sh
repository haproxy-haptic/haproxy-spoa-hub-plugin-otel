#!/usr/bin/env bash
# E2E: send a request with a W3C `traceparent` header. Verify the
# plugin produces a span sharing the trace_id and pointing at the
# inbound span_id as parent, and that the outbound `traceparent` on
# the backend request preserves the trace_id.

source "$(dirname "$0")/_lib.sh"

PROJECT="otel-tp-$$"
export HAPROXY_PORT=$(shuf -i 30000-50000 -n 1)
export PROJECT

trap e2e_down EXIT
e2e_up

PARENT_TRACE_ID="0af7651916cd43dd8448eb211c80319c"
PARENT_SPAN_ID="b7ad6b7169203331"
TRACEPARENT="00-${PARENT_TRACE_ID}-${PARENT_SPAN_ID}-01"

echo "Sending request with traceparent: $TRACEPARENT"
TMP=$(mktemp -d)
curl -sf --max-time 5 -D "$TMP/headers" -o "$TMP/body" \
    -H "traceparent: $TRACEPARENT" \
    "http://$DOCKER_HOST_ADDR:$HAPROXY_PORT/api"

OUTBOUND=$(e2e_extract_traceparent "$TMP/body")
[ -n "$OUTBOUND" ] || { echo "FAIL: outbound traceparent missing"; cat "$TMP/body"; exit 1; }

OUTBOUND_TRACE_ID=$(echo "$OUTBOUND" | cut -d- -f2)
[ "$OUTBOUND_TRACE_ID" = "$PARENT_TRACE_ID" ] \
    || { echo "FAIL: outbound trace_id $OUTBOUND_TRACE_ID != $PARENT_TRACE_ID"; exit 1; }

OUTBOUND_SPAN_ID=$(echo "$OUTBOUND" | cut -d- -f3)
[ "$OUTBOUND_SPAN_ID" != "$PARENT_SPAN_ID" ] \
    || { echo "FAIL: outbound span_id should differ from inbound"; exit 1; }

echo "Waiting for span to reach the collector..."
e2e_wait_for_telemetry '.resourceSpans[]?.scopeSpans[]?.spans[]?' 30 \
    || { echo "FAIL: no spans"; docker compose -p "$PROJECT" logs hub | tail -50; exit 1; }

SPAN=$(e2e_telemetry \
    | jq -c --arg tid "$PARENT_TRACE_ID" \
        '.resourceSpans[]?.scopeSpans[]?.spans[]? | select(.traceId == $tid)' \
    | head -1)

[ -n "$SPAN" ] || { echo "FAIL: no span with trace_id $PARENT_TRACE_ID"; e2e_telemetry | head -20; exit 1; }

PARENT=$(echo "$SPAN" | jq -r '.parentSpanId')
[ "$PARENT" = "$PARENT_SPAN_ID" ] \
    || { echo "FAIL: span.parentSpanId '$PARENT' != '$PARENT_SPAN_ID'"; exit 1; }

NEW_SPAN_ID=$(echo "$SPAN" | jq -r '.spanId')
[ "$NEW_SPAN_ID" = "$OUTBOUND_SPAN_ID" ] \
    || { echo "FAIL: span.spanId '$NEW_SPAN_ID' != outbound span_id '$OUTBOUND_SPAN_ID'"; exit 1; }

echo "PASS: E2E traceparent round-trip"
echo "  inbound  trace_id: $PARENT_TRACE_ID"
echo "  inbound  span_id:  $PARENT_SPAN_ID"
echo "  outbound traceparent: $OUTBOUND"
echo "  recorded span:    trace=$(echo "$SPAN" | jq -r '.traceId')  parent=$PARENT  span=$NEW_SPAN_ID"
