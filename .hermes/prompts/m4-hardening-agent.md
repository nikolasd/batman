# BATMAN M4 — Hardening & Release Agent Prompt

You are a senior Rust/TypeScript engineer. Your task is to implement **M4 (Hardening & Release)** of the BATMAN project, a repository-scoped OMP extension daemon.

## Repository

```
~/Personal/Repos/batman/   # git root on macOS MBP (arm64)
```

**Current state:** 75 commits on `main`, clean working tree. M0 (Foundation), M1 (Orchestration Extension), M2 (Worker Adapters), M3 (Workspaces & Displays), and M3.5 (Gap Closure) are all complete. M4 (8 tasks) is entirely not started.

## Architecture at a glance

- **`crates/protocol/`** — Canonical Rust wire types (source of truth for JSON-RPC protocol). Generates JSON Schema + TypeScript bindings.
- **`crates/runtime/`** — The `batcave` daemon. Tokio async, CLI via clap, SQLite via rusqlite with `rusqlite_migration`, protocol, lifecycle, IPC, domain persistence, orchestration, coordination, approvals, display backends (herdr/tmux/terminal), adapter registry for Claude/Codex/Copilot/OMP-RPC.
- **`crates/xtask/`** — Codegen for schema/TS bindings + package assembly.
- **`packages/extension/`** — OMP extension in TypeScript/Bun.
- **`packages/protocol-ts/`** — Generated TypeScript bindings + JSON Schema + Ajv validators.
- **`fixtures/`** — Cross-language golden fixtures.
- **`docs/`** — engineering docs (getting-started, manual-testing, architecture, etc.)

## Key conventions you MUST follow

1. **Rust types are canonical.** Run `bun run generate` (which calls `cargo run -p batman-xtask -- generate`) after ANY protocol type change. With `--check` to verify no drift.
2. **TDD: tests before code.** For every M4 task, first write the test file that describes the expected behaviour, watch it fail, then implement until it passes. Tests live in `crates/runtime/tests/` for integration tests and at the bottom of each source file with `#[cfg(test)]` for unit tests.
3. **Clippy warnings denied.** Run `cargo clippy --workspace --all-targets --all-features -- -D warnings` and fix every issue.
4. **No test can require network calls.** Fixture mode only. No model calls, no API keys.
5. **SQLite schema changes** go through `crates/runtime/src/db/migrations.rs` using `rusqlite_migration::M`. Never raw DDL.
6. **Generated code is never hand-edited.** Only `bun run generate` produces changes to `packages/protocol-ts/` or `packages/protocol-ts/schema/batman.schema.json`.
7. **Commit granularity:** one commit per logical unit (one step in a task). Use `git commit -m "feat(ctx): description"` format.
8. **Documentation updates** are part of every task, not a separate afterthought. Update `docs/` files when behaviour changes.

## Build & test commands

```bash
cd ~/Personal/Repos/batman

# Full check (what CI runs)
bun run check          # generate --check + build + bun test + cargo test

# Rust test by area
cargo test -p batman-runtime --test <name>
cargo test -p batman-protocol
cargo test -p batman-xtask

# Clippy (must pass)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format
cargo fmt --all --check

# Build
cargo build -p batman-runtime
```

## Existing test integration tests (references for style)

- `crates/runtime/tests/redaction_boundary.rs` — tests for security/redaction
- `crates/runtime/tests/display_selector.rs` — tests for display backend selection
- `crates/runtime/tests/display_registry.rs` — tests for display registry
- `crates/runtime/tests/database.rs` — tests for SQLite actor
- `crates/runtime/tests/lifecycle.rs` — tests for daemon lifecycle
- `crates/runtime/tests/approval.rs` — tests for approval service
- `crates/runtime/tests/domain_repository.rs` — tests for domain persistence
- `crates/runtime/tests/orchestration_rpc.rs` — tests for orchestration RPC
- `crates/runtime/tests/coordination.rs` — tests for coordination broker
- `crates/runtime/tests/conformance.rs` — tests for adapter conformance

