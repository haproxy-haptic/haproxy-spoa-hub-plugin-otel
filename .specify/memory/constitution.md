<!--
SYNC IMPACT REPORT
==================
Version change: (template, unratified) → 1.0.0
Bump rationale: Initial ratification — first concrete constitution replacing the
unfilled template. Treated as a MAJOR baseline (1.0.0) since prior file contained
only placeholders and provided no governance contract.

Modified principles:
  - [PRINCIPLE_1_NAME] → I. Code Quality (NON-NEGOTIABLE)
  - [PRINCIPLE_2_NAME] → II. Testing Standards (NON-NEGOTIABLE)
  - [PRINCIPLE_3_NAME] → III. User Experience Consistency
  - [PRINCIPLE_4_NAME] → IV. Performance Requirements
  - (PRINCIPLE_5 removed — user requested four focus areas)

Added sections:
  - Performance Standards & Reliability (replaces [SECTION_2_*])
  - Development Workflow & Quality Gates (replaces [SECTION_3_*])
  - Governance (filled in)

Removed sections:
  - 5th Core Principle slot (intentionally dropped per user input)

Templates requiring updates:
  - ✅ .specify/templates/plan-template.md — Constitution Check section verified;
       gates align with the four principles via the existing placeholder block.
  - ✅ .specify/templates/spec-template.md — Mandatory sections (User Scenarios,
       Requirements, Success Criteria) consistent with Principle II/III/IV.
  - ✅ .specify/templates/tasks-template.md — Phase structure (Setup, Foundational,
       per-story, Polish) compatible with TDD and performance-validation gates.
  - ✅ .specify/templates/checklist-template.md — Generic, no constitution-bound
       references; no edit required.
  - ✅ .specify/skills / command files — generic guidance, no agent-specific
       references requiring removal.

Follow-up TODOs: none. RATIFICATION_DATE set to today (2026-04-25) since the
project has no prior ratified constitution on record.
-->

# HAProxy SPOA Hub OTEL Plugin Constitution

## Core Principles

### I. Code Quality (NON-NEGOTIABLE)

All code merged to the main branch MUST satisfy the following:

- Static analysis (linter + type checker where the language supports it) MUST pass
  with zero warnings; warnings are not deferred via inline suppressions without an
  accompanying comment that names the specific defect being suppressed.
- Public APIs (exported functions, configuration keys, metric/trace attribute
  names) MUST have a documented contract: input/output shape, error modes, and
  units for any numeric value.
- No commented-out code, dead branches, or unreferenced symbols MAY be merged.
  Removed functionality MUST be deleted, not parked.
- Duplication MUST be removed at the root: when the same logic appears in two
  places, it is extracted into a shared function or module rather than copied.
- Cyclomatic complexity, file length, and function length thresholds enforced by
  the linter MAY NOT be raised to accommodate a specific change; the change is
  refactored instead.

**Rationale**: The plugin runs in HAProxy's hot path. Subtle bugs caused by dead
code, unchecked types, or copy-paste drift translate directly into production
incidents on traffic-handling infrastructure. Quality gates are cheaper than
post-hoc forensics.

### II. Testing Standards (NON-NEGOTIABLE)

Testing discipline is enforced as follows:

- Test-Driven Development is mandatory for bugs and SHOULD be applied to features:
  write a failing test, observe it fail for the right reason, then implement.
- Every functional requirement (FR-*) in a feature spec MUST map to at least one
  automated test before the feature is marked complete.
- Each user story in `spec.md` MUST be covered by an integration test that
  exercises the story end-to-end and is runnable independently of other stories.
- Tests MUST be deterministic. Flaky tests are treated as failing tests and
  blocked from merging until the underlying cause is fixed; retries do not count
  as a fix.
- External dependencies (HAProxy SPOE peer, OTEL collector, network, clock) MUST
  be controllable by the test (real instance, fake, or recorded fixture) — never
  reached over the public internet from a test run.
- A test that passes when you expected it to fail is a defect in the test or the
  assumption, not a green light. Investigate before writing implementation code.

**Rationale**: The plugin sits between HAProxy and observability backends; bugs
are invisible until traffic is replayed against them. A reproducible, hermetic
test suite is the only safety net before production.

### III. User Experience Consistency

The "user" of this plugin is the operator who configures it and the developer
who consumes its telemetry. Their experience MUST be predictable across the
codebase:

- Configuration keys MUST follow a single, documented naming convention (case,
  separators, prefixes). New options MUST match neighbouring options.
- Error messages MUST identify (a) what the plugin was trying to do, (b) which
  configuration value or input caused the failure, and (c) how the operator can
  resolve it. Stack traces alone do not satisfy (c).
- Log levels MUST be used consistently: `error` for operator-actionable failures,
  `warn` for degraded but recoverable states, `info` for lifecycle events,
  `debug` for diagnostic detail. Spam at higher levels is a defect.
- OpenTelemetry attribute names, metric names, and span names MUST follow the
  OTEL semantic conventions where one exists; deviations MUST be documented in
  the spec for the change introducing them.
