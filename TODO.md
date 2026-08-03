# BATMAN TODO

Every item below was verified against the current codebase (not inferred from prior docs). Superseded/false claims from earlier sessions are corrected inline. Priority order reflects what blocks core functionality first, then Hardening/release readiness, then polish. Last full re-verification pass: 2026-08-03 (full validation sweep of every open item, plus a fresh `bun test` run — not previously included in prior sweeps). No Critical-severity items remain open. Zero regressions found among previously-tracked items; one stale/false claim corrected (item 27); two new gaps discovered (items 9, 30).

---

## High — blocks Hardening (M4) readiness

### 1. Nested-worker policy violations are journaled but never quarantined, cancelled, or reported to OMP — `policy/violation/decide` is a stub

**Status:** Open (re-verified 2026-08-03, unchanged)
**Priority:** High
**Labels:** security, policy, hardening

**Description:**
The Hardening plan's Task 1 requires: on `NestedWorkerObserved` while the effective capability is `nested:none`, the runtime must atomically persist a `PolicyViolation`, set `policyQuarantined`, block messages/artifact publication/workspace apply, create an audited worker-cancellation intent, and notify the owning OMP client — resolvable only via `policy/violation/decide`, exposed to the owning `ompExtension` client as `batman_worker op:"resolvePolicyViolation"`.

Verified current state:
- `AdapterEventSink::build_runtime_event` (`event_sink.rs:288`) maps `NestedWorkerObserved` straight to `RuntimeEvent::AdapterNestedWorkerEvent` for the journal — no policy hook, no `PolicyViolation` record, no quarantine flag, no cancellation, no OMP notification.
- `PolicyEvaluator` (`policy/evaluate.rs`) only enforces nested-worker policy at **pre-authorization** time (`authorize()` rejects `is_nested && !self.allow_nested` before a worker starts) — this is a different mechanism from the plan's requirement, which is about a worker that is *already running* and then unexpectedly reports a child mid-run.
- `nested_violation_action` (the config knob controlling `quarantine`/`cancel`/`quarantineAndCancel`) appears exactly once in the entire runtime crate — a hardcoded default in `evaluate.rs:270` — with no consumer.
- `OrchestrationService::dispatch` explicitly stubs the method: `BatmanMethod::PolicyViolationDecide => Err(ServiceError::internal("method is not routed through OrchestrationService"))` (`orchestration.rs:165-167`). `OrchestrationService` has no `policy` field and no `decide_violation` function. The method IS registered and routable for `ompExtension` clients (confirmed present in `InitializeResult.allowedMethods` — see item 19 below), it simply errors when called.

**Implementation:**
- Add a policy-violation service (analogous to `approval::ApprovalService`) that `AdapterEventSink`/`OrchestrationService` calls on `NestedWorkerObserved` when the run's effective `nested` capability is `none`
- Apply `nestedViolationAction`, set `Run.flags.policyQuarantined`, block messages/artifacts/workspace-apply while quarantined, and create an audited cancellation intent
- Implement `policy/violation/decide` for real, restricted to the owning `ompExtension` client; releasing quarantine must never revive a cancelled/terminal run

**References:** `crates/runtime/src/adapter/event_sink.rs:288`, `crates/runtime/src/policy/evaluate.rs`, `crates/runtime/src/service/orchestration.rs:165-167`, `.../2026-07-22-batman-hardening-release.md` (Task 1)

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
Verified `crates/runtime/src/db/migrations.rs:13-19`: the `events` table has only `sequence, timestamp, project_id, run_id, event_json` — no `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns (`source` is still hardcoded `runtime` at the call site). A *live* `events/event` notification's envelope carries `task_id`/`worker_id` (set from the caller's parameters at append time), but a *replayed* one from `events/replay` always has them `None` — `ipc/connection.rs::replay()` can only reconstruct an envelope from what the `events` table's columns actually hold.

The monitor is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields — but any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` gets silently wrong (empty) results.

**Implementation:**
- Schema migration adding `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns to `events`
- Update `append_and_apply` in `crates/runtime/src/domain/repository.rs` to populate these columns
- Update `replay()` in `crates/runtime/src/ipc/connection.rs` to read the new columns

**References:** `crates/runtime/src/db/migrations.rs:13-19`, `crates/runtime/src/domain/repository.rs`, `crates/runtime/src/ipc/connection.rs`

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
`crates/runtime/src/conformance/scenario.rs:45` defines `RESULT_USAGE_ARTIFACTS: &str = "result_usage_artifacts"` as one of the canonical scenario name constants every adapter's conformance report is expected to cover. Three adapters' own conformance test suites fail because their generated in-process reports don't include it:
- `claude_adapter.rs::conformance_fixture_report_covers_every_canonical_scenario_and_all_pass`: `panicked ... unexpected scenario name: result_usage_artifacts`
- `codex_adapter.rs::fixture_conformance_report_covers_every_canonical_scenario_exactly_once`: same panic message
- `copilot_adapter.rs::fixture_conformance_report_covers_every_canonical_scenario_and_provable_ones_pass`: `assertion left == right failed: expected exactly 14 scenarios, got: [...13 listed, result_usage_artifacts absent...]`

Confirmed not caused by any change made in any session — reproduces identically across at least three separate re-runs (2026-08-02, 2026-08-03).

**Implementation:**
- Audit each of the three adapters' `conformance.rs` scenario-list construction (`claude/conformance.rs`, `codex/conformance.rs`, `copilot/conformance.rs`) for why `result_usage_artifacts` is missing or misnamed relative to the canonical constant
- Add the missing scenario coverage (or fix a naming mismatch) so each report enumerates every canonical scenario exactly once

**References:** `crates/runtime/src/conformance/scenario.rs:45`, `crates/runtime/src/adapter/claude/conformance.rs`, `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `crates/runtime/tests/claude_adapter.rs`, `crates/runtime/tests/codex_adapter.rs`, `crates/runtime/tests/copilot_adapter.rs`

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

### 15. `batcave doctor` CLI + `/batman-doctor` OMP command — RESOLVED

**Status:** Closed (re-verified 2026-08-01 and 2026-08-03)
**Priority:** — (was Medium)
**Labels:** cli, doctor, extension

**Description:**
Re-verified: `cli.rs` still has a `Doctor { state_dir, repo, json }` variant wired to `run_doctor()`. `cargo test -p batman-runtime --test doctor` still passes 4/4 (`doctor_with_nonexistent_state_dir`, `doctor_with_missing_db_returns_failure`, `doctor_json_mode_with_missing_db`, `doctor_with_nonexistent_repo`). `packages/extension/src/doctor.ts` and the `/batman-doctor` command remain in place. Note: the extension-side registration test for this feature has since drifted stale in an unrelated way — see item 30.

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
- Full workspace regression check (`--no-fail-fast`), re-run 2026-08-03: every failure present is already tracked (items 5, 6) — none are caused by this fix.

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
