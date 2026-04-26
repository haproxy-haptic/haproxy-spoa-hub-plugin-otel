.PHONY: build test test-quickstart test-traceparent test-metrics clean logs

# All targets require the docker group. Most desktop installs put the
# user in `docker` after `usermod -aG docker $USER` but the existing
# shell session still lacks the group. If `make test` fails with
# "permission denied while trying to connect to the Docker daemon", run
# the make invocation under `sg docker -c '...'` or open a new shell.

build:
	docker compose build

test: test-quickstart test-traceparent test-metrics
	@echo "All E2E tests passed."

test-quickstart: build
	@echo "=== E2E: Quickstart (root span, attributes) ==="
	@bash scripts/test_quickstart.sh

test-traceparent: build
	@echo "=== E2E: traceparent round-trip ==="
	@bash scripts/test_traceparent.sh

test-metrics: build
	@echo "=== E2E: metric counter ==="
	@bash scripts/test_metrics.sh

logs:
	docker compose logs

clean:
	docker compose down -v --remove-orphans 2>/dev/null || true
	docker image prune -f 2>/dev/null || true