## Implementation Plan (8 tasks, do them IN ORDER)

---

### Task 1: Configuration precedence and immutable effective policy

**Goal:** Implement multi-layer config merging with organization-locked fields, strict YAML parsing, SHA-256 fingerprinting, and `PolicyEvaluator` that implements the existing `AdapterAuthorization` trait.

**Files to create:**
- `crates/runtime/src/config/mod.rs` — Typed config patch types + merge logic
- `crates/runtime/src/config/merge.rs` — Layer merging with lock enforcement
- `crates/runtime/src/policy/mod.rs` — Policy evaluator module
- `crates/runtime/src/policy/evaluate.rs` — `PolicyEvaluator` implementing `AdapterAuthorization`
- `fixtures/config/org.yml` — Example org-level config fixture
- `fixtures/config/repo.yml` — Example repo-level config fixture
- `fixtures/config/user.yml` — Example user-level config fixture
- `packages/extension/src/config.ts` — TypeScript side of config resolution

**Files to modify:**
- `crates/runtime/src/lib.rs` — Export config/policy modules
- `Cargo.toml` — Add `serde_yaml_ng` if needed
- `crates/runtime/Cargo.toml` — Add deps if needed
- `packages/extension/src/monitor/model.ts` — Wire policy info to monitor
- `packages/extension/src/tools/workers.ts` — Pass policy to run creation

**Test files to create:**
- `crates/runtime/tests/config.rs` — Integration tests for precedence, locks, strict parsing, fingerprints, unknown-YAML rejection
- `packages/extension/src/config.test.ts` — TypeScript config tests

**Key design decisions:**
1. Precedence (highest wins): per-run params → user policy → repo policy → org policy
2. Org policy can lock fields — locked fields from lower layers are rejected, not merged
3. Unknown YAML keys fail closed with line/column diagnostics
4. Result is an immutable `EffectivePolicy` with SHA-256 fingerprint
5. Display preference follows same precedence; absent fields resolve to `backend: auto`
6. `PolicyEvaluator` implements the existing `AdapterAuthorization` trait from `crates/runtime/src/adapter/trait.rs`
7. Wired through `AdapterRegistry` at daemon startup (in `crates/runtime/src/lifecycle.rs`)
8. Policy violation events: add `PolicyViolation` variant to `RuntimeEvent` in protocol, regenerate bindings

**Verification:**
- Precedence tests pass (org locks retention=30d, max_workers=8; repo sets 4; user sets 6; per-run sets 2 — result is 2, display preference wins from user layer)
- Locked-field override rejected with clear error
- Unknown YAML keys rejected
- `PolicyEvaluator` correctly denies disallowed models
- Concurrency enforcement (3rd run blocked when ceiling=2)
- Nested worker policy violation recorded

**Commit after verification:** `git commit -m "feat(policy): merge immutable runtime configuration"`

---

### Task 2: Redact secrets, define audit retention, and export

**Goal:** Extend the existing redaction boundary with organization-configurable rules, implement audit retention/pruning, add `batcave audit export --jsonl` command.

**Files to create:**
- `crates/runtime/src/security/rules.rs` — Organization redaction rules
- `crates/runtime/src/audit/mod.rs` — Audit module
- `crates/runtime/src/audit/retention.rs` — Pruning logic
- `crates/runtime/src/audit/export.rs` — Export logic

**Files to modify:**
- `crates/runtime/src/security/mod.rs` — Wire org rules into redactor
- `crates/runtime/src/security/redaction.rs` — Extend `Redactor` with org pattern loading
- `crates/runtime/src/db/migrations.rs` — Add migration 4 for audit/metadata tables
- `crates/runtime/src/cli.rs` — Add `Audit { Export { repo, state_dir, from, to, output } }` subcommand
- `crates/runtime/src/lib.rs` — Export audit module

**Test files to create:**
- `crates/runtime/tests/redaction.rs` — Full redaction boundary tests (API keys, bearer tokens, private keys, thinking blocks, org patterns)
- `crates/runtime/tests/audit.rs` — Retention pruning, export format tests

