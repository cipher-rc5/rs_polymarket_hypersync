# Production Hardening Plan

This plan is designed as a phased execution roadmap you can hand to coding agents.
It targets moving this codebase from approximately **6.5/10** to **8.5+/10** production readiness while preserving current behavior and output style.

## Current State Snapshot

- Strengths:
  - Clear module boundaries (`main`, `contracts`, `exchange`, `enrich`, `storage`)
  - Good use of `anyhow::Result` with contextual errors
  - Security-conscious RTDS defaults (strict TLS enabled, optional cert pinning)
  - Feature flags and env-driven runtime controls
- Gaps:
  - No automated tests
  - No CI workflow
  - `cargo fmt -- --check` currently fails
  - No robust retry/reconnect/backoff strategy
  - Hot-path synchronous writes and inline enrichment can bottleneck stream throughput
  - Limited structured observability

## Non-Negotiable Constraints (for all phases)

- Use strictly idiomatic Rust.
- Target Rust edition `2024`.
- Target Rust toolchain `1.93`.
- Keep event topic/address constants in `src/contracts.rs`.
- Respect `DATA_DIR`; all persisted output must remain under it.
- Keep default runtime behavior low-noise and production-safe.
- Never hardcode secrets/tokens.
- Verify with:
  - `cargo fmt`
  - `cargo check`
  - `cargo clippy --all-targets --all-features -- -D warnings`

## Reference Specifications (must consult)

- https://docs.polymarket.com/llms.txt
- https://docs.polymarket.com/market-data/websocket/rtds.md
- https://duckdb.org/docs/stable/clients/rust
- https://docs.rs/parquet/latest/parquet
- https://github.com/denoland/fastwebsockets
- https://rust-unofficial.github.io/patterns/idioms

---

## Phase 0 - Baseline Stabilization (Quick Wins)

### Objectives

- Restore formatting compliance.
- Establish baseline documentation for hardening progress.
- Lock deterministic quality gates for local development.

### Corrections

1. Run and commit formatting updates.
2. Ensure `Cargo.toml` contains explicit `rust-version = "1.93"` (already present; verify).
3. Add `just` recipe(s) for strict local validation if missing (e.g., `just check-strict`).
4. Ensure README and AGENTS remain aligned with toolchain + references.

### Deliverables

- Clean `cargo fmt -- --check` pass.
- Updated `justfile` recipe(s) for full local checks.
- No behavior changes.

### Acceptance Criteria

