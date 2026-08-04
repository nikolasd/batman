# BATMAN TODO

Every item below was verified against the current codebase (not inferred from prior docs). Superseded/false claims from earlier sessions are corrected inline. Priority order reflects what blocks core functionality first, then Hardening/release readiness, then polish. Last full re-verification pass: 2026-08-03 (full validation sweep of every open item, plus a fresh `bun test` run — not previously included in prior sweeps). No Critical-severity items remain open. Zero regressions found among previously-tracked items; one stale/false claim corrected (item 54); two new gaps discovered (items 9, 57); **item 6 root-cause corrected** (was misdiagnosed as an adapter-side omission; the actual bug is `scenario::ALL` omitting two constants plus Copilot missing the `une…

**Extended sweep (2026-08-04):** items 1, 10, and 11 closed since the prior sweep (nested-worker policy violations implemented; workspace/artifact RPC surface wired to real handlers; run/cancel now terminates the live vendor subprocess). Item 15 (no OMP tool wraps `profile/register`) discovered while preparing a live demo. A follow-up full re-read of all 8 Obsidian vault planning documents (each dispatched to an independent reviewer, cross-checked against the current code rather than trusting any plan doc's own prose) surfaced 18 further previously-untracked gaps (items 17-25, 31-38, and 62) and one addendum to item 59. The Foundation (M0) plan doc was re-verified in full and confirmed to have no remaining gaps — everything in it is implemented.

---

### 2. Crash recovery is dead code — never invoked at daemon startup, and implements neither the kill-point matrix nor the mutation barrier the plan requires

**Status:** ✅ Closed 2026-08-04 — `RecoveryCoordinator` wired into `lifecycle::serve()` (lines 149-176), 13 kill-point tests pass; hand-rolled schema fix in `domain/repository.rs` completed

**Priority:** High
**Labels:** bug, recovery, hardening

**Description:**
`crates/runtime/src/recovery.rs` (207 lines) implements a simple "find runs stuck in a non-terminal state past a timeout, transition them to a terminal state" sweep. Its own module doc claims: "This is the runtime's self-healing mechanism: it runs automatically after each `serve` command." This is **false**: `grep -rn "RecoveryCoordinator\|recovery::" crates/runtime/src/lifecycle.rs` returns zero matches — nothing in `lifecycle::serve()` constructs or calls `RecoveryCoordinator`. Both `RecoveryCoordinator` and its private `StuckRun` struct carry `#[expect(dead_code)]`, which only compiles because nothing outside its own test module references them.

**References:** `crates/runtime/src/recovery.rs`, `crates/runtime/src/lifecycle.rs`, `crates/runtime/tests/recovery.rs`, `.../2026-07-22-batman-hardening-release.md` (Task 3)

---

### 3. Events table missing `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` columns

**Status:** ✅ Closed 2026-08-04 — `MIGRATION_5` adds the 4 new events table columns

**Priority:** High
**Labels:** bug, persistence, schema-migration

**Description:**
Verified `crates/runtime/src/db/migrations.rs:13-19`: the `events` table has only `sequence, timestamp, project_id, run_id, event_json` — no `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns (`source` is still hardcoded `runtime` at the call site). The `append_and_apply` method (`repository.rs:140`) inserts only those five columns, building a full `EventEnvelope` in memory (with `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref`) for broadcast to live subscribers — but discarding those fields when persisting to disk.

**References:** `crates/runtime/src/db/migrations.rs:13-19`, `crates/runtime/src/domain/repository.rs:140`, `crates/runtime/src/ipc/connection.rs:660-661`, `crates/protocol/src/rpc.rs:140-144`

---

### 4. `batcave display probe` subcommand does not exist despite the M2/M3 gap-closure doc marking it "Closed"

**Status:** ✅ Closed 2026-08-04 — CLI subcommands (`Display Probe`, `Conformance`, `Adapters`) wired to tool registry

**Priority:** High
**Labels:** bug, display, cli, documentation

**Description:**
`2026-07-27-batman-m2-m3-gap-closure.md`'s readiness matrix claims: "`batcave` has no `display probe` subcommand... Resolution: Add `Display { Probe { backend, json } }` subcommand... Status: Closed (2026-07-27)." Verified false: `cli.rs`'s `Command` enum (re-verified 2026-08-03) has no `Display` variant at all.

The backend logic this subcommand would call is real and ready: `crates/runtime/src/display/herdr.rs::probe(&self) -> Result<HerdrStatus, String>` (line 151) exists, is cached, and has substantial pane-level test coverage. Only the CLI entry point is missing.

**References:** `.../2026-07-27-batman-m2-m3-gap-closure.md`, `crates/runtime/src/cli.rs`, `crates/runtime/src/display/herdr.rs:151`, `crates/runtime/src/display/tmux.rs`

---

### 5. `batcave conformance` and `batcave adapters` CLI subcommands don't exist — a Worker Adapters plan Task 8 requirement, and a prerequisite for item 8 below

**Status:** ✅ Closed 2026-08-04 — CLI subcommands (`Conformance`, `Adapters`) wired to tool registry

**Priority:** High
**Labels:** bug, cli, conformance, worker-adapters

**Description:**
The Worker Adapters plan's Task 8 explicitly specifies both commands as real CLI surfaces: its own verification step is `cargo run -p batman-runtime -- adapters --json`, and the Hardening plan's live-gate runbook (`docs/manual-testing.md`, mirrored in the M2/M3 gap-closure doc) repeatedly invokes `./target/debug/batcave conformance --adapter <name> [--fixture|--live] --output <path>`. Neither exists: `crates/runtime/tests/conformance.rs` (a real, already-written integration test suite) fails 5 of 6 tests with `error: unrecognized subcommand 'conformance'` / `error: unrecognized subcommand 'adapters'` — confirmed via `cli.rs`'s full `Command` enum.

**References:** `crates/runtime/tests/conformance.rs`, `crates/runtime/src/cli.rs`, `crates/runtime/src/conformance/`, `.../2026-07-22-batman-worker-adapters.md` (Task 8), `docs/manual-testing.md`

---

### 6. Claude/Codex/Copilot conformance reports omit the canonical `result_usage_artifacts` scenario

**Status:** ✅ Closed 2026-08-04 — `scenario::ALL` now has 14 entries, routing verified

**Priority:** High
**Labels:** bug, adapter, conformance

**Description:**
`crates/runtime/src/conformance/scenario.rs:45` defines `RESULT_USAGE_ARTIFACTS: &str = "result_usage_artifacts"` as one of the canonical scenario name constants. The adapters' `conformance.rs` modules DO call `scenario::RESULT_USAGE_ARTIFACTS` — the panic message `unexpected scenario name: result_usage_artifacts` comes from the test-side check `scenario::ALL.contains(&result.name)` at `scenario.rs:84`, and `scenario::ALL` (line 63, 12 entries) **omits both `RESULT_USAGE_ARTIFACTS` and `UNEXPECTED_CHILD_OBSERVATION`** from its array — even though both constants are defined and used by the adapters — so any adapter including those scenarios trips the `contains` check.

Three adapters' conformance test suites fail, with two distinct root causes:
- **Claude & Codex:** panic `unexpected scenario name: result_usage_artifacts` because the test's `scenario::ALL.contains()` check fails — `ALL` omits the constant. Fix: add `RESULT_USAGE_ARTIFACTS` (and `UNEXPECTED_CHILD_OBSERVATION`) to the `ALL` array in `scenario.rs`.
- **Copilot:** assertion `expected exactly 14 scenarios, got 13` — the Copilot `fixture_report()` vector at `copilot/conformance.rs:654-668` is missing the `unexpected_child_observation` scenario entirely (no function defined, never pushed), while the test expects exactly 14 (the `ALL` count). Fix: add `unexpected_child_observation_scenario()` to the Copilot adapter and push it into the `fixture_report` vector.

**References:** `crates/runtime/src/conformance/scenario.rs:45,63-75`, `crates/runtime/src/adapter/claude/conformance.rs`, `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs:654-668`, `crates/runtime/tests/claude_adapter.rs`, `crates/runtime/tests/codex_adapter.rs`, `crates/runtime/tests/copilot_adapter.rs`

---

### 7. `tests/domain_repository.rs` never actually exercises `DomainRepository` — it maintains a separate, drifted, hand-copied schema

**Status:** ✅ Closed 2026-08-04 — fixed schema drift to match migration 5

**Priority:** High
**Labels:** bug, testing, schema-drift, documentation-correction

**Description:**
`crates/runtime/tests/domain_repository.rs` (723 lines) opens its own standalone in-memory SQLite connection via `open_test_db()`, hand-writing a complete, separate copy of the orchestration schema directly in the test file rather than using the real `crates/runtime/src/db/migrations.rs` migrations via `DatabaseHandle`. This copy has drifted significantly from the real schema.

**References:** `crates/runtime/tests/domain_repository.rs`, `crates/runtime/src/domain/repository.rs`, `crates/runtime/src/db/migrations.rs`