**Key design decisions:**
1. Organization rules are loaded from policy config, compiled at startup using bounded `regex` engine
2. Raw secret-shaped values never appear in SQLite, WAL, logs, monitor, or export
3. Thinking content is dropped entirely (not replaced), secrets become `[REDACTED:<rule-id>]`
4. Retention runs only while no migration/recovery transaction is active
5. Export is redacted JSONL (every string scanned before output)
6. Existing `Redactor` struct is extended, not replaced — it already does built-in API key/bearer token redaction

**Verification:**
- API keys, bearer tokens, private-key blocks redacted in events, artifacts, logs, export
- Thinking fragments absent from journal
- Org-loaded patterns applied correctly
- Retention pruning removes expired events but preserves active-run data
- Export command produces valid JSONL with no secret content

**Commit:** `git commit -m "feat(security): redact and retain audit records"`

---

### Task 3: Crash recovery and reconciliation

**Goal:** Implement `RecoveryCoordinator` that runs at daemon startup, before accepting mutation methods, reconciling incomplete operations based on the intent journal.

**Files to create:**
- `crates/runtime/src/recovery/mod.rs` — Recovery coordinator
- `crates/runtime/src/recovery/operations.rs` — Operation-level reconciliation
- `crates/runtime/src/recovery/workers.rs` — Worker process reconciliation
- `crates/runtime/src/recovery/orphans.rs` — Orphan detection
- `crates/runtime/src/recovery/workspaces.rs` — Workspace lease reconciliation
- `crates/runtime/tests/recovery.rs` — Full kill-point test matrix

**Files to modify:**
- `crates/runtime/src/lifecycle.rs` — Wire recovery barrier before serve

**Key invariants:**
1. Recovery completes before the runtime accepts any mutation method
2. Every recovery decision is recorded as a committed `RecoveryStarted`, per-item decision, and `RecoveryCompleted` event
3. Unacknowledged message → `unknown` delivery state
4. Vendor-resumable sessions → resume through a new process
5. Runtime-scoped workers → reconnect only with verifiable live PID + process identity (never PID alone)
6. Parent-scoped OMP-native runs → `lost` state
7. Orphaned active runs → pause, create no child/task
8. Stale workspace leases with no run → quarantined, not deleted

**Verification:**
- Kill-after-intent test: after restart, unacknowledged operation is `unknown`, no duplicate side effect
- Kill-after-vendor-ack: session resumes through new process
- Kill-after-event-append: event is durable, no duplicate
- Orphaned runtime-scoped worker: paused, no new child
- Stale workspace: quarantined, not deleted
- Run `cargo test -p batman-runtime --test recovery -- --test-threads=1` — all pass

**Commit:** `git commit -m "feat(runtime): reconcile state after crashes"`

---

### Task 4: Health, doctor, and rollout gates

**Goal:** Implement `batcave doctor --json` CLI command that runs side-effect-free checks and reports a machine-readable status. Checks include CLI versions, display backends, database integrity, policy completeness, and unresolved rollout gates.

**Files to create:**
- `crates/runtime/src/doctor/mod.rs` — Doctor module
- `crates/runtime/src/doctor/check.rs` — Individual check implementations
- `crates/runtime/src/doctor/report.rs` — Report formatting
- `crates/runtime/tests/doctor.rs` — Doctor test suite
- `packages/extension/src/doctor.ts` — OMP `/batman-doctor` tool

**Files to modify:**
- `crates/runtime/src/cli.rs` — Add Doctor subcommand
- `crates/runtime/src/lib.rs` — Export doctor module

**Checks to implement:**
1. **platform:** OS, libc, arch match supported targets
2. **state permissions:** state root is private (0700/0600)
3. **database integrity:** SQLite integrity_check passes
4. **binary integrity:** schema fingerprint matches committed
5. **CLI versions:** claude, codex, copilot, omp detected with versions
6. **display backends:** herdr (with protocol compatibility), tmux availability, embedded always pass
7. **policy completeness:** all required policy values set
8. **rollout gates:** vendor terms, retention configured, model allowlist, concurrency ceiling, native-discovery review, Ornith identity — each is `productionBlocking: true` if unresolved

