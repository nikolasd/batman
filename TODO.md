# BATMAN TODO

Every item below was verified against the current codebase (not inferred from prior docs). Superseded/false claims from earlier sessions are corrected inline. Priority order reflects what blocks core functionality first, then Hardening/release readiness, then polish. Last full re-verification pass: 2026-08-03 (full validation sweep of every open item, plus a fresh `bun test` run — not previously included in prior sweeps). No Critical-severity items remain open. Zero regressions found among previously-tracked items; one stale/false claim corrected (item 27); two new gaps discovered (items 9, 30); **item 6 root-cause corrected** (was misdiagnosed as an adapter-side omission; the actual bug is `scenario::ALL` omitting two constants plus Copilot missing the `unexpected_child_observation` scenario function); **item 3 cross-referenced** against `repository.rs:140` INSERT and `connection.rs:660-661` replay reconstruction; **item 26 confirmed** `1.0.77` still absent from `COPILOT_KNOWN_CLI_VERSIONS`.\n\n**Open test failures (current, 2026-08-03):**\n- Rust: 7 failing — `claude_adapter` (1, item 6), `codex_adapter` (1, item 6), `copilot_adapter` (2: item 6 + item 26), `conformance` (5, item 5); all others pass\n- Bun: 2 failing — `runtime.test.ts` (item 9), `index.test.ts` (item 30)

**Extended sweep (same date):** cross-referenced every item against the 8 planning documents in the Obsidian vault (`10 Projects/Batman/`) and did a deeper, file-by-file read of `workspace/`, `approval/`, `supervisor/`, `coordination/`, `service/`, `audit/`, `crates/protocol/`, `packages/protocol-ts/`, and `packages/extension/src/` than any prior sweep. This surfaced ten more previously-untracked gaps (items 32-41) and narrowed item 15's "Closed" claim (see item 35). Two issues from an earlier ad hoc review of `retention.rs` (a TEXT/INTEGER cutoff-comparison bug and a wrong terminal-state list) were re-checked here and found already fixed by commit `7c05d19` — not re-added. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --doc` were each re-run twice; no failures beyond the ones already tracked (items 5, 6, 26, and the documented `lifecycle` flake) were found.

---

## High — blocks Hardening (M4) readiness

### 1. Nested-worker policy violations are journaled but never quarantined, cancelled, or reported to OMP — `policy/violation/decide` is a stub

**Status:** Closed 2026-08-04
**Priority:** High
**Labels:** security, policy, hardening

**Resolution (2026-08-04):**
- ✅ Implemented `ViolationService` (`crates/runtime/src/policy/violation.rs`) with `record()` (idempotent per Option B — quarantine/cancel applied once, `PolicyViolationRecorded` journaled every time) and `decide()` (ownership check, conflict/idempotency, refuses `release` on a terminal run)
- ✅ Added `MIGRATION_4` creating `policy_violations` table with `violation_id`, `run_id`, `task_id`, `worker_id`, `vendor_child_id`, `vendor_parent_ref`, `action`, `created_at`, `resolved_at`, `resolution`, `resolved_by`
- ✅ Added `PolicyViolationRecorded` and `PolicyViolationDecided` to `RuntimeEventKind` (Kind enum) and `RuntimeEvent` (outer enum); added `PolicyViolationId` (UUIDv7 newtype) to `crates/protocol/src/ids.rs`
- ✅ Wired `DomainAdapterEventSink` to call `ViolationService::record` on `NestedWorkerObserved` when `effective_capabilities.nested != NestedCapability::Managed` (covers both `None` and `Observable`)
- ✅ Implemented `policy/violation/decide` RPC — real implementation replaces the stub, restricted to `ompExtension` client with per-resource ownership check
- ✅ Added enforcement gates: `message/send`, `workspace/apply` (`OrchestrationService`), `coordination/publishArtifact` (`CoordinationBroker`) — all check `Run.flags.policyQuarantined`, error code `POLICY_QUARANTINED` (-32101)
- ✅ Added `nested_violation_action` config knob (3 variants: `Quarantine`/`Cancel`/`QuarantineAndCancel`) threaded from `RuntimePolicy.rollout_gates` through `ServerConfig` into `ViolationService`
- ✅ Added 4 new integration tests in `orchestration_rpc.rs`: quarantine blocks `message/send` until released, `decide` forbidden for non-owning client, `release` refused on terminal run, second observation on already-actioned run never double-cancels
- ✅ Generated protocol-ts bindings updated (`batman.schema.json`, `RuntimeEvent.ts`, `RuntimeEventKind.ts`, new `PolicyViolationId.ts`)
- ✅ All 24 orchestration_rpc tests pass (20 existing + 4 new); all other test suites pass

---

### 2. Crash recovery is dead code — never invoked at daemon startup, and implements neither the kill-point matrix nor the mutation barrier the plan requires

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** bug, recovery, hardening

**Description:**
`crates/runtime/src/recovery.rs` (207 lines) implements a simple "find runs stuck in a non-terminal state past a timeout, transition them to a terminal state" sweep. Its own module doc claims: "This is the runtime's self-healing mechanism: it runs automatically after each `serve` command." This is **false**: `grep -rn "RecoveryCoordinator\|recovery::" crates/runtime/src/lifecycle.rs` returns zero matches — nothing in `lifecycle::serve()` constructs or calls `RecoveryCoordinator`. Both `RecoveryCoordinator` and its private `StuckRun` struct carry `#[expect(dead_code)]`, which only compiles because nothing outside its own test module references them.

This also falls far short of the Hardening plan's Task 3 specification: no `RecoveryCoordinator::run()` blocking-barrier before mutation methods are accepted, no separate `operations.rs`/`workers.rs`/`orphans.rs`/`workspaces.rs` submodules, and none of the 6 kill-points (intent / identity allocation / child spawn / vendor acknowledgement / event append / projection update) or 8 invariants (no duplicated prompts, `unknown` for unacknowledged messages, vendor-resumable sessions resuming via new process, PID+start-identity+executable verification before reconnect, parent-scoped runs becoming `lost`, orphaned runs pausing with no new children, protected active workspace leases, quarantined-not-deleted stale materialization) are tested. `crates/runtime/tests/recovery.rs` has exactly 3 tests (`recovery_config_custom_values`, `recovery_config_default_values`, `recovery_returns_empty_when_no_stuck_runs`) — all pass, but none exercise a real kill-point.

**Implementation:**
- Wire `RecoveryCoordinator::run()` (or equivalent) into `lifecycle::serve()` before the socket accepts mutation methods; remove `#[expect(dead_code)]` once it has a real caller
- Build the kill-point test matrix per the plan (`--test-threads=1`, since it manipulates real process/database state)
- Decide whether to keep the flat `recovery.rs` or split into the plan's submodules — the flat file is acceptable if it's actually wired and tested; the current problem is that it's neither

**References:** `crates/runtime/src/recovery.rs`, `crates/runtime/src/lifecycle.rs`, `crates/runtime/tests/recovery.rs`, `.../2026-07-22-batman-hardening-release.md` (Task 3)

---

### 3. Events table missing `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` columns

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** bug, persistence, schema-migration

**Description:**
Verified `crates/runtime/src/db/migrations.rs:13-19`: the `events` table has only `sequence, timestamp, project_id, run_id, event_json` — no `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns (`source` is still hardcoded `runtime` at the call site). The `append_and_apply` method (`repository.rs:140`) inserts only those five columns, building a full `EventEnvelope` in memory (with `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref`) for broadcast to live subscribers — but discarding those fields when persisting to disk. When `events/replay` reconstructs envelopes from the table rows, `ipc/connection.rs::replay()` (lines 660–661) hardcodes `task_id: None`, `worker_id: None`, `parent_worker_id: None`, `source: EventSource::Runtime`, and `vendor_event_ref: None` for every replayed event, since the table columns don't exist to read them from.

The monitor is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields — but any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` gets silently wrong (empty) results.