- CLI flags, environment variables, and configuration file keys MUST stay in
  sync: a setting expressible in one MUST be expressible in the others using
  predictable transformations (e.g. `--foo-bar` ↔ `FOO_BAR` ↔ `foo.bar`).
- Breaking changes to any of the above are governed by Principle II of the
  Governance section (versioning) and require a migration note.

**Rationale**: Operators integrate the plugin once and live with its surface for
years. Inconsistency in config keys, log levels, or attribute names taxes every
operator who touches the system and erodes trust faster than missing features.

### IV. Performance Requirements

Performance is a first-class requirement, not a polish item:

- Every feature plan MUST declare its performance budget in `plan.md` under
  `Performance Goals` and `Constraints`. Examples: per-request CPU, per-request
  allocation count, p95/p99 added latency, peak memory, max in-flight spans.
- Changes that may affect the data path MUST include a benchmark or load-test
  result in the PR description, comparing before/after on the budgeted metrics.
  Regressions beyond the declared tolerance MUST be justified or rejected.
- Allocations on the hot path SHOULD be bounded and pooled; unbounded buffering,
  unbounded goroutine/thread fan-out, and unbounded retry queues are forbidden
  without an explicit, documented backpressure strategy.
- Telemetry MUST NOT block traffic. Export, batching, and flush operations MUST
  be asynchronous with respect to HAProxy's request loop, and MUST degrade by
  shedding telemetry rather than by stalling traffic.
- Resource caps (queue sizes, worker counts, timeouts) MUST be configurable and
  MUST have safe defaults; "unlimited" is never a default.

**Rationale**: A telemetry plugin that adds tail latency or stalls under load
makes the system it observes worse, not better. Budgets and benchmarks make the
tradeoffs explicit instead of discovering them under incident.

## Performance Standards & Reliability

The following project-wide standards apply in addition to per-feature budgets:

- **Added latency**: The plugin MUST NOT add more than the p95 latency budget
  declared in the active feature plan; in absence of a feature-specific budget,
  the default ceiling is 1 ms p95 and 5 ms p99 of added per-request overhead at
  the configured target throughput.
- **Memory**: Steady-state RSS MUST be bounded and reported in benchmark output.
  Memory leaks (unbounded growth under steady load) are release blockers.
- **Failure modes**: Loss of the OTEL collector, DNS failure, or backpressure
  from the exporter MUST NOT cause request failures in HAProxy. The plugin MUST
  fail open with respect to traffic and surface the degradation via metrics and
  logs.
- **Observability of self**: The plugin MUST expose its own health (queue
  depths, drop counts, export errors) so operators can detect that it is the
  cause of an incident without reading its source.

## Development Workflow & Quality Gates

- **Spec-driven flow**: Non-trivial work follows the Spec Kit workflow —
  `/speckit-specify` → `/speckit-clarify` (when needed) → `/speckit-plan` →
  `/speckit-tasks` → `/speckit-implement`. Skipping steps requires written
  justification in the PR description.
- **Constitution Check gate**: Every `plan.md` MUST pass the Constitution Check
  before Phase 0 research and re-pass after Phase 1 design. Violations MUST be
  recorded in the plan's Complexity Tracking table with the rejected simpler
  alternative.
- **Pre-merge gate**: A change MAY be merged only when (a) all automated tests
  and linters pass, (b) for hot-path changes, benchmarks have been run and
  attached, (c) any new public surface is documented, and (d) the change has a
  reviewing approver who is not the author.
- **Dead code & duplication sweep**: Authors MUST verify, before requesting
  review, that the change has not orphaned code or duplicated existing logic
  elsewhere in the tree. Reviewers MUST flag violations.
- **Commit hygiene**: Commits MUST be focused and describe *why*; mechanical
  reformatting and behaviour changes MUST be in separate commits to keep review
  diffs readable.

## Governance

- This constitution supersedes ad-hoc conventions and reviewer preferences.
  Where it conflicts with a personal style guide, the constitution wins.
- **Amendment procedure**: Changes are proposed via a PR that updates
  `.specify/memory/constitution.md`, includes a Sync Impact Report comment at
  the top of the file, and updates any dependent templates in the same PR.
- **Versioning policy** (semantic versioning of the constitution itself):
  - **MAJOR**: A principle is removed, redefined in a backward-incompatible
    way, or governance authority is materially changed.
  - **MINOR**: A new principle or normative section is added, or an existing
    principle is materially expanded.
  - **PATCH**: Wording clarifications, typo fixes, or non-semantic
    refinements that do not change behaviour expected from contributors.
- **Compliance review**: Every PR MUST verify that it complies with the
  constitution. Reviewers SHOULD reject PRs that violate principles without
  using the Complexity Tracking escape hatch in `plan.md`.
- **Runtime guidance**: Day-to-day technical context for contributors lives in
  the project's `CLAUDE.md` and active `specs/<feature>/plan.md` files; this
  constitution defines the durable rules those documents operate within.

**Version**: 1.0.0 | **Ratified**: 2026-04-25 | **Last Amended**: 2026-04-25