**Key design rules:**
- Checks never launch a model, stop Herdr, mutate config, or clean state
- Each check returns `pass|warn|fail` with evidence and remediation text
- `--json` flag produces a structured report
- `/batman-doctor` in OMP renders the same report

**Verification:**
- Doctor runs in fixture mode (no side effects)
- Structured CLIs detected correctly
- Missing rollout gates reported as `productionBlocking:true`
- Test with known fixture baselines (Claude 2.1.217, Codex 0.145.0, etc.)

**Commit:** `git commit -m "feat: report runtime and rollout readiness"`

---

### Task 5: Cross-platform release artifacts

**Goal:** Build CI workflows, xtask `package-set` command, and platform leaf package structure for all 4 target triples.

**Files to create:**
- `.github/workflows/ci.yml` — CI workflow (format, clippy, tests, conformance, package)
- `.github/workflows/release.yml` — Release workflow
- `crates/xtask/src/package.rs` — Package-set assembly logic
- `crates/xtask/src/manifest.rs` — Manifest generation
- `release/targets.json` — Target triple metadata
- `crates/xtask/tests/package.rs` — Package determinism tests
- `tests/conformance/run.ts` — Conformance test runner script

**Files to modify:**
- Platform leaf `package.json` files (darwin-arm64, darwin-x64, linux-arm64-gnu, linux-x64-gnu)
- `crates/xtask/src/main.rs` — Wire `package-set` command