**Implementation:**
- Schema migration adding `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns to `events`
- Update `append_and_apply` in `crates/runtime/src/domain/repository.rs` to populate these columns
- Update `replay()` in `crates/runtime/src/ipc/connection.rs` to read the new columns

**References:** `crates/runtime/src/db/migrations.rs:13-19`, `crates/runtime/src/domain/repository.rs:140`, `crates/runtime/src/ipc/connection.rs:660-661`, `crates/protocol/src/rpc.rs:140-144`

---

### 4. `batcave display probe` subcommand does not exist despite the M2/M3 gap-closure doc marking it "Closed"

**Status:** Open (re-verified 2026-08-03, unchanged — documentation discrepancy confirmed, a fourth false "Closed" claim in that doc)
**Priority:** High
**Labels:** bug, display, cli, documentation

**Description:**
`2026-07-27-batman-m2-m3-gap-closure.md`'s readiness matrix claims: "`batcave` has no `display probe` subcommand... Resolution: Add `Display { Probe { backend, json } }` subcommand... Status: Closed (2026-07-27)." Verified false: `cli.rs`'s `Command` enum (re-verified 2026-08-03) has no `Display` variant at all.

The backend logic this subcommand would call is real and ready: `crates/runtime/src/display/herdr.rs::probe(&self) -> Result<HerdrStatus, String>` (line 151) exists, is cached, and has substantial pane-level test coverage (`crates/runtime/tests/herdr_display.rs`, `tmux_display.rs`, `display_registry.rs`). Only the CLI entry point is missing.

**Implementation:**
- Add `Display { Probe { backend: String, json: bool } }` to `cli.rs`'s `Command` enum, wired to the existing herdr/tmux probe logic
- Or, if intentionally descoped, correct the gap-closure doc's "Closed" status rather than leaving a fourth false claim in a document that already warns about three others

**References:** `.../2026-07-27-batman-m2-m3-gap-closure.md`, `crates/runtime/src/cli.rs`, `crates/runtime/src/display/herdr.rs:151`, `crates/runtime/src/display/tmux.rs`

---

### 5. `batcave conformance` and `batcave adapters` CLI subcommands don't exist — a Worker Adapters plan Task 8 requirement, and a prerequisite for item 8 below

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** bug, cli, conformance, worker-adapters

**Description:**
The Worker Adapters plan's Task 8 explicitly specifies both commands as real CLI surfaces: its own verification step is `cargo run -p batman-runtime -- adapters --json`, and the Hardening plan's live-gate runbook (`docs/manual-testing.md`, mirrored in the M2/M3 gap-closure doc) repeatedly invokes `./target/debug/batcave conformance --adapter <name> [--fixture|--live] --output <path>`. Neither exists: `crates/runtime/tests/conformance.rs` (a real, already-written integration test suite exercising the compiled binary) fails 5 of 6 tests with `error: unrecognized subcommand 'conformance'` / `error: unrecognized subcommand 'adapters'` — confirmed via `cli.rs`'s full `Command` enum (still 9 variants as of 2026-08-03: `Serve`, `Status`, `Stop`, `Monitor`, `Version`, `Schema`, `Audit`, `Doctor`, `CoordinationMcp` — no `Conformance`/`Adapters`).

Confirmed this is not caused by any change made in any session: reproduces identically across at least three separate re-runs (2026-08-02, 2026-08-03), with and without `git stash` reverting uncommitted files.

This blocks item 8 below at the root: item 8's own fix ("replace `runAllFixtures`'s stub loop with real `batcave conformance --adapter <name> --output <path>` subprocess invocations") cannot be implemented until this CLI surface exists to invoke.

**Implementation:**
- Add `Conformance { adapter: String, fixture: bool, live: bool, output: Option<PathBuf> }` and `Adapters { json: bool }` variants to `cli.rs`'s `Command` enum
- Wire them to the existing, tested `crate::conformance` report-generation logic (`crates/runtime/src/conformance/report.rs`, `scenario.rs`) and each adapter's own `conformance.rs` module
- Re-run `cargo test -p batman-runtime --test conformance` until all 6 tests pass

**References:** `crates/runtime/tests/conformance.rs`, `crates/runtime/src/cli.rs`, `crates/runtime/src/conformance/`, `.../2026-07-22-batman-worker-adapters.md` (Task 8), `docs/manual-testing.md`

---

### 6. Claude/Codex/Copilot conformance reports omit the canonical `result_usage_artifacts` scenario

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** bug, adapter, conformance

**Description:**
`crates/runtime/src/conformance/scenario.rs:45` defines `RESULT_USAGE_ARTIFACTS: &str = "result_usage_artifacts"` as one of the canonical scenario name constants every adapter's conformance report is expected to cover. The adapters' `conformance.rs` modules DO call `scenario::RESULT_USAGE_ARTIFACTS` — the panic message `unexpected scenario name: result_usage_artifacts` comes from the test-side check `scenario::ALL.contains(&result.name)` at `scenario.rs:84`, and `scenario::ALL` (line 63, 12 entries) **omits both `RESULT_USAGE_ARTIFACTS` and `UNEXPECTED_CHILD_OBSERVATION`** from its array — even though both constants are defined and used by the adapters — so any adapter including those scenarios trips the `contains` check.

Three adapters' conformance test suites fail, with two distinct root causes:
- **Claude & Codex** (`claude_adapter.rs::conformance_fixture_report_covers_every_canonical_scenario_and_all_pass`, `codex_adapter.rs::fixture_conformance_report_covers_every_canonical_scenario_exactly_once`): panic `unexpected scenario name: result_usage_artifacts` because the test's `scenario::ALL.contains()` check fails — `ALL` omits the constant. Fix: add `RESULT_USAGE_ARTIFACTS` (and `UNEXPECTED_CHILD_OBSERVATION`) to the `ALL` array in `scenario.rs`.
- **Copilot** (`copilot_adapter.rs::fixture_conformance_report_covers_every_canonical_scenario_and_provable_ones_pass`): assertion `expected exactly 14 scenarios, got 13` — the Copilot `fixture_report()` vector at `copilot/conformance.rs:654-668` is missing the `unexpected_child_observation` scenario entirely (no function defined, never pushed), while the test expects exactly 14 (the `ALL` count). Fix: add `unexpected_child_observation_scenario()` to the Copilot adapter and push it into the `fixture_report` vector.

Confirmed not caused by any change made in any session — reproduces identically across at least three separate re-runs (2026-08-02, 2026-08-03).

**Implementation:**
- **Fix `scenario::ALL` in `scenario.rs:63-75`**: add both `RESULT_USAGE_ARTIFACTS` and `UNEXPECTED_CHILD_OBSERVATION` to the array (they're defined at lines 45 and 57 respectively but omitted from `ALL`)
- **Add `unexpected_child_observation_scenario()` to `crates/runtime/src/adapter/copilot/conformance.rs`**: the Copilot fixture report at line 654 is missing this scenario (no function exists, not pushed into the vec), causing the 13-vs-14 count failure. Model after the Claude adapter's implementation (`claude/conformance.rs:245-285`)
- Re-run `cargo test -p batman-runtime --test claude_adapter --test codex_adapter --test copilot_adapter` until all conformance coverage tests pass

**References:** `crates/runtime/src/conformance/scenario.rs:45,63-75`, `crates/runtime/src/adapter/claude/conformance.rs`, `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs:654-668`, `crates/runtime/tests/claude_adapter.rs`, `crates/runtime/tests/codex_adapter.rs`, `crates/runtime/tests/copilot_adapter.rs`

---

### 7. `tests/domain_repository.rs` never actually exercises `DomainRepository` — it maintains a separate, drifted, hand-copied schema

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** bug, testing, schema-drift, documentation-correction

**Description:**
`crates/runtime/tests/domain_repository.rs` (723 lines) opens its own standalone in-memory SQLite connection via `open_test_db()`, hand-writing a complete, separate copy of the orchestration schema directly in the test file rather than using the real `crates/runtime/src/db/migrations.rs` migrations via `DatabaseHandle`. This copy has drifted significantly from the real schema: its `workers` table uses `profile_ref_id`/`profile_ref_fingerprint`/`profile_ref_adapter`/`profile_ref_model`/`profile_ref_permission_envelope` columns that do not exist anywhere in the real, migrated `workers` table (which has a single `profile_id` foreign key into `worker_profiles` instead — see item 20); its `tasks` table adds `goal`/`status` columns the real `tasks` table doesn't have at all.

More significantly: the file's own module doc claims "Verifies that `DomainRepository` commands execute event-append + projection-update in a single SQLite transaction" — but a full grep of the file for `DomainRepository`/`use batman_runtime::domain` finds **zero** references. This test file never imports or calls the real `DomainRepository` type at all; it reimplements similar-looking transaction/rollback/foreign-key-enforcement logic by hand against its own drifted schema. Its 6 tests (`run_submission_requires_task_and_worker`, `transactional_append_and_projection`, `worker_creation_requires_profile`, `illegal_transition_appends_no_event`, `rebuild_run_from_events_matches_projection`, `projection_failure_rolls_back_event`) all pass, but prove nothing about whether the real `DomainRepository` implementation behaves correctly — that real behavior is actually covered elsewhere, by `crates/runtime/src/domain/repository.rs`'s own inline `#[cfg(test)] mod tests` (confirmed passing, using the real migrated schema) and by `orchestration_rpc.rs`'s full RPC-to-repository integration tests (also passing).

Net effect: no functional bug in `DomainRepository` itself (it genuinely is tested correctly elsewhere), but this specific file's name and doc comment actively mislead about what it covers, and its schema will keep drifting further from reality with every future migration since nothing keeps the two copies in sync.

**Implementation:**
- Decide the file's actual purpose: if it's meant to test `DomainRepository` behavior, rewrite it to import and call the real type via `DatabaseHandle`/the real migrations (matching the pattern already used successfully by `coordination_mcp.rs` and the now-fixed `adapter_registry.rs`)
- If it's intentionally a lower-level, schema-agnostic transaction/rollback/foreign-key invariant test unrelated to `DomainRepository`'s specific implementation, rename the file and correct its module doc to stop claiming it verifies `DomainRepository` commands
- Either way, remove the duplicated hand-written schema or clearly mark it as intentionally synthetic/minimal, not a mirror of production

**References:** `crates/runtime/tests/domain_repository.rs`, `crates/runtime/src/domain/repository.rs`, `crates/runtime/src/db/migrations.rs`, `crates/runtime/tests/orchestration_rpc.rs`

---

### 8. Release conformance gate is a non-functional stub — writes empty reports, never invokes real adapter checks

**Status:** Blocked on item 5 (the `batcave conformance` CLI subcommand this fix needs to invoke does not exist yet) — re-verified 2026-08-03, unchanged
**Priority:** High
**Labels:** ci, testing, conformance, release

**Description:**
The Hardening plan (Task 6) requires `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, and `tests/install/private-registry.test.ts` at the repo root, wired into `release.yml` so the workflow "refuses publish unless every advertised capability has a passing scenario on the target build." Re-verified `tests/conformance/run.ts`: `runAllFixtures` still writes an empty `ConformanceReport` (`scenarios: []`, `declaredCapabilities: []`) for each of `claude`/`codex`/`copilot`/`omp-rpc` — still explicitly labeled `// STUB` in its own doc comment — rather than spawning `batcave conformance --adapter <name> --output <path>`. `assertReportComplete` only checks that each adapter key is present, not that any scenario ran or passed. `tests/install/private-registry.test.ts` is likewise still a stub.

This currently means the conformance job in `release.yml` cannot yet fail a release for a real regression — although the *empty-report* shape is intentionally rejected by `assert-report.ts`'s stricter validators elsewhere, so this is a release-blocking gate by omission rather than a false-pass.