- `cargo fmt -- --check` passes.
- `cargo check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.

### Agent Prompt

```text
Phase 0 task: perform baseline stabilization only. Do not change runtime behavior.
Actions:
1) Run cargo fmt and commit formatting-only changes.
2) Verify rust-version is 1.93 in Cargo.toml.
3) Add a justfile recipe named check-strict that runs fmt/check/clippy with warnings denied.
4) Keep README and AGENTS consistent with edition 2024 + rust 1.93 + required reference specs.
5) Return a concise diff summary and command outputs.
```

---

## Phase 1 - Test Foundation (Correctness First)

### Objectives

- Add high-value tests around deterministic logic.
- Protect condition matching and token tracking from regressions.

### Corrections

1. Add unit tests in `src/exchange.rs` for:
   - `normalize_topic_word`
   - `normalize_condition_id_word`
   - `extract_first_word_hex`
   - `topic_u256_to_decimal`
   - `matches_order_filled` with matching/non-matching data
   - `matches_orders_matched` with matching/non-matching data
2. Extract pure matching helpers from `src/main.rs` into testable functions (if needed), then test:
   - CTF condition matching by event type/topic index
   - NegRisk topic matching
   - Exchange token registration + filtered match decisions
3. Add failure-mode tests for malformed/short ABI data payloads.

### Deliverables

- A minimal but meaningful test suite covering core filtering and decoding logic.
- No external network dependency in tests.

### Acceptance Criteria

- `cargo test` passes reliably offline.
- Test suite validates all event matching branches.
- No flaky timing-dependent tests.

### Agent Prompt

```text
Phase 1 task: add deterministic tests for matching/decoding logic.
Requirements:
- Prioritize src/exchange.rs and extracted pure helpers from src/main.rs.
- Cover happy paths and malformed ABI payloads.
- Avoid network calls and live chain dependencies.
- Keep tests idiomatic and compact.
- Return test list + coverage rationale + cargo test output summary.
```

---

## Phase 2 - CI and Quality Gates

### Objectives

- Enforce quality gates on every change.
- Prevent regressions from bypassing local checks.

### Corrections

1. Add GitHub Actions workflow under `.github/workflows/ci.yml` with jobs for:
   - `cargo fmt -- --check`
   - `cargo check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test`
2. Optional but recommended:
   - `cargo deny` (licenses/advisories)
   - `cargo audit` (vulnerability scan)
3. Cache Rust dependencies/toolchain to reduce CI time.

### Deliverables

- CI workflow with required status checks.
- README section documenting CI expectations.

### Acceptance Criteria

- CI runs clean on a fresh clone.
- Any formatting/lint/test regression fails CI.

### Agent Prompt

```text
Phase 2 task: implement CI quality gates.
Create .github/workflows/ci.yml with fmt/check/clippy/test and stable caching.
Use rust toolchain 1.93 and edition 2024 assumptions.
If adding cargo audit/deny, make failures non-flaky and deterministic.
Return workflow YAML summary and a list of enforced checks.
```

---

## Phase 3 - Runtime Resilience and Failure Handling

### Objectives

- Avoid hard-stop behavior on transient network faults.
- Improve uptime during RTDS/HTTP/HyperSync disruptions.

### Corrections

1. Introduce retry/backoff policy (with jitter) for:
   - HyperSync stream startup and recoverable stream errors
   - HTTP enrichment requests (Gamma/CLOB)
   - RTDS connection and reconnect loops
2. Add bounded timeouts for external I/O calls.
3. Distinguish recoverable vs non-recoverable errors clearly.
4. Add graceful shutdown handling on SIGINT/SIGTERM:
   - Drain in-flight work
   - Flush and finalize storage
5. Add circuit-breaker style suppression for repeated noisy errors.

### Deliverables

- Reconnecting stream and RTDS listener.
- Configurable retry/timeouts via env vars with safe defaults.

### Acceptance Criteria

- Simulated transient failures recover without process restart.
- Finalization still occurs on shutdown path.
- Log noise remains controlled under repeated failure.

### Agent Prompt

```text
Phase 3 task: add resilience without changing matching semantics.
Implement retry/backoff+timeout policies for HyperSync, RTDS, and HTTP enrichment.
Add graceful shutdown handling and ensure EventStore finalize is called on shutdown.
Expose retry/timeout controls via env vars and document defaults.
Return a failure-matrix (error type -> action) and validation steps.
```

---

## Phase 4 - Throughput and Storage Performance

### Objectives

- Remove hot-path bottlenecks.
- Keep stream ingestion responsive under larger log volume.

### Corrections

1. Replace per-event insert with buffered/batched DuckDB writes:
   - Accumulate N events or T duration
   - Insert in transaction for each batch
2. Decouple enrichment from ingestion path:
   - Queue enrichment tasks with bounded concurrency
   - Avoid awaiting HTTP in the tight stream loop
3. Add backpressure safeguards:
   - Bounded channels
   - Explicit drop/defer policy when overloaded
4. Add cheap periodic stats printouts (configurable frequency):
   - ingest rate
   - queue depth
   - enrichment success/failure counts

### Deliverables

- Measurable throughput improvements.
- Stable memory behavior under sustained ingestion.

### Acceptance Criteria

- No unbounded queues.
- Ingestion remains smooth when enrichment endpoints are slow.
- Batch write path validates data parity vs old path.

### Agent Prompt

```text
Phase 4 task: optimize throughput with batching and decoupled enrichment.
Requirements:
- Implement bounded channel architecture.
- Batch DuckDB inserts in transactions.
- Ensure backpressure and overload behavior is explicit and documented.
- Keep printed output concise and low-noise.
Return benchmark notes (before/after qualitative or quantitative).
```

---

## Phase 5 - Observability and Operability

### Objectives

- Make runtime behavior diagnosable in production.
- Support incident response with actionable telemetry.

### Corrections

1. Move from ad-hoc `println!` to structured logging (`tracing` + subscriber).
2. Introduce log levels via env (`RUST_LOG`) and stable field names:
   - chain, block, tx, source, event_type, retry_count, endpoint
3. Add lightweight internal metrics counters and periodic summaries.
4. Standardize error messages with operation + endpoint + context.
5. Add runbook documentation for common incidents.

### Deliverables

- Structured logs suitable for shipping to log aggregation tools.
- Operator-facing troubleshooting section in README.

### Acceptance Criteria

- Key failures have searchable structured context.
- Operators can diagnose stream stalls, RTDS disconnects, and storage failures quickly.

### Agent Prompt

```text
Phase 5 task: implement structured observability.
Use tracing, include consistent structured fields, and preserve concise human-readable defaults.
Add periodic counters and operator runbook notes in README.
Do not introduce excessive log volume.
Return sample log lines and rationale for chosen fields.
```

---

## Phase 6 - Security and Configuration Hardening

### Objectives

- Reduce configuration footguns.
- Strengthen secure-by-default behavior.

### Corrections

1. Centralize env parsing and validation in a typed config module.
2. Validate incompatible/unsafe config combinations early.
3. Keep strict TLS default; print explicit warning only when relaxed mode enabled.
4. Add optional redaction helpers for sensitive config in logs.
5. Add `cargo audit` to regular checks and document response process.

### Deliverables

- Typed validated config with clear startup error messages.
- Security checks documented and automated where feasible.

### Acceptance Criteria

- Invalid config fails fast with clear guidance.
- No secrets/tokens emitted to logs.

### Agent Prompt

```text
Phase 6 task: implement config and security hardening.
Create a typed configuration layer with validation and safe defaults.
Preserve strict TLS defaults and cert pinning behavior.
Ensure no sensitive values are logged.
Return config schema, validation rules, and migration notes.
```

---

## Phase 7 - Release Readiness

### Objectives

- Standardize release process and confidence checks.
- Make production rollout repeatable.

### Corrections

1. Add release checklist document:
   - version bump
   - changelog entry
   - CI green
   - smoke run command
2. Define bounded smoke tests and expected outcomes.
3. Add simple rollback guidance (binary/version + data artifacts).
4. Tag release policy and support matrix (Rust 1.93, edition 2024).

### Deliverables

- `RELEASE.md` (or equivalent) with cut-and-paste process.
- A reliable pre-release smoke test procedure.

### Acceptance Criteria

- A new maintainer can execute release steps without tribal knowledge.
- Rollback steps are explicit and tested.

### Agent Prompt

```text
Phase 7 task: create release readiness assets.
Add a release checklist, smoke test protocol, and rollback guidance.
Keep commands copy/paste-ready and aligned with justfile recipes.
Return the final release flow in numbered steps.
```

---

## Execution Order and Estimated Impact

- Phase 0: very low effort, immediate hygiene gain
- Phase 1-2: moderate effort, major correctness/confidence gain
- Phase 3-4: moderate/high effort, biggest reliability/performance gain
- Phase 5-6: moderate effort, major operability/security gain
- Phase 7: low/moderate effort, strong maintenance gain

Expected score progression (approximate):

- After Phase 0-2: **7.5/10**
- After Phase 3-4: **8.2/10**
- After Phase 5-7: **8.8-9.1/10**

## Final Verification Checklist (after each phase)

Run:

```bash
cargo fmt -- --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

For stream behavior changes, also run a bounded smoke test:

```bash
FROM_BLOCK=84023890 TO_BLOCK_EXCL=84023910 FOLLOW_TAIL=false cargo run --quiet
```