**Key design:**
- 4 targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`
- `batman-xtask package-set --version <semver> --input <dir> --output <dir>` creates tarballs + manifest
- CI: formatting, clippy with warnings denied, Rust/Bun tests, generation drift check, secret scan, fixture conformance
- Release CI: build on all 4 targets, build core + leaf packages, verify checksums, publish to npm
- Windows/musl targets are explicitly rejected at the type level

**Verification:**
- Package generation produces 5 npm tarballs + manifest + SBOM
- Missing target, version mismatch, wrong binary name all fail
- Install test selects correct platform leaf

**Commit:** `git commit -m "build: assemble verified platform packages"`

---

### Task 6: Full conformance and platform CI gates

**Goal:** Implement conformance test runner and assertion reporter in TypeScript, wire them into CI as publish gates.

**Files to create:**
- `tests/conformance/run.ts` — Runs all common + adapter-specific fixture scenarios
- `tests/conformance/assert-report.ts` — Validates report completeness
- `tests/install/private-registry.test.ts` — Install flow test

**Files to modify:**
- `.github/workflows/ci.yml` — Add conformance step
- `.github/workflows/release.yml` — Gate publish on conformance pass

**Key design:**
- Report format: array of `{ adapter, scenarios: [{ name, passed, reason }], timestamp }`
- Release refuses to publish unless every advertised capability has a passing scenario
- Test at least: cancellation, recovery, native discovery, redaction, display equivalence, unexpected-child policy enforcement

**Verification:**
- `bun tests/conformance/run.ts --mode fixture --report /tmp/report.json` produces valid report
- `bun tests/conformance/assert-report.ts /tmp/report.json` validates completeness
- Missing scenario causes assertion failure

**Commit:** `git commit -m "test: gate releases on adapter conformance"`

---

### Task 7: Operator and compatibility documentation

**Goal:** Write all M4 documentation files based on actual passing test/doctor/conformance output. Never invent commands or outputs — capture them from real runs.

**Files to create:**
- `README.md` — Update to reflect M4 status (current says M1)
- `docs/installation.md` — Private npm install, OMP config, daemon commands
- `docs/configuration.md` — Policy file syntax, precedence, lock semantics
- `docs/operations.md` — Normal stop/restart, coordinated Herdr restart, tmux fallback, package rollback, uninstall
- `docs/security.md` — Redaction boundary, audit, file permissions, socket security
- `docs/recovery.md` — Crash states, `unknown` delivery, `lost` parent-scoped runs, manual reconciliation
- `docs/compatibility.md` — Generated from conformance reports: adapter capabilities by version

**Key rule:** Every copyable command must succeed when run verbatim in a clean temporary HOME with fixture mode. Do not paste hypothetical output.

**Verification:**
- All commands in the new docs produce the documented output
- `batcave doctor --json` output matches what's documented
- Fixture conformance report matches documented capabilities

**Commit:** `git commit -m "docs: add Batman operator runbook"`

---

### Task 8: Release candidate 0.1.0

**Goal:** Produce a verified `0.1.0` release candidate with all pre-requisite checks passing.

**Files to modify:**
- `package.json` — Set version
- `packages/extension/package.json` — Set version
- All platform leaf `package.json` — Set version
- Create `release/0.1.0-checklist.json` — Evidence of every code/conformance/platform gate

**Verification (run full suite):**
```bash
bun install --frozen-lockfile
bun run generate --check
bun test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
bun tests/conformance/run.ts --mode fixture --report /tmp/batman-report.json
bun tests/conformance/assert-report.ts /tmp/batman-report.json
cargo run -p batman-runtime -- doctor --json
```

The candidate is technically complete only when all the above pass. Doctor may still report unresolved rollout gates — those go in the checklist as `blocked` but never block the candidate.

**Commit:** `git commit -m "chore: prepare Batman 0.1.0 candidate"`

---

## Final deliverable

After completing all 8 tasks, produce:

1. **A summary of what changed** — bullet list per task, with file paths and key design decisions.
2. **A manual test walkthrough** — exactly what commands the user can run to verify the hardening release end-to-end. Start from `bun install` and end with `batcave doctor --json`. Include:
   - The daemon cycle (serve → status → stop)
   - `batcave doctor --json` output (expected shape)
   - Display probe (`batcave display probe --backend herdr`, `batcave display probe --backend tmux`)
   - Adapter conformance (`batcave conformance --adapter all --fixture --output /tmp/report.json`)
   - Policy enforcement test (set a restrictive org policy and verify rejection)
   - Recovery test (kill daemon during a run, restart, verify state reconciliation)
   - Package install test (build, pack, install from local tarball)

## Code quality rules

1. **Every new public type** has doc comments.
2. **Every new function** has a doc comment explaining what it does, what it returns, and errors.
3. **No unwrap()** in production code (use `.context()` from `anyhow`, or proper error types).
4. **No wildcard imports** (`use module::*`) — name imports explicitly.
5. **No `todo!()` or `unimplemented!()`** in production code — only in test stubs that are expected to fail.
6. **Constants over magic numbers** — every literal has a named constant.
7. **Integration tests** use `tempfile::tempdir()` for state directories, never real state.
8. **Event journal** is append-only — no UPDATE OR DELETE on the events table.
9. **New SQLite migrations** use `M::up("...")` with a const string (see `crates/runtime/src/db/migrations.rs` for style).
10. **Commit messages** follow conventional commits format: `feat(scope): description`, `fix(scope): description`, `docs(scope): description`, `test(scope): description`.

## Important paths reference

| Path | Purpose |
|------|---------|
| `crates/runtime/src/` | Daemon library root |
| `crates/runtime/tests/` | Rust integration tests |
| `crates/protocol/src/` | Wire types |
| `crates/xtask/src/` | Codegen + packaging |
| `packages/extension/src/` | OMP extension |
| `packages/protocol-ts/` | Generated TS bindings |
| `fixtures/` | Golden test fixtures |
| `docs/` | Documentation |
| `crates/runtime/src/db/migrations.rs` | SQLite migrations |
| `crates/runtime/src/adapter/trait.rs` | Adapter trait with `AdapterAuthorization` |
| `crates/runtime/src/security/` | Existing security module |
| `crates/runtime/src/lifecycle.rs` | Daemon lifecycle |
| `crates/runtime/src/cli.rs` | CLI argument parsing |
| `crates/runtime/src/display/mod.rs` | Display backend registry/selector |