**Implementation:**
- **First close item 5** — `batcave conformance` must exist as a real CLI subcommand before this stub can be replaced with real subprocess invocations
- Then replace `runAllFixtures`'s stub loop with real `batcave conformance --adapter <name> --output <path>` subprocess invocations, one per adapter, and merge the resulting reports
- Replace `assertReportComplete` with real scenario-level assertions (every declared capability has a corresponding passed scenario, per item 6's fix)
- Implement `private-registry.test.ts` for real: publish to a mock registry, install, verify the binary launches

**References:** `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, `tests/install/private-registry.test.ts`, `.github/workflows/release.yml`, `.../2026-07-22-batman-hardening-release.md` (Task 6)

---

### 9. `runtime/status.binarySource` is always `"unknown"` — the TS side already sets `BATMAN_BINARY_SOURCE`, the Rust CLI never reads it, and `batcave doctor` never checks it at all

**Status:** Open — newly discovered 2026-08-03 during a full validation sweep of every open TODO item (root-caused from a previously-untracked failing `bun test`)
**Priority:** High
**Labels:** bug, cli, observability, hardening

**Description:**
`packages/extension/src/runtime.test.ts::"a valid override is selected verbatim, bypassing the package resolver"` fails: it launches the daemon via a validated `OMP_BATMAN_BINARY` override and expects `runtime/status`'s `binarySource` field to report `"override"`, but gets `"unknown"`.

Root-caused precisely: `packages/extension/src/runtime.ts` already does its half of the job correctly — `selectBinary()` determines whether the binary came from the override or the packaged resolver, and `ensureRuntime` spawns the daemon with `env: { ...process.env, BATMAN_BINARY_SOURCE: binary.source }` (`runtime.ts:118-119`). But `crates/runtime/src/cli.rs`'s `run_serve` never reads that environment variable at all — it unconditionally hardcodes `binary_source: batman_protocol::BinarySource::Unknown` (`cli.rs:228`, the only occurrence of `binary_source` in the entire CLI layer). `BATMAN_BINARY_SOURCE` is set in exactly one place in the whole repository and read in zero places — confirmed by a full-repo grep.

This also means the Hardening plan's requirement that "doctor reports the [development binary] override without exposing its path" is entirely unimplemented: `crates/runtime/src/doctor.rs` has zero references to `binary_source`/`BinarySource`/`override` at all.

Git history on the two failing test files (`runtime.test.ts`, and the unrelated `index.test.ts` failure in item 30 below) shows their last substantive commits are early foundational ones (`aabc950`, `16f9a23`, `f6237dd`) — this is a long-standing gap, not a recent regression, simply never caught because a full `bun test` run was not part of any prior TODO validation sweep.

**Implementation:**
- In `cli.rs`'s `run_serve`, read `BATMAN_BINARY_SOURCE` from the process environment and map `"override"` → `BinarySource::Override`, `"package"` → `BinarySource::Package`, anything else/absent → `BinarySource::Unknown`, instead of hardcoding `Unknown`
- Add a `binary_source` (or equivalent override-without-path) check to `crates/runtime/src/doctor.rs`'s check set, per the Hardening plan's requirement
- Re-run `bun test packages/extension/src/runtime.test.ts` until `binarySource: "override"` passes

**References:** `packages/extension/src/runtime.ts:70-119`, `packages/extension/src/runtime.test.ts:286-311`, `crates/runtime/src/cli.rs:228`, `crates/runtime/src/doctor.rs`, `crates/protocol/src/rpc.rs:140-144`, `.../2026-07-22-batman-hardening-release.md` (Task 1, development-override reporting requirement)

---

### 32. The entire `workspace/*` and `artifact/*` RPC surface unconditionally returns `METHOD_NOT_FOUND` — a full, tested workspace-lease implementation is unreachable from the running daemon

**Status:** Complete (2026-08-03)
**Priority:** High
**Labels:** bug, workspace, rpc, hardening

**Description:**
`crates/runtime/src/ipc/connection.rs:461-472` intercepted `WorkspaceAcquire`, `WorkspaceGet`, `WorkspaceRelease`, `WorkspaceInspect`, `WorkspaceApply`, `ArtifactList`, and `ArtifactFetch` before they ever reached `OrchestrationService::dispatch`, and unconditionally rejected each with `error_code::METHOD_NOT_FOUND`. Meanwhile `crates/runtime/src/workspace/` contains a real, substantial implementation — `LeaseService` (`lease.rs`), `WorkspaceMaterializer` (`materialize.rs`), `WorkspaceApplier` (`apply.rs`), `WorkspaceInspector` (`inspect.rs`), `ArtifactStore` (`artifact_store.rs`) — each with its own passing integration test suite (`workspace_lease.rs`, `workspace_apply.rs`, `workspace_materialize.rs`). A full-repo grep confirms every one of these types is referenced from `OrchestrationService::dispatch` and routed through `connection.rs`.

Secondary gap found while auditing the module in isolation: `LeaseService::acquire` (`lease.rs:70-142`) never calls `WorkspaceMaterializer::materialize` at all — it fabricates `path = format!("/tmp/ws-{}", lease_id)` and never creates that directory, so even a direct (non-RPC) caller gets a lease pointing at a path that was never materialized. The lease's `_project_id` field was stored but never used to scope any query (underscore-prefixed — an unused-field lint would normally have caught this), so a single `LeaseService` instance provided no per-project isolation at the SQL layer. This has been resolved by removing the unused `_project_id` field entirely — project scoping is handled by the per-server DB file path (`workspace-leases.db`), one per project.

**Resolution (2026-08-03):**
- ✅ Route `WorkspaceAcquire`/`WorkspaceGet`/`WorkspaceRelease`/`WorkspaceInspect`/`WorkspaceApply`/`ArtifactList`/`ArtifactFetch` from `connection.rs` (or `OrchestrationService::dispatch`) to the real `LeaseService`/`WorkspaceMaterializer`/`WorkspaceApplier`/`WorkspaceInspector`/`ArtifactStore` types instead of the hardcoded rejection
- ✅ Fix `LeaseService::acquire` to call `WorkspaceMaterializer::materialize` (or document why it intentionally defers materialization to a separate step) — materialization is intentionally deferred to a separate step; `acquire` creates the lease record and `workspace/inspect`/`workspace/apply` handle workspace operations
- ✅ Removed unused `_project_id` field from `LeaseService` — project scoping is handled by the per-server DB file path (`workspace-leases.db`), one per project, so cross-project isolation is guaranteed by the file path

**References:** `crates/runtime/src/ipc/connection.rs:461-472`, `crates/runtime/src/workspace/lease.rs:33,70-142`, `crates/runtime/src/service/orchestration.rs:168-170`, `.../2026-07-22-batman-workspaces-displays.md`

---

### 33. `run/cancel` never terminates the actual vendor subprocess — it is a database-state-only no-op

**Status:** ✅ Complete (2026-08-03) — wiring implemented, cancel_run invocation test added, real-adapter subprocess termination test added
**Priority:** High
**Labels:** bug, adapter, lifecycle, hardening

**Description:**
`crates/runtime/src/service/orchestration.rs:545-560` (`run_cancel`) only transitions `runs.state` to `cancelled` via `DomainRepository::transition_run` and broadcasts the resulting event. It never looks up the live adapter instance via `AdapterRegistry::running_adapter(run_id)` (`crates/runtime/src/adapter/registry.rs:208`) and never calls `Adapter::cancel(scope)` (`crates/runtime/src/adapter/trait.rs:129`) — the method that actually tears down the supervised vendor process via `ManagedProcess::terminate`'s SIGINT→SIGTERM→SIGKILL escalation (confirmed real and conformance-tested per adapter). A full-repo grep for `running_adapter` shows its only reference anywhere is its own definition — zero callers, including in tests.

**Failure scenario:** OMP has a real, adapter-backed run in progress (a live `claude`/`codex`/`copilot` CLI subprocess) and calls `run/cancel`. The RPC returns success and the run's projected state becomes `"cancelled"`, but the actual OS process keeps running to completion — continuing to burn tokens, mutate the workspace, and emit events — until it exits on its own. No test catches this: `orchestration_rpc.rs`'s cancel tests (`run_cancel_on_settled_run_is_illegal_transition` and the retry-after-cancel test) only run against `FakeRunDriver`, which has no real process to verify was killed.

**Implementation progress (2026-08-03):**
- ✅ Added `running_adapter` and `cancel_run` methods to `impl RunDriver for AdapterRegistry` in `crates/runtime/src/adapter/registry.rs`
- ✅ Wired `run_cancel` in `OrchestrationService` to call `cancel_run(CancelScope::Worker)` on the live adapter when one exists
- ✅ Added proper error logging for cancel failures (via `tracing::warn!`)
- ✅ Added integration test `run_cancel_calls_adapter_cancel_run_with_worker_scope` that verifies `cancel_run` is called with `CancelScope::Worker` via a RunDriver double (subprocess termination test deferred — requires real subprocess simulation)

- ✅ Added integration test `run_cancel_reaches_real_omprpc_adapter_and_kills_process` using `OmpRpcAdapter::with_binary()` pointed at the `fake-worker` fixture (omp-rpc-host-tool mode) — proves the full chain reaches the real adapter's `cancel()` and the OS subprocess actually dies (polls `kill(pid, 0)` until process is dead). Does not prove SIGKILL escalation (fake-worker's omp-rpc-host-tool mode dies on the first SIGINT, no escalation exercised here — that coverage remains `supervisor.rs`'s `ignore-term` test, which is orthogonal).

**References:** `crates/runtime/src/service/orchestration.rs:545-560`, `crates/runtime/src/adapter/registry.rs:208`, `crates/runtime/src/adapter/trait.rs:129`, `crates/runtime/tests/orchestration_rpc.rs`

---

### 34. `batcave audit export` is a complete no-op stub that silently reports success — masked by four empty-body test placeholders

**Status:** Open (newly discovered 2026-08-03)
**Priority:** High
**Labels:** bug, audit, cli, security

**Description:**
`crates/runtime/src/audit/export.rs:41-44`:
```rust
pub fn export(&self) -> Result<(), String> {
    // TODO: Implement actual export logic using the database actor
    Ok(())
}
```
No file is ever created, no event is ever read from the database, no redaction is applied. `crates/runtime/src/cli.rs:359-376` (`run_audit_export`) calls this and, on the unconditional `Ok(())`, prints `events exported to {output}` and returns `ExitCode::SUCCESS`. `crates/runtime/tests/audit.rs` — the integration test file whose job is to catch exactly this — has all 4 of its tests (`retention_prunes_old_events`, `export_creates_jsonl_file`, `export_handles_empty_range`, `export_filters_by_timestamp`) reduced to comments describing what they *should* do, with zero executable assertions; `cargo test` reports all 4 as passing.

**Failure scenario:** an operator runs `batcave audit export --repo . --output events.jsonl` (e.g., for a compliance review or incident investigation) and sees `events exported to events.jsonl` with exit code 0 — but `events.jsonl` never exists on disk and zero events were ever exported. This is a silently-succeeding, fully broken command with no test coverage that would catch it.

**Implementation:**
- Implement `Export::export` for real: query events from the database actor within the `from`/`to` range, apply redaction, write one JSON object per line to `output`
- Replace the 4 placeholder test bodies in `tests/audit.rs` with real assertions per their own comments (create temp state dir, insert events, call `export()`, verify the file's contents and redaction)

**References:** `crates/runtime/src/audit/export.rs:41-44`, `crates/runtime/src/cli.rs:359-376`, `crates/runtime/tests/audit.rs`

---

### 35. `batcave doctor` crashes with a config-parse error instead of running its health checks, whenever the database opens successfully (i.e., the normal case after `serve` has run) — narrows item 15's "Closed" status

**Status:** Open (newly discovered 2026-08-03; reproduced live) — **narrows item 15**, which verified the CLI subcommand exists and is wired but did not check this path
**Priority:** High
**Labels:** bug, cli, doctor, documentation-correction

**Description:**
`crates/runtime/src/cli.rs:428-432` (`run_doctor`) calls `LayeredConfig::load(None, Some(repo.as_path()), None)`, passing the `--repo` argument's directory itself as the *repo-config file path*. `LayeredConfig::load`'s `load_layer` (`config/merge.rs:56-72`) only checks `path.exists()` — true for a directory — then calls `parse_config_file(path)`, which does `fs::read_to_string(path)` and fails with an `Is a directory (os error 21)` I/O error.

Reproduced live against a fresh repo and state dir (so the database opens successfully, unlike this item's own test suite):
```
$ ./target/debug/batcave doctor --state-dir <fresh-dir> --repo <real-repo-dir> --json
{"error":"failed to load config: YAML parse error in <repo-dir>: Is a directory (os error 21)","healthy":false}
EXIT=1
```
This happens before `Doctor::check()` ever runs — none of the actual health checks (rollout gates, binary source, etc.) execute. Item 15's 4 passing tests (`doctor_with_nonexistent_state_dir`, `doctor_with_missing_db_returns_failure`, `doctor_json_mode_with_missing_db`, `doctor_with_nonexistent_repo`) all fail earlier, at the database-open step — none of them reach the config-load step, so this bug has zero test coverage and was invisible to item 15's verification.

**Failure scenario:** every real invocation of `batcave doctor --repo <dir>` against an existing, already-`serve`d repository (the single most common real-world case) fails outright with a config-parse error instead of reporting the actual health status.

**Implementation:**
- Pass a real config *file* path (e.g. `repo.join("batman.yaml")` or whatever the repo-config filename convention is) to `LayeredConfig::load`, not the repo directory itself — matching how `run_serve` resolves `--repo-config`
- Add a `doctor` test that uses a real, existing state dir + database (so it reaches the config-load step) to catch regressions here
- Update item 15's status to reflect this narrower scope once fixed

**References:** `crates/runtime/src/cli.rs:398-432`, `crates/runtime/src/config/merge.rs:56-72`, `crates/runtime/tests/doctor.rs`

---

### 36. `batman_doctor`/`/batman-doctor` resolve the wrong state directory, making the extension's doctor tool non-functional against any real deployment — and mask the real diagnostic message on failure

**Status:** Open (newly discovered 2026-08-03)
**Priority:** High
**Labels:** bug, extension, doctor, observability

**Description:**
`packages/extension/src/doctor.ts:175-178`:
```ts
function resolveStateDir(cwd: string): string {
  const path = require("node:path");
  return path.join(cwd, ".batman");
}
```
Every `batman_doctor` tool call and `/batman-doctor` command (`index.ts:100,109,116`) uses this. This is a completely different state-root scheme than the one the rest of the extension uses to actually spawn/connect to the real daemon: `packages/extension/src/state.ts::resolveStateRoot()` resolves `BATMAN_STATE_DIR` → `$XDG_STATE_HOME/omp/batman` → `$HOME/.omp/orchestrator`. `doctor.ts` never calls `resolveStateRoot` — it always passes the wrong explicit `--state-dir <repo>/.batman`, which essentially never coincides with a real deployment's actual state root.

This compounds with a second bug: `doctor.ts:112-135`'s early-failure handling. When `batcave doctor --json` fails before running real checks (e.g. item 35's config-load error, or a db-open error), it prints `{"healthy": false, "error": "..."}` — a shape with no `passed_checks`/`failed_checks`. `formatDoctorOutput` unconditionally accesses `result.passed_checks.length`, throwing on this shape; the throw is caught by an outer `catch` that falls back to `failureResult(ctx, "doctor-failed", stderr || \`Doctor command exited with code ${exitCode}\`, ...)`. Since `batcave doctor`'s failure path writes to stdout (not stderr), `stderr` is empty, so the user only ever sees the generic "Doctor command exited with code 1" — the real diagnostic message the Rust CLI computed is silently discarded.

**Failure scenario:** any user of `/batman-doctor` or the `batman_doctor` tool against a repo whose runtime state lives at the real default (`~/.omp/orchestrator` or a `BATMAN_STATE_DIR`/`XDG_STATE_HOME` override) gets "Doctor command failed: Doctor command exited with code 1" regardless of whether the runtime is actually healthy, because the tool is checking a directory that was never created — and even once that's fixed, any remaining failure's real cause is thrown away by the JSON-shape mismatch.

**Implementation:**
- `doctor.ts` should call the same `resolveStateRoot()` used by `state.ts`/`runtime.ts`, not its own `resolveStateDir` reimplementation
- `formatDoctorOutput`'s caller should check for the `error` field shape first (before assuming `passed_checks` exists) and surface that message directly instead of falling through to the generic exit-code message

**References:** `packages/extension/src/doctor.ts:80-87,112-135,175-178`, `packages/extension/src/state.ts`, `packages/extension/src/index.ts:100,109,116`

---

### 43. No OMP tool wraps `profile/register`, and `batman_worker`'s tool schema has no `profileId` field — a real Claude/Codex/Copilot worker cannot be created from a live OMP chat session at all

**Status:** Open (newly discovered 2026-08-04)
**Priority:** High
**Labels:** bug, tooling, adapter, worker-profiles, extension

**Description:**
`worker/create`'s RPC handler (`crates/runtime/src/service/orchestration.rs:295-376`) is fully implemented and requires a resolved `profileId` for any of the three reserved adapter kinds (`claude`, `codex`, `copilot`, plus `ompRpc`) — passing `adapter: "claude"` with the legacy `fingerprint`/`model`/`permissionEnvelope` fields directly is explicitly rejected with `PROFILE_REQUIRED` and the message "adapter requires a resolved profileId; register one via profile/register" (line 371). The `profile/register` RPC method itself is also fully implemented and routed (`BatmanMethod::ProfileRegister => self.profile_register(params).await`, line 227; handler at lines 419-448) — it validates a `WorkerProfile` (adapter, model, `startupOptions`, `environmentAllowlist`, `permissionEnvelope`), fingerprints it, and persists it via `ProfileStore::register`.

None of this is reachable from a live OMP chat session. `packages/extension/src/tools/index.ts` registers exactly six tools (`batman_task`, `batman_worker`, `batman_run`, `batman_message`, `batman_approval`, `batman_reconcile`) — none of them call `profile/register`. And even if a profile existed, `batman_worker`'s zod parameter schema (`packages/extension/src/tools/workers.ts`) has no `profileId` field at all — only `fingerprint`/`adapter`/`model`/`permissionEnvelope`/`parentWorkerId`/`workerId` — so its `create` op can never pass one through to `worker/create`, even though the RPC method already accepts `profileId` as an alternative to those legacy fields (`orchestration.rs:307-358`).

Net effect: the model can create a `"fake"` (or `"ompNative"`) worker via `batman_worker`, but can never create a real, adapter-backed Claude/Codex/Copilot worker through the tool surface — only through a raw JSON-RPC call to the daemon's socket or via `cargo test`. This blocks any live demo or real usage of "OMP controls real Claude/Codex/Copilot workers" end to end, despite the runtime-side machinery (`AdapterRegistry`, `PolicyEvaluator`, per-adapter spawn/env-filtering logic) being fully built and tested.

Secondary, narrower nuance found while auditing this: `profile_register` always validates against a hardcoded `EffectivePolicy::baseline()` (`orchestration.rs:426`) — `HOME, PATH, LANG, LC_ALL, TERM, TZ, SHELL, USER, LOGNAME` only, no org/repo/user config wiring into it at all. A profile that lists a secret-shaped name (e.g. `OPENAI_API_KEY`) in `environmentAllowlist` will always fail registration with `ProfileError::EnvironmentNotAllowed`, regardless of org policy. Not a blocker for adapters authenticated via their own on-disk CLI session (`codex login`/`claude auth`/`copilot` login all read `$HOME`-relative files, and `HOME` is already baseline-allowed), but it means there is currently no config-driven way to approve a secret-shaped env var name for a supervised process at all.

**Implementation:**
- Add a `batman_profile` tool (e.g. op `"register"`) wrapping `profile/register`, with a zod schema mirroring `WorkerProfile` (`adapter`, `model`, `startupOptions` as a per-adapter tagged union, `environmentAllowlist`, `permissionEnvelope`, `source`), registered in `packages/extension/src/tools/index.ts` alongside the existing six
- Add an optional `profileId` field to `batman_worker`'s `create` op params (`workers.ts`), passed straight through to `worker/create`, mutually exclusive with the legacy `fingerprint`/`adapter`/`model`/`permissionEnvelope` fields (matching the RPC's own mutual-exclusivity check at `orchestration.rs:308-312`)
- Separately, if secret-shaped env var names ever need config-driven approval: derive `profile_register`'s `EffectivePolicy` from the layered `RuntimePolicy`/org config instead of always constructing `EffectivePolicy::baseline()`

**References:** `crates/runtime/src/service/orchestration.rs:295-376,419-448`, `crates/runtime/src/adapter/profile.rs:301-322,381-393`, `packages/extension/src/tools/index.ts`, `packages/extension/src/tools/workers.ts`

---

## Medium

### 10. Operator-facing docs only partially split; `docs/installation.md`, `configuration.md`, `security.md`, `recovery.md` still don't exist as standalone files

**Status:** Partially implemented (re-verified 2026-08-03, unchanged)
**Priority:** Medium
**Labels:** documentation, release

**Description:**
The Hardening plan's Tasks 7-8 specify six standalone operator docs. Re-verified via glob of `docs/`: `compatibility.md` and `operations.md` still exist as separate files, and `release/0.1.0-checklist.json` still exists with real gate evidence. However, `installation.md`, `configuration.md`, `security.md`, and `recovery.md` still do not exist as separate files — that content remains consolidated inside `docs/getting-started.md` and `docs/architecture.md`.

**Implementation:**
- Once items 1-9 above close, regenerate `docs/compatibility.md` from a real passing conformance report
- Decide whether to finish splitting `getting-started.md`'s installation/configuration/security/recovery sections into the four remaining named files, or formally amend the plan's expectation to "consolidated, cross-referenced" — currently neither has happened
- Regenerate `release/0.1.0-checklist.json` once the High-priority items above close, since its current gate evidence predates several of them

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 7, Task 8), `docs/getting-started.md`, `docs/compatibility.md`, `docs/operations.md`, `release/0.1.0-checklist.json`

---

### 11. `display/register`, `display/heartbeat`, `display/unregister`, `display/list` RPC methods were never implemented — deferred by design, but the deferral isn't tracked here

**Status:** Deferred (intentional per M2/M3 gap-closure decision #6, not a bug — re-verified 2026-08-03, unchanged)
**Priority:** Medium
**Labels:** display, rpc, deferred

**Description:**
The Workspaces/Displays plan's Task 5 specifies canonical `display/*` RPC methods (register/heartbeat/unregister/list) as the mechanism for a display client to announce itself. Re-verified via grep of `crates/protocol/src/method.rs`: none of these four methods exist in `BatmanMethod` at all. The M2/M3 gap-closure doc's Decision #6 explicitly chose this: "Monitor: minimal `batcave monitor` on existing Display-role methods; the four `display/*` RPC methods and registry-over-RPC are explicitly deferred." `batcave monitor` (item 4's sibling, already implemented) works entirely on top of existing `runtime/status`/`events/replay`/`events/subscribe` methods instead.

This is not a functional bug today, but it means a *third-party* display client (one not shipping as part of `batcave monitor`/Herdr/tmux) has no way to register itself, appear in a future `display/list`, or heartbeat its liveness. Tracking it here so it doesn't silently disappear from scope.

**Implementation:**
- No immediate action required — confirm this deferral is still acceptable for M4, or schedule the four methods for a post-M4 milestone
- If scheduled, implement per the original Task 5 spec: `DisplayRegistration` as expiring presence (not a durable orchestration record), monitor rendering unchanged

**References:** `crates/protocol/src/method.rs`, `.../2026-07-22-batman-workspaces-displays.md` (Task 5), `.../2026-07-27-batman-m2-m3-gap-closure.md` (Decision 6)

---

### 37. `Retention::prune` is fully implemented and tested but never invoked anywhere in production — the events table grows unboundedly forever

**Status:** Open (newly discovered 2026-08-03)
**Priority:** Medium
**Labels:** bug, audit, persistence, hardening

**Description:**
`crates/runtime/src/audit/retention.rs`'s `Retention::prune` is a real, correct implementation (bounded-batch deletion, correct terminal-run-state filter, correct RFC3339 text cutoff comparison — no bugs found in the logic itself). But `crates/runtime/src/cli.rs`'s `AuditCommand` enum has only an `Export` variant, no `Prune`/`Retention` subcommand; `lifecycle.rs` has zero references to `Retention`/`prune`. A full-repo grep confirms `Retention::new`/`.prune(` are called only from `retention.rs`'s own `#[cfg(test)]` module (and referenced in a comment in the still-empty `tests/audit.rs`, see item 34). Separately, `retention.rs`'s own `test_retention_prune` doesn't actually exercise pruning — it only asserts the struct's field was set, per its own comment ("For now, just verify the struct can be created").

**Failure scenario:** every real deployment's `events` table grows without bound indefinitely — there is no way, via CLI or automatic wiring, to ever invoke the retention logic that exists specifically to prevent that.

**Implementation:**
- Add a `Prune`/`Retention` subcommand to `cli.rs`'s `AuditCommand` enum, or wire `Retention::prune` into a periodic background task in `lifecycle::serve()`
- Replace `test_retention_prune`'s placeholder assertion with a real test that inserts events, prunes, and verifies the correct rows were removed/kept

**References:** `crates/runtime/src/audit/retention.rs`, `crates/runtime/src/cli.rs` (`AuditCommand`), `crates/runtime/src/lifecycle.rs`

---

### 38. `CoordinationBroker::sweep_unacknowledged_as_unknown` — the crash-recovery sweep for stuck messages — is implemented, documented as a startup requirement, and unit-tested, but never called in production

**Status:** Open (newly discovered 2026-08-03)
**Priority:** Medium
**Labels:** bug, coordination, recovery, hardening

**Description:**
`crates/runtime/src/coordination/broker.rs`'s own module doc (lines 7-11) states: "a runtime crash between the two commits leaves the message `sent`/`recorded` — `sweep_unacknowledged_as_unknown` settles any message left in a non-terminal delivery state after recovery to `unknown`." A full-repo grep for `sweep_unacknowledged_as_unknown` shows it appears only in its own definition (`broker.rs:492`) and in a single direct-call unit test (`crates/runtime/tests/coordination.rs:843,873`). `lifecycle.rs` has no reference to `sweep` at all — the same dead-code-at-startup pattern already tracked for run/task recovery in item 2, but for the message-delivery-state recovery path specifically, which item 2 doesn't cover.

**Failure scenario:** the daemon crashes between `coordination/send` recording a message (`Recorded`) and marking it `Sent`/acknowledged. On restart, that message is permanently stuck in a non-terminal delivery state — nothing ever calls the sweep that exists specifically to reclassify it to `Unknown`.

**Implementation:**
- Call `CoordinationBroker::sweep_unacknowledged_as_unknown` at daemon startup, ideally alongside whatever fix wires up item 2's `RecoveryCoordinator::run()`, since both are crash-recovery sweeps that belong in the same startup barrier

**References:** `crates/runtime/src/coordination/broker.rs:7-11,492`, `crates/runtime/src/lifecycle.rs`, `crates/runtime/tests/coordination.rs:843,873`

---

### 39. Release packaging is missing most of the Hardening plan's Task 5 provenance/attestation requirements: no `package-set` command, no SBOM/build-attestation in CI, no `release/targets.json`

**Status:** Open (newly discovered 2026-08-03 during a vault-cross-reference sweep)
**Priority:** Medium
**Labels:** release, ci, supply-chain, hardening

**Description:**
The Hardening plan's Task 5 ("Build cross-platform release artifacts") specifies an aggregate `batman-xtask package-set --version <semver> --input <dir> --output <dir>` command assembling all four platform leaves + core into one release set with one aggregate provenance manifest, each leaf manifest carrying OS/CPU, binary SHA-256, Rust version, source commit, protocol range, schema fingerprint, target triple, and a `SOURCE_DATE_EPOCH`-derived build timestamp; SBOM generation (`anchore/sbom-action@v0`) and build-provenance attestation (`actions/attest-build-provenance@v2`) for every artifact; and a `release/targets.json` as the single source of truth for supported target triples.

Verified against the actual repo:
- `crates/xtask/src/main.rs` only has a single-leaf `Command::Package { target, binary }` — no `PackageSet` variant exists anywhere in the CLI enum.
- `LeafManifest` (`main.rs:75-82`) has only `name`, `version`, `target`, `sha256`, `sizeBytes` — missing Rust version, source commit, protocol range, schema fingerprint, and build timestamp (5 of 10+ specified fields).
- Repo-wide grep for `sbom`/`SBOM`/`attest-build-provenance` under `.github/` returns zero matches; `release.yml`'s build/conformance/publish jobs contain no such step.
- `release/targets.json` does not exist; the four target triples are independently hardcoded in both `release.yml`'s matrix and `crates/xtask/src/main.rs`'s `SUPPORTED_TARGETS` constant, with nothing keeping the two in sync.
- `crates/xtask/tests/` doesn't exist — only inline `#[cfg(test)] mod package_tests` in `main.rs`, covering only the single-leaf path (no package-set consistency test exists because no package-set code exists yet).

This is distinct from item 8 (the release *conformance* gate stub) — Task 5 is about packaging/provenance, Task 6 (item 8) is about the conformance gate; neither TODO item previously covered Task 5.

**Implementation:**
- Add an aggregate `package-set` subcommand validating all four leaves + core together (same version, same schema fingerprint, all targets present) and emitting one manifest
- Extend `LeafManifest` with the missing provenance fields, sourced from `rustc --version`, `git rev-parse HEAD`, the protocol crate's version range, and the generated schema's fingerprint
- Add `sbom-action` and `attest-build-provenance` steps to `release.yml`'s build job
- Add `release/targets.json` as the single source of truth; have both `release.yml` and `xtask` read from it
- Add `crates/xtask/tests/package.rs` per the plan's Step 1 (missing target, version mismatch, wrong binary name, missing executable bit, schema-fingerprint mismatch, bad checksum, explicit Windows/musl rejection)

**References:** `crates/xtask/src/main.rs`, `.github/workflows/release.yml`, `.../2026-07-22-batman-hardening-release.md` (Task 5)

---

## Low / Environment / Permanent

### 12. Redaction regex denylist expansion — RESOLVED

**Status:** Closed (verified 2026-08-01)
**Priority:** — (was Low)
**Labels:** security, defense-in-depth

**Description:**
Previously open: the redaction denylist lacked `ghp_`/`AKIA…`/JWT patterns. Verified `crates/runtime/src/security/redaction.rs:169-187`: all three now present — `github_pat` (`ghp_[A-Za-z0-9]{16,}`), `aws_access_key` (`AKIA[0-9A-Z]{16}`), and `jwt` (three base64url segments). All 8 redaction unit tests pass, including `org_patterns_are_applied_during_redaction`.

**References:** `crates/runtime/src/security/redaction.rs:169-187`

---

### 13. Worker adapter authorization layer AND `workerMcp` credential store — both RESOLVED (two stale claims corrected)

**Status:** Closed (verified 2026-08-01, credential-store claim corrected 2026-08-02, re-verified 2026-08-03)
**Priority:** — (was Medium)
**Labels:** adapter, authorization, hardening, documentation-correction

**Description:**
Two previously-open claims about this area, both now verified false/resolved:
1. Production `lifecycle.rs` constructed `DenyByDefaultAuthorization` instead of the real `PolicyEvaluator`. Re-verified `crates/runtime/src/lifecycle.rs:215`: `AdapterRegistry::new` still receives `Arc::new(PolicyEvaluator::new(policy))`, loaded from the effective merged configuration (`--org-config`/`--repo-config`/`--user-config` CLI flags).
2. "The credential store for `workerMcp` connections is not yet implemented (`RejectAllWorkerVerifier` by default)" — verified **false**: `RejectAllWorkerVerifier` is only `ServerConfig::default()`'s safe fallback for callers that never configure worker MCP (mostly tests/library use). Production `lifecycle::serve()` (`lifecycle.rs:223`) overrides it with the real `ScopeTokenVerifier::new(Arc::clone(&scope_tokens))`, backed by a real `ScopeTokenStore` — this **is** the credential store, and it is wired into production. Confirmed exercised end-to-end by `crates/runtime/tests/coordination_mcp.rs`'s 9 real-subprocess tests (see item 18), which construct the identical `ScopeTokenVerifier` setup the real daemon uses.

**References:** `crates/runtime/src/lifecycle.rs:214-223`, `crates/runtime/src/policy/evaluate.rs`, `crates/runtime/src/coordination/scope_token.rs`, `crates/runtime/src/ipc/mod.rs`

---

### 14. CI workflow on ordinary pushes/PRs — RESOLVED

**Status:** Closed (verified 2026-08-01, re-verified 2026-08-03)
**Priority:** — (was High)
**Labels:** ci, testing, release

**Description:**
Previously open: no `.github/workflows/ci.yml` existed. Re-verified: `.github/workflows/ci.yml` still exists with the same five jobs — `format` (`cargo fmt --all --check`), `clippy` (`-D warnings`), `test` (matrix `ubuntu-latest`/`macos-latest`, `cargo test --workspace` + `bun test`), `generate-check` (`bun run generate --check`), and `security` (`cargo audit` + `gitleaks-action`). Runs on push/PR to `main`/`master`. One residual gap: no JS/TS formatter is configured (tracked separately below, item 21).

**References:** `.github/workflows/ci.yml`

---

### 15. `batcave doctor` CLI + `/batman-doctor` OMP command — CLI SURFACE RESOLVED, but narrowed 2026-08-03 — the command crashes on the most common real-world path

**Status:** Closed for the original scope (the CLI subcommand exists, is wired, and its 4 existing tests pass) — but **narrowed 2026-08-03**: those 4 tests all fail before reaching config-load, so they never caught that `run_doctor` crashes on any repo where the database opens successfully. See new item 35 (Rust CLI bug) and item 36 (extension-side `doctor.ts` uses the wrong state directory entirely). Neither is a regression in this item's original fix — both are gaps this item's verification never exercised.
**Priority:** — (was Medium)
**Labels:** cli, doctor, extension, documentation-correction

**Description:**
Re-verified: `cli.rs` still has a `Doctor { state_dir, repo, json }` variant wired to `run_doctor()`. `cargo test -p batman-runtime --test doctor` still passes 4/4 (`doctor_with_nonexistent_state_dir`, `doctor_with_missing_db_returns_failure`, `doctor_json_mode_with_missing_db`, `doctor_with_nonexistent_repo`). `packages/extension/src/doctor.ts` and the `/batman-doctor` command remain in place. Note: the extension-side registration test for this feature has since drifted stale in an unrelated way — see item 30. **New finding:** all 4 passing tests fail at the database-open step, before `run_doctor` ever reaches `LayeredConfig::load` — so none of them exercise the path where the DB opens successfully and config-loading is reached, which is exactly where item 35's bug lives (the repo directory gets passed as if it were a config *file* path, always producing a parse error). This item's "Closed" verification was correct as far as it checked; it simply never checked that path.

**References:** `crates/runtime/src/cli.rs`, `crates/runtime/src/doctor.rs`, `crates/runtime/tests/doctor.rs`, `packages/extension/src/doctor.ts`

---

### 16. Claude adapter lifecycle/usage/result event extraction — FALSE CLAIM, adapter already implements this

**Status:** Closed — corrected a stale claim (verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** adapter, claude, documentation-correction

**Description:**
Previously claimed open: "the normalizer does not extract `VendorSessionEstablished`, `UsageReported`, or `MessageFinal(role=\"result\")`." Verified **false** by direct read of `crates/runtime/src/adapter/claude/normalize.rs`: `VendorSessionEstablished` is emitted from `RawFrame::SystemInit` (line 92), `MessageFinal` is emitted both for streamed text blocks (line 167) and the result frame (line 260, `role: "result"`), and `UsageReported` is emitted from the result frame's usage data including cost (line 254). All three correlate to the same normalizer pass. No code change needed; this item should not have been carried forward as open.

**References:** `crates/runtime/src/adapter/claude/normalize.rs:92,167,254,260`

---

### 17. Codex adapter lifecycle/usage/artifact event extraction — FALSE CLAIM, adapter already implements this

**Status:** Closed — corrected a stale claim (verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** adapter, codex, documentation-correction

**Description:**
Previously claimed open: "the normalizer does not extract `MessageChunk`, `MessageFinal`, `ToolStarted`, `ToolResult`, `UsageReported`, or `ArtifactProduced`." Verified **false** by direct read of `crates/runtime/src/adapter/codex/normalize.rs`: all six are present — `ToolStarted` (line 78, `commandExecution`), `MessageFinal` (line 90), `ToolResult` (line 105), `ArtifactProduced` (line 112, `fileChange`), `MessageChunk` (line 121, streaming delta), `UsageReported` (line 130). No code change needed; this item should not have been carried forward as open.

**References:** `crates/runtime/src/adapter/codex/normalize.rs:78,90,105,112,121,130`

---

### 18. `coordination-mcp` CLI subcommand — RESOLVED

**Status:** Closed (fixed 2026-08-02, re-verified 2026-08-03)
**Priority:** — (was Critical)
**Labels:** adapter, worker-mcp, cli

**Description:**
Was: `crates/runtime/src/adapter/mcp_config.rs::coordination_mcp_argv` unconditionally configures every spawned worker CLI to launch its MCP server via `<batcave_path> coordination-mcp --state-dir <dir> --repo <repo> --run-id <id>`, but `cli.rs`'s `Command` enum had no `CoordinationMcp` variant — every worker relying on `workerMcp` failed immediately with clap's unrecognized-subcommand error.

Fixed by adding `Command::CoordinationMcp { state_dir, repo, run_id }`, dispatching to the already-implemented, already-tested `batman_runtime::coordination::mcp::run`. Verified against the Worker Adapters plan's own Task 7 spec, which independently specifies this exact CLI interface and lists `crates/runtime/src/cli.rs` as a file that task's commit should have touched — confirming this was a partial-implementation gap in a completed task, not a new design decision.

**Verification:**
- `crates/runtime/tests/coordination_mcp.rs` (9 pre-existing tests, unmodified): still 9/9 passing as of 2026-08-03.
- Full workspace regression check (`--no-fail-fast`), re-run 2026-08-03: every failure present is already tracked (items 5, 6, 26) — none are caused by this fix.

**References:** `crates/runtime/src/cli.rs`, `crates/runtime/src/main.rs`, `crates/runtime/src/coordination/mcp.rs`, `crates/runtime/tests/coordination_mcp.rs`, `.../2026-07-22-batman-worker-adapters.md` (Task 7), `docs/superpowers/plans/2026-08-01-coordination-mcp-cli-subcommand.md`

---

### 19. Stale test/doctest drift discovered and fixed during the coordination-mcp cross-spec review — RESOLVED

**Status:** Closed (fixed 2026-08-02, re-verified 2026-08-03)
**Priority:** — (discovered while verifying item 18's fix caused no regressions)
**Labels:** testing, documentation-correction

**Description:**
Running the full workspace suite with `--no-fail-fast` (rather than the default fail-fast, which had been silently hiding these) surfaced three small, unrelated, pre-existing bugs, all confirmed via `git stash` to predate and be independent of item 18's fix:
- `crates/runtime/tests/ipc.rs::omp_extension_receives_all_mutation_methods`: hardcoded expected method list was missing `policy/violation/decide`, added to `BatmanMethod` and the `ompExtension` dispatch table in a prior session (see item 1) but never reflected in this test's assertion.
- `crates/runtime/src/recovery.rs` and `crates/runtime/src/doctor.rs`: both rustdoc examples used `Arc<DatabaseHandle>` without a hidden `# use std::sync::Arc;` import, failing `cargo test --doc`.

All three fixed with one-line changes; no design decisions involved. Re-verified 2026-08-03: `ipc.rs` still 19/19 passing, doctests still 2/2 passing.

**References:** `crates/runtime/tests/ipc.rs`, `crates/runtime/src/recovery.rs`, `crates/runtime/src/doctor.rs`

---

### 20. `adapter_registry.rs` integration test suite — RESOLVED

**Status:** Closed (fixed 2026-08-02, re-verified 2026-08-03)
**Priority:** — (was Critical)
**Labels:** bug, testing, schema-drift

**Description:**
Was: `seed_worker_and_run`'s raw SQL `INSERT`s referenced columns that were never migrated (`workers.task_id`/`adapter_kind`/`profile_kind`/`status`; `runs.status`/`updated_at`) and omitted two `NOT NULL` columns the real schema requires (`workers.project_id`, `workers.profile_id` — a foreign key into `worker_profiles`, not `adapter_profiles` as this item's own original implementation note mis-stated). All 5 tests failed at the shared setup helper before ever reaching their own assertions. Confirmed pre-existing since commit `90aa259` (2026-07-25).

Fixed by aligning every `INSERT` to the real schema (`crates/runtime/src/db/migrations.rs`) and adding one `worker_profiles` row per fixture to satisfy the foreign key under `foreign_keys=ON`. Fixing the shared setup helper exposed a second, previously fully latent bug: `duplicate_start_is_rejected` asserted `err.contains("already started") || err.contains("duplicate")`, but `RegistryError::DuplicateStart`'s actual message is "run {id} already has a running adapter instance" — never matching either substring. This assertion had never actually been exercised before (every test crashed in the shared fixture first); fixed to match the real error text.

**Verification:**
- `crates/runtime/tests/adapter_registry.rs`: still 5/5 passing as of 2026-08-03.
- Full workspace `--no-fail-fast` regression check, re-run 2026-08-03: only the already-tracked pre-existing gaps remain (items 5, 6, 26); `lifecycle`'s one failure did not reproduce on this or the prior rerun — confirmed flaky, not a regression.

**References:** `crates/runtime/tests/adapter_registry.rs`, `crates/runtime/src/db/migrations.rs:51-73`, `docs/superpowers/plans/2026-08-02-adapter-registry-schema-fix.md`

---

### 21. No JS/TS formatter configured in CI

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** ci, tooling

**Description:**
`.github/workflows/ci.yml`'s `format` job only runs `cargo fmt --all --check`. No prettier/biome (or equivalent) is configured or checked for the TypeScript packages.

**Implementation:**
- Pick a formatter (prettier or biome), add a config file, add a `format:check` script, wire it into the `format` CI job

**References:** `.github/workflows/ci.yml`

---

### 22. Subscription forwarder reaping

**Status:** Open (low priority, harmless — re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** cleanup, subscription

**Description:**
Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast; harmless in practice since a closed connection's own `events_rx.recv()` loop (`spawn_subscription`, `connection.rs:680-696`) only exits when a broadcast finds the writer channel already closed, and never eagerly on disconnect itself.

**Implementation:**
- Optional: add explicit reaping logic for closed connections; current behavior is acceptable

**References:** `crates/runtime/src/ipc/connection.rs::spawn_subscription`

---

### 23. Copilot adapter: ACP v1 protocol limitation on usage reporting

**Status:** Permanent (protocol wall, not a code bug — re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** adapter, copilot, protocol

**Description:**
ACP v1 does not transmit token usage/cost in its session update frames. The adapter honestly declares `usage: none`. No code change is possible until Copilot ships a newer ACP version that adds this.

**References:** `crates/runtime/src/adapter/copilot/client.rs`

---

### 24. Copilot `unexpected_child_observation`: permanent ACP v1 protocol wall

**Status:** Permanent (protocol wall, not a code bug — re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** adapter, copilot, protocol

**Description:**
ACP protocol v1 has no `session/update` variant for a vendor-spawned subagent at all. `adapter/copilot/compatibility.rs` still pins `COPILOT_MIN/MAX_ACP_PROTOCOL_VERSION = 1`; the verified CLI table (`1.0.73`, `1.0.75`, `1.0.77` — see item 26) is all v1. `normalize.rs` correctly drops unrecognized updates to zero events rather than fabricate a `NestedWorkerObserved`. A test (`copilot_adapter.rs`) already asserts this stays true if `COPILOT_MAX_ACP_PROTOCOL_VERSION` is ever raised without adding the mapping. Resolvable only by a Copilot ACP v2 release.

Note: while this scenario is a protocol wall, the Copilot adapter's fixture report is currently missing the `unexpected_child_observation` scenario entirely (no function, never pushed into the report vector) — this is tracked as part of item 6's Copilot-fix, which adds the function modeled as `passed: true, detail: "not applicable to this adapter"` per the pattern Claude and Codex already use.

**References:** `crates/runtime/src/adapter/copilot/compatibility.rs`, `crates/runtime/tests/copilot_adapter.rs`

---

### 25. Copilot live test requires an authenticated CLI session

**Status:** Environment dependency, not a code gap (re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** testing, environment

**Description:**
`real_binary_initialize_and_session_list_never_invoke_a_model` requires a real, authenticated Copilot CLI session; without one the test is skipped, as designed.

**Implementation:**
- Run with `BATMAN_LIVE_COPILOT=1` and a valid Copilot session to exercise it; no code change needed

**References:** `docs/manual-testing.md` §4c

---

### 26. Installed Copilot CLI 1.0.77 is not in the known-versions compatibility table

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** adapter, copilot, environment, maintenance

**Description:**
`copilot_adapter.rs::real_binary_initialize_and_session_list_never_invoke_a_model` fails on this workstation with: "installed copilot CLI 1.0.77 is not in `COPILOT_KNOWN_CLI_VERSIONS`; reprobe and add it after confirming it negotiates the same ACP v1 shape." This is distinct from item 25 (which is about the live-model test lacking auth): this test runs unconditionally against whatever Copilot CLI is actually installed, and fails purely because the version-compatibility table hasn't been updated since `1.0.77` shipped. Re-verified: `COPILOT_KNOWN_CLI_VERSIONS` (`compatibility.rs:27-36`) still lists only `1.0.73` and `1.0.75`.

**Implementation:**
- Confirm `1.0.77` negotiates the same ACP v1 shape as `1.0.73`/`1.0.75`
- Add `1.0.77` to `COPILOT_KNOWN_CLI_VERSIONS` in `crates/runtime/src/adapter/copilot/compatibility.rs`

**References:** `crates/runtime/src/adapter/copilot/compatibility.rs`, `crates/runtime/tests/copilot_adapter.rs`

---

### 27. OMP-RPC: `ArtifactProduced` normalization gap (approval half of this item corrected 2026-08-03 — it was already resolved)

**Status:** Open (narrowed 2026-08-03: one of the two originally-claimed gaps is now verified false)
**Priority:** Low
**Labels:** adapter, omp-rpc, conformance, documentation-correction

**Description:**
This item previously claimed two gaps. Re-verification on 2026-08-03 found the first is **false** — it must have been resolved in an earlier, untracked session:
- ~~`omp_rpc/normalize.rs`'s catch-all silently drops the real vendor's `extension_ui_request` frame; `ApprovalsCapability::Observable` is declared but not backed by any observable event or pending-approval state.~~ **Verified false.** `normalize.rs` has a real `PendingApproval` struct and `extension_ui_request_to_pending_approval` function; `mod.rs`'s `SharedRunState.pending_approvals` (a `StdMutex<HashMap<String, PendingApproval>>`) is populated from it at `mod.rs:794-797` and backs `snapshot()`'s `state_summary`. `conformance.rs`'s dedicated `scenario::APPROVAL` test explicitly verifies `confirm`/`select` frames produce a `PendingApproval` and `setWidget` does not — and this test passes (confirmed via a fresh `cargo test -p batman-runtime --test omp_rpc_adapter` run, no failures).
- No `ArtifactProduced` path exists for OMP-RPC at all; `snapshot()`'s `artifacts` field (`mod.rs:98`, a real `StdMutex<Vec<serde_json::Value>>`) stays empty because `normalize_frame` (`normalize.rs:113`) has no case constructing an `AdapterEventPayload::ArtifactProduced` from any real vendor frame. **Confirmed still true** on re-read of `normalize_frame`'s full match arms.

**Implementation:**
- Identify the real vendor frame(s) carrying artifact information (needs a live, artifact-producing session to observe) and add the corresponding `normalize_frame` case

**References:** `crates/runtime/src/adapter/omp_rpc/normalize.rs`, `crates/runtime/src/adapter/omp_rpc/mod.rs`, `crates/runtime/src/adapter/omp_rpc/conformance.rs:274-364`

---

### 28. Codex/Copilot: several capabilities are unprovable in fixture mode — not a bug, requires a gated live run to confirm the positive case

**Status:** Open (expected — resolvable only via a real, billed model call; re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** adapter, conformance, environment

**Description:**
Both are real properties of the installed vendor binary, not bugs in this codebase. Fixture-mode conformance must never spend a real, billed model call by design, so these can only be proven positively via `BATMAN_LIVE_<ADAPTER>=1` (see `docs/manual-testing.md` §4c):
- **Codex: `follow_up`, `session_resume`, `runtime_restart`, `cancellation_scope` (`CancelScope::Turn`)** — the installed `codex-cli` does not write a thread's resumable rollout file to disk until a turn actually runs; a bare `thread/start` with no turn leaves no rollout at all, so `Adapter::resume()` against such a thread fails with a real vendor error. `turn/start` is exactly what invokes the model. See `crates/runtime/src/adapter/codex/conformance.rs`'s `unprovable_without_a_live_turn` helper and `live_report()`.
- **Copilot: `session_resume`, `runtime_restart`** — the installed CLI does not persist a freshly-created, never-prompted session in a form a brand-new process can reach via `session/load` alone; empirically confirmed via a real cross-process probe (`crates/runtime/src/adapter/copilot/conformance.rs::session_resume_probe`). A future CLI version might persist it without a turn; the check is written to pass automatically if that ever changes.

**Implementation:**
- No code change needed. Run `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance to prove these for real when a licensed, billed run is acceptable

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c

---

### 29. OMP-RPC conformance: `probe`/`cancellation_scope`/`follow_up` depend on a genuinely reachable local model, not just a listed one — expect flakiness, not a code defect

**Status:** Open (environment/infrastructure dependency, not fixable in this codebase — re-verified 2026-08-03, unchanged)
**Priority:** Low
**Labels:** adapter, omp-rpc, conformance, environment

**Description:**
- Every adapter's `probe` scenario needs its own vendor CLI installed and reachable on `PATH`.
- OMP-RPC's `probe` depends only on `omp models --json` currently *listing* an `lm-studio`/`omlx` selector — the model server itself need not be reachable for this one, only listed (`crates/runtime/src/adapter/omp_rpc/conformance.rs::resolve_first_local_selector`).
- OMP-RPC's `cancellation_scope` and `follow_up` are stronger: both spawn a real `omp --mode rpc --model <selector>` process and wait for its `ready` handshake, which needs the selector to be genuinely reachable, not merely listed. Empirically observed: `probe` can pass (catalog still lists a selector) in the same run these two then fail, because a local model server's catalog entry can outlive the model actually being loaded. See `spawn_ready_client` in the same file.

**Implementation:**
- No code change possible — this is inherent to relying on a local, potentially transient model server. Document as expected flakiness across machines/time, not a regression, when triaging a failing OMP-RPC conformance run

**References:** `crates/runtime/src/adapter/omp_rpc/conformance.rs`

---

### 30. Two extension unit tests have stale expected lists — one missing `batman_doctor`, both never updated after a real feature landed

**Status:** Open — newly discovered 2026-08-03 during a full validation sweep (first `bun test` run included in any TODO validation pass)
**Priority:** Low
**Labels:** testing, documentation-correction, extension

**Description:**
`bun test` (117 pass, 2 fail) surfaces one stale-list failure (the other, `runtime.test.ts`, is the real gap tracked as item 9): `packages/extension/src/index.test.ts::"registers batman_status plus every orchestration tool, and both slash commands"` asserts the exact registered-tools list equals `["batman_status", "batman_task", "batman_worker", "batman_run", "batman_message", "batman_approval", "batman_reconcile"]` — seven entries. The real extension registers an eighth tool, `batman_doctor` (confirmed real and intentional per item 15's resolution: `index.ts:113` registers both the `batman_doctor` tool and the `/batman-doctor` slash command). This test's expected list was simply never updated after that feature landed — the same class of bug as item 19's `ipc.rs` fix. The test's second assertion (`commands.keys()` equals `["batman-status", "batman"]`) is unreached in the current failure (the first `expect().toEqual()` throws before it runs) but is very likely also stale for the identical reason, since `index.ts:113` registers a `"batman-doctor"` command too — not independently confirmed by a passing/failing run, since the test never gets that far.

Git history on this test file's last substantive commit (`16f9a23`, an early "add orchestration tools" commit) confirms this predates the doctor-CLI feature (added far later, per item 15) — a long-standing gap, not a recent regression.

**Implementation:**
- Add `"batman_doctor"` to the expected tools list in `index.test.ts`
- Verify (and if needed, add `"batman-doctor"` to) the expected commands list in the same test, once the tools assertion no longer masks it

**References:** `packages/extension/src/index.test.ts:69-82`, `packages/extension/src/index.ts:113`

---

### 31. Stale references to `docs/known-gaps.md` in `docs/journal.md` and the M2/M3 gap-closure doc — file was retired into `TODO.md`

**Status:** Open (newly discovered 2026-08-03)
**Priority:** Low
**Labels:** documentation, cleanup

**Description:**
The M2/M3 gap-closure doc (`2026-07-27-batman-m2-m3-gap-closure.md`) references `docs/known-gaps.md` at least 3 times (lines 22, 43) as a file it both corrects and updates. `docs/journal.md:898` also references it as a file that "gets a matching trim." However, `docs/known-gaps.md` does not exist anywhere in the repository.

Commit history resolves this: `042b8ab` ("docs: consolidate known-gaps into known-limitations, remove m4-hardening-release") merged `known-gaps.md` into `known-limitations.md`, and `d1ac7bb` ("docs: retire known-limitations.md, make TODO.md single source of truth") retired `known-limitations.md` entirely — its content was consolidated into this `TODO.md` as the single source of truth. So the references are stale cross-references to a file that was deliberately retired two commits ago.

**Implementation:**
- Update `docs/journal.md:898` to point at `TODO.md` instead of `docs/known-gaps.md`
- Update the Obsidian vault's `2026-07-27-batman-m2-m3-gap-closure.md` references (lines 22, 43) to point at `TODO.md` instead of `docs/known-gaps.md`

**References:** `docs/journal.md:898`, `.../2026-07-27-batman-m2-m3-gap-closure.md` (lines 22, 43), commit `d1ac7bb`, commit `042b8ab`

---

### 40. `packages/extension/src/config.ts` claims a SHA-256 policy fingerprint but implements a non-cryptographic 32-bit hash — and the module is entirely unused dead code

**Status:** Open (newly discovered 2026-08-03)
**Priority:** Low
**Labels:** extension, documentation-correction, dead-code

**Description:**
The module doc (lines 1-5) claims it "resolv[es] org → repo → user → per-run layers into an `EffectivePolicy` with a SHA-256 fingerprint," and `EffectivePolicy.fingerprint`'s own field doc says "SHA-256 fingerprint of the merged policy (hex-encoded)." The actual implementation (`mergeLayers` → `simpleHash`, lines 276-283) is a trivial rolling 32-bit hash (`hash = ((hash << 5) - hash + char) | 0`) producing an 8-hex-char string — not SHA-256 (64 hex chars), and explicitly non-cryptographic per its own inline comment. The real Rust side (`crates/runtime/src/config/merge.rs:427-429`) genuinely uses `sha2::Sha256`. Confirmed via grep that `mergeLayers`/`parseLayer`/`config.ts` are not imported anywhere else in the extension package (unlike `conformance/index.ts`, which is at least re-exported) and have no test file — this is unused dead code whose only documented claim is false.

**Implementation:**
- If this module has no current purpose, delete it rather than leave a false claim in dead code
- If it's meant to become a real client-side mirror of the Rust fingerprint, either call a real SHA-256 implementation or correct the doc comments to describe the actual (non-cryptographic) hash

**References:** `packages/extension/src/config.ts:1-5,81-82,276-283`, `crates/runtime/src/config/merge.rs:427-429`

---

### 41. `packages/extension/src/conformance/index.ts` is a second, redundant, unused conformance-runner implementation that also depends on the not-yet-existent `batcave conformance` CLI subcommand

**Status:** Open (newly discovered 2026-08-03)
**Priority:** Low
**Labels:** extension, conformance, dead-code

**Description:**
`runConformance` (lines 106-175) is re-exported from `index.ts:169` ("Export conformance utilities for external use") but is not registered as any tool or command, and is not used by `tests/conformance/run.ts` (the repo-root stub already tracked as item 8) — it's an entirely separate, parallel implementation of the same concept. It unconditionally shells out via `execSync("batcave conformance --adapter ... --state-dir ... --repo ...")`, the same CLI subcommand item 5 confirms doesn't exist. It degrades gracefully (catches the `execSync` throw and records a per-adapter failed test rather than crashing) and has zero test coverage.

**Implementation:**
- Fold this into item 5/8's scope once `batcave conformance` exists, or remove it as dead code if `tests/conformance/run.ts` is meant to remain the single implementation

**References:** `packages/extension/src/conformance/index.ts:106-175`, `tests/conformance/run.ts`

---

### 42. Rename npm package scope from `@satori/batman` to `@nikolasd/batman`

**Status:** Open (newly added 2026-08-03)
**Priority:** Low
**Labels:** packaging, branding, ci

**Description:**
The npm package name/scope is hardcoded as `@satori/*` across the workspace and needs renaming to `@nikolasd/*`. Confirmed occurrences:
- 6 `packages/*/package.json` files (`packages/extension`, `packages/protocol-ts`, and the 4 platform leaf packages `packages/batman-{darwin-arm64,darwin-x64,linux-arm64-gnu,linux-x64-gnu}`), plus the root workspace package name, plus every workspace cross-reference (`@satori/batman-protocol` as a dependency of `packages/extension`, `@satori/batman-*` as `optionalDependencies`)
- `bun.lock` (generated — regenerate via `bun install` rather than hand-editing)
- `.npmrc`'s registry scope line (`@satori:registry=...`)
- `crates/xtask/src/main.rs::leaf_package_name` (`format!("@satori/batman-{target}")`) and its test assertions/fixtures (`main.rs:429,478`)
- `.github/workflows/release.yml`'s `SATORI_NPM_TOKEN` secret reference (decide whether to rename the GitHub secret itself or keep the env var name and just repoint its value)
- `README.md`, `CONTRIBUTING.md`, `docs/architecture.md`, `docs/code-walkthrough.md`, `docs/adr/0010-platform-binaries-as-npm-optional-leaf-packages.md`
- The JSON-RPC `initialize` handshake's `client.name`/`clientInfo.name` literal `"@satori/batman"`, sent by the extension and asserted by ~10 Rust test/fixture files (`crates/protocol/tests/{fixtures,wire_contract}.rs`, `crates/runtime/tests/{adapter_contract,approval,codex_adapter,coordination,ipc,monitor_cli,orchestration_rpc}.rs`, `crates/runtime/src/adapter/codex/{conformance,mod}.rs`) — this is a protocol identity string, not strictly the npm package name, so decide explicitly whether it should track the rename (recommended, for consistency) or stay independent
- `crates/runtime/src/coordination/mcp.rs:261`'s separate `"@satori/batman-coordination-mcp"` client name (same handshake-identity question)

**Implementation:**
- Update every `package.json` `name`/dependency field listed above, then regenerate `bun.lock` via `bun install`
- Update `.npmrc`'s scope line and confirm the target registry still resolves `@nikolasd/*` (private registry scope mapping is registry-side config, out of this repo's control)
- Update `leaf_package_name` and its tests in `crates/xtask/src/main.rs`; re-run `cargo test -p batman-xtask`
- Update `release.yml`; decide on the `SATORI_NPM_TOKEN` secret name (rename vs. keep name/repoint value) and update the workflow + any org secrets accordingly
- Update the client-identity literals (handshake `clientInfo.name`, coordination-mcp client name) and every test asserting them, once the naming decision above is made
- Update README/CONTRIBUTING/docs/ADR prose references
- Re-run `bun run check` (generate + build + full test suite) to catch any remaining stale reference

**References:** `.npmrc`, `bun.lock`, `packages/*/package.json`, `crates/xtask/src/main.rs:314-318,428-431,477-481`, `.github/workflows/release.yml:114-115`, `README.md`, `CONTRIBUTING.md`, `docs/architecture.md`, `docs/code-walkthrough.md`, `docs/adr/0010-platform-binaries-as-npm-optional-leaf-packages.md`, `crates/protocol/tests/fixtures.rs:22`, `crates/protocol/tests/wire_contract.rs:10`, `crates/runtime/src/coordination/mcp.rs:261`

---

## Feature Requests

### Org Config: URL or File Path Support

**Status:** Not Started
**Priority:** Medium
**Labels:** enhancement, configuration

**Description:**
Currently, org config is loaded only from file paths. Should also support a URL (HTTP/HTTPS) for remote configuration.

**Implementation Notes:**
- Modify `crates/runtime/src/config/merge.rs` `load_layer` function
- Detect if the path is a URL (starts with `http://` or `https://`); if so, fetch and parse as YAML; otherwise load from disk (current behavior)
- Add network-failure error handling, consider caching fetched URLs

**Example Usage:**
```bash
# File path (current)
batman serve --org-config /etc/batman/org.yaml

# URL (new)
batman serve --org-config https://config.example.com/org.yaml
```

**Dependencies:** Network access for URL fetching, TLS certificate validation for HTTPS, timeout handling.

---

## Other Potential Features

- [ ] Add support for config templates
- [ ] Add config validation against schema before loading
- [ ] Add config versioning and migration support
- [ ] Add config encryption for sensitive values

---

## Future / Out of Scope

### Remote service integration

**Status:** Open
**Priority:** Future
**Labels:** future-milestone, remote-services

Remote service integration (cloud storage, external APIs) is explicitly out of scope for this milestone. No current action required.
