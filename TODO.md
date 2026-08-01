# BATMAN TODO

Every item below was verified against the current codebase on 2026-08-01 (not inferred from prior docs). Superseded/false claims from earlier sessions are corrected inline. Priority order reflects what blocks core functionality first, then Hardening/release readiness, then polish.

---

## Critical — breaks core functionality end-to-end

### 1. `coordination-mcp` CLI subcommand does not exist — worker MCP integration is broken end-to-end

**Status:** Open
**Priority:** Critical
**Labels:** bug, adapter, worker-mcp, cli

**Description:**
`crates/runtime/src/adapter/mcp_config.rs::coordination_mcp_argv` unconditionally configures every spawned worker CLI (Claude, Codex, Copilot, etc.) to launch its MCP server via `<batcave_path> coordination-mcp --state-dir <dir> --repo <repo> --run-id <id>` (verified: `coordination_mcp_argv` test asserts exactly this 7-element argv, `mcp_config.rs:144-156`).

`crates/runtime/src/cli.rs`'s `Command` enum has exactly 8 variants — `Serve`, `Status`, `Stop`, `Monitor`, `Version`, `Schema`, `Audit`, `Doctor` (verified by reading the full enum, `cli.rs:22-104`) — no `CoordinationMcp` variant. The proxy implementation itself is fully built and tested (`crates/runtime/src/coordination/mcp.rs::run`, 512 lines, reads `BATMAN_WORKER_SCOPE_TOKEN`, authenticates over the owner-only socket, proxies MCP `initialize`/`tools/list`/`tools/call`) — it simply has no CLI entry point. Any worker whose harness spawns this MCP server entry fails immediately with clap's unrecognized-subcommand error, breaking `batman_task`/`batman_send` tool access for every adapter that relies on `workerMcp`.

**Implementation:**
- Add a `CoordinationMcp { state_dir: Option<PathBuf>, repo: PathBuf, run_id: String }` variant to `cli.rs`'s `Command` enum
- Wire it to `crate::coordination::mcp::run` with `ProcessEnvironment` as the token source
- Add an integration test that spawns `batcave coordination-mcp` as a real subprocess and confirms it serves the tool schemas in `crate::coordination::mcp_protocol`

**References:** `crates/runtime/src/adapter/mcp_config.rs`, `crates/runtime/src/cli.rs`, `crates/runtime/src/coordination/mcp.rs`, `crates/runtime/src/coordination/mcp_protocol.rs`

---

### 2. `adapter_registry.rs` integration test suite fails — `workers` table schema mismatch

**Status:** Open (confirmed pre-existing, not introduced by recent sessions — present since commit `90aa259`, 2026-07-25)
**Priority:** Critical
**Labels:** bug, testing, schema-drift

**Description:**
`cargo test -p batman-runtime` fails 5 tests in `crates/runtime/tests/adapter_registry.rs` with `Sqlite(SqliteFailure(... "table workers has no column named task_id"))`. Verified root cause: the actual `workers` table migration (`crates/runtime/src/db/migrations.rs:51-57`) has columns `worker_id, project_id, profile_id, parent_worker_id, created_at, resolved_profile_json` — but `adapter_registry.rs::seed_worker_and_run` (line 53) inserts `INSERT INTO workers (worker_id, task_id, adapter_kind, profile_kind, resolved_profile_json, status, created_at, updated_at)`, none of `task_id`/`adapter_kind`/`profile_kind`/`status`/`updated_at` exist in the real schema. Confirmed via `git show HEAD~1` (and `git show 90aa259~1`) that this INSERT already referenced these non-existent columns before any recent edit — this test file was written against an assumed schema shape that was never migrated. Every test in the file (`a_terminal_profile_uses_terminal_adapter`, `a_terminal_degraded_profile_uses_terminal_adapter`, `authorization_denial_prevents_the_adapter_from_ever_starting`, `duplicate_start_is_rejected`, `running_count_tracks_active_adapters`) fails at the shared setup helper.

**Implementation:**
- Rewrite `seed_worker_and_run`'s `INSERT INTO workers` to match the real schema (`worker_id, project_id, profile_id, parent_worker_id, created_at`), inserting a matching row into `adapter_profiles`/`worker_profiles` first for the `profile_id` foreign key
- Decide whether `tasks`/`runs` inserts in the same helper need equivalent correction (`runs` schema requires `task_id`, `worker_id`, `state`, not `status`)
- Re-run `cargo test -p batman-runtime --test adapter_registry` until all 5 tests pass

**References:** `crates/runtime/tests/adapter_registry.rs`, `crates/runtime/src/db/migrations.rs:51-73`

---

## High — blocks Hardening (M4) readiness

### 3. Nested-worker policy violations are journaled but never quarantined, cancelled, or reported to OMP — `policy/violation/decide` is a stub

**Status:** Open
**Priority:** High
**Labels:** security, policy, hardening

**Description:**
The Hardening plan's Task 1 requires: on `NestedWorkerObserved` while the effective capability is `nested:none`, the runtime must atomically persist a `PolicyViolation`, set `policyQuarantined`, block messages/artifact publication/workspace apply, create an audited worker-cancellation intent, and notify the owning OMP client — resolvable only via `policy/violation/decide`, exposed to the owning `ompExtension` client as `batman_worker op:"resolvePolicyViolation"`.

Verified current state:
- `AdapterEventSink::build_runtime_event` (`event_sink.rs:288`) maps `NestedWorkerObserved` straight to `RuntimeEvent::AdapterNestedWorkerEvent` for the journal — no policy hook, no `PolicyViolation` record, no quarantine flag, no cancellation, no OMP notification.
- `PolicyEvaluator` (`policy/evaluate.rs`) only enforces nested-worker policy at **pre-authorization** time (`authorize()` rejects `is_nested && !self.allow_nested` before a worker starts) — this is a different mechanism from the plan's requirement, which is about a worker that is *already running* and then unexpectedly reports a child mid-run.
- `nested_violation_action` (the config knob controlling `quarantine`/`cancel`/`quarantineAndCancel`) appears exactly once in the entire runtime crate — a hardcoded default in `evaluate.rs:270` — with no consumer.
- `OrchestrationService::dispatch` explicitly stubs the method: `BatmanMethod::PolicyViolationDecide => Err(ServiceError::internal("method is not routed through OrchestrationService"))` (`orchestration.rs:165-167`). `OrchestrationService` has no `policy` field and no `decide_violation` function.

**Implementation:**
- Add a policy-violation service (analogous to `approval::ApprovalService`) that `AdapterEventSink`/`OrchestrationService` calls on `NestedWorkerObserved` when the run's effective `nested` capability is `none`
- Apply `nestedViolationAction`, set `Run.flags.policyQuarantined`, block messages/artifacts/workspace-apply while quarantined, and create an audited cancellation intent
- Implement `policy/violation/decide` for real, restricted to the owning `ompExtension` client; releasing quarantine must never revive a cancelled/terminal run

**References:** `crates/runtime/src/adapter/event_sink.rs:288`, `crates/runtime/src/policy/evaluate.rs`, `crates/runtime/src/service/orchestration.rs:165-167`, `.../2026-07-22-batman-hardening-release.md` (Task 1)

---

### 4. Crash recovery is dead code — never invoked at daemon startup, and implements neither the kill-point matrix nor the mutation barrier the plan requires

**Status:** Open
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

### 5. Events table missing `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` columns

**Status:** Open
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

### 6. `batcave display probe` subcommand does not exist despite the M2/M3 gap-closure doc marking it "Closed"

**Status:** Open (documentation discrepancy confirmed — a fourth false "Closed" claim in that doc)
**Priority:** High
**Labels:** bug, display, cli, documentation

**Description:**
`2026-07-27-batman-m2-m3-gap-closure.md`'s readiness matrix claims: "`batcave` has no `display probe` subcommand... Resolution: Add `Display { Probe { backend, json } }` subcommand... Status: Closed (2026-07-27)." Verified false: `cli.rs`'s `Command` enum (same exhaustive read as item 1, now 8 variants including `Doctor`) has no `Display` variant at all.

The backend logic this subcommand would call is real and ready: `crates/runtime/src/display/herdr.rs::probe(&self) -> Result<HerdrStatus, String>` (line 151) exists, is cached, and has substantial pane-level test coverage (`crates/runtime/tests/herdr_display.rs`, `tmux_display.rs`, `display_registry.rs`). Only the CLI entry point is missing.

**Implementation:**
- Add `Display { Probe { backend: String, json: bool } }` to `cli.rs`'s `Command` enum, wired to the existing herdr/tmux probe logic
- Or, if intentionally descoped, correct the gap-closure doc's "Closed" status rather than leaving a fourth false claim in a document that already warns about three others

**References:** `.../2026-07-27-batman-m2-m3-gap-closure.md`, `crates/runtime/src/cli.rs`, `crates/runtime/src/display/herdr.rs:151`, `crates/runtime/src/display/tmux.rs`

---

### 7. Release conformance gate is a non-functional stub — writes empty reports, never invokes real adapter checks

**Status:** Partially implemented — structural gate wired into `release.yml`, but the runner itself is a stub
**Priority:** High
**Labels:** ci, testing, conformance, release

**Description:**
Verified `tests/conformance/run.ts` (confirmed by direct read): `runAllFixtures` writes an empty `ConformanceReport` (`scenarios: []`, `declaredCapabilities: []`) for each of `claude`/`codex`/`copilot`/`omp-rpc` — explicitly labeled `// STUB` in its own doc comment — rather than spawning `batcave conformance --adapter <name> --output <path>`. `assertReportComplete` only checks that each adapter key is present, not that any scenario ran or passed. `tests/install/private-registry.test.ts` is likewise a stub (`expect(true).toBe(true)`).

This currently means the conformance job in `release.yml` cannot yet fail a release for a real regression — although the *empty-report* shape is intentionally rejected by `assert-report.ts`'s stricter validators elsewhere (verified in a prior session: the gate correctly blocks on an empty report), so this is a release-blocking gate by omission rather than a false-pass. Confirm this remains true before loosening it.

**Implementation:**
- Replace `runAllFixtures`'s stub loop with real `batcave conformance --adapter <name> --output <path>` subprocess invocations, one per adapter, and merge the resulting reports
- Replace `assertReportComplete` with real scenario-level assertions (every declared capability has a corresponding passed scenario)
- Implement `private-registry.test.ts` for real: publish to a mock registry, install, verify the binary launches

**References:** `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, `tests/install/private-registry.test.ts`, `.github/workflows/release.yml`, `.../2026-07-22-batman-hardening-release.md` (Task 6)

---

## Medium

### 8. Operator-facing docs only partially split; `docs/installation.md`, `configuration.md`, `security.md`, `recovery.md` still don't exist as standalone files

**Status:** Partially implemented
**Priority:** Medium
**Labels:** documentation, release

**Description:**
The Hardening plan's Tasks 7-8 specify six standalone operator docs. Verified via glob of `docs/`: `compatibility.md` and `operations.md` now exist as separate files (confirming the prior session's claim), and `release/0.1.0-checklist.json` exists with real gate evidence. However, `installation.md`, `configuration.md`, `security.md`, and `recovery.md` still do not exist as separate files — that content remains consolidated inside `docs/getting-started.md` and `docs/architecture.md`.

**Implementation:**
- Once items 1-7 above close, regenerate `docs/compatibility.md` from a real passing conformance report
- Decide whether to finish splitting `getting-started.md`'s installation/configuration/security/recovery sections into the four remaining named files, or formally amend the plan's expectation to "consolidated, cross-referenced" — currently neither has happened
- Regenerate `release/0.1.0-checklist.json` once the Critical/High items above close, since its current gate evidence predates several of them

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 7, Task 8), `docs/getting-started.md`, `docs/compatibility.md`, `docs/operations.md`, `release/0.1.0-checklist.json`

---

### 9. `display/register`, `display/heartbeat`, `display/unregister`, `display/list` RPC methods were never implemented — deferred by design, but the deferral isn't tracked here

**Status:** Deferred (intentional per M2/M3 gap-closure decision #6, not a bug)
**Priority:** Medium
**Labels:** display, rpc, deferred

**Description:**
The Workspaces/Displays plan's Task 5 specifies canonical `display/*` RPC methods (register/heartbeat/unregister/list) as the mechanism for a display client to announce itself. Verified via grep of `crates/protocol/src/method.rs`: none of these four methods exist in `BatmanMethod` at all. The M2/M3 gap-closure doc's Decision #6 explicitly chose this: "Monitor: minimal `batcave monitor` on existing Display-role methods; the four `display/*` RPC methods and registry-over-RPC are explicitly deferred." `batcave monitor` (item 6's sibling, already implemented) works entirely on top of existing `runtime/status`/`events/replay`/`events/subscribe` methods instead.

This is not a functional bug today, but it means a *third-party* display client (one not shipping as part of `batcave monitor`/Herdr/tmux) has no way to register itself, appear in a future `display/list`, or heartbeat its liveness. Tracking it here so it doesn't silently disappear from scope.

**Implementation:**
- No immediate action required — confirm this deferral is still acceptable for M4, or schedule the four methods for a post-M4 milestone
- If scheduled, implement per the original Task 5 spec: `DisplayRegistration` as expiring presence (not a durable orchestration record), monitor rendering unchanged

**References:** `crates/protocol/src/method.rs`, `.../2026-07-22-batman-workspaces-displays.md` (Task 5), `.../2026-07-27-batman-m2-m3-gap-closure.md` (Decision 6)

---

## Low / Environment / Permanent

### 10. Redaction regex denylist expansion — RESOLVED

**Status:** Closed (verified 2026-08-01)
**Priority:** — (was Low)
**Labels:** security, defense-in-depth

**Description:**
Previously open: the redaction denylist lacked `ghp_`/`AKIA…`/JWT patterns. Verified `crates/runtime/src/security/redaction.rs:169-187`: all three now present — `github_pat` (`ghp_[A-Za-z0-9]{16,}`), `aws_access_key` (`AKIA[0-9A-Z]{16}`), and `jwt` (three base64url segments). All 8 redaction unit tests pass, including `org_patterns_are_applied_during_redaction`.

**References:** `crates/runtime/src/security/redaction.rs:169-187`

---

### 11. Worker adapter authorization layer — RESOLVED

**Status:** Closed (verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** adapter, authorization, hardening

**Description:**
Previously open: production `lifecycle.rs` constructed `DenyByDefaultAuthorization` instead of the real `PolicyEvaluator`. Verified `crates/runtime/src/lifecycle.rs:215`: `AdapterRegistry::new` now receives `Arc::new(PolicyEvaluator::new(policy))`, loaded from the effective merged configuration (`--org-config`/`--repo-config`/`--user-config` CLI flags). The credential store for `workerMcp` connections (`RejectAllWorkerVerifier`) is a separate, still-open concern — not re-verified in this pass; if still a reject-all default, it belongs as its own tracked item once item 1 (coordination-mcp CLI) is fixed and the credential path becomes exercisable end-to-end.

**References:** `crates/runtime/src/lifecycle.rs:214-218`, `crates/runtime/src/policy/evaluate.rs`

---

### 12. CI workflow on ordinary pushes/PRs — RESOLVED

**Status:** Closed (verified 2026-08-01)
**Priority:** — (was High)
**Labels:** ci, testing, release

**Description:**
Previously open: no `.github/workflows/ci.yml` existed. Verified: `.github/workflows/ci.yml` now exists with five jobs — `format` (`cargo fmt --all --check`), `clippy` (`-D warnings`), `test` (matrix `ubuntu-latest`/`macos-latest`, `cargo test --workspace` + `bun test`), `generate-check` (`bun run generate --check`), and `security` (`cargo audit` + `gitleaks-action`). Runs on push/PR to `main`/`master`. One residual gap: no JS/TS formatter is configured (tracked separately below, item 16).

**References:** `.github/workflows/ci.yml`

---

### 13. `batcave doctor` CLI + `/batman-doctor` OMP command — RESOLVED

**Status:** Closed (re-verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** cli, doctor, extension

**Description:**
Re-verified: `cli.rs` has a `Doctor { state_dir, repo, json }` variant wired to `run_doctor()`. `cargo test -p batman-runtime --test doctor` passes 4/4 (`doctor_with_nonexistent_state_dir`, `doctor_with_missing_db_returns_failure`, `doctor_json_mode_with_missing_db`, `doctor_with_nonexistent_repo`). `packages/extension/src/doctor.ts` and the `/batman-doctor` command remain in place.

**References:** `crates/runtime/src/cli.rs`, `crates/runtime/src/doctor.rs`, `crates/runtime/tests/doctor.rs`, `packages/extension/src/doctor.ts`

---

### 14. Claude adapter lifecycle/usage/result event extraction — FALSE CLAIM, adapter already implements this

**Status:** Closed — corrected a stale claim (verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** adapter, claude, documentation-correction

**Description:**
Previously claimed open: "the normalizer does not extract `VendorSessionEstablished`, `UsageReported`, or `MessageFinal(role=\"result\")`." Verified **false** by direct read of `crates/runtime/src/adapter/claude/normalize.rs`: `VendorSessionEstablished` is emitted from `RawFrame::SystemInit` (line 92), `MessageFinal` is emitted both for streamed text blocks (line 167) and the result frame (line 260, `role: "result"`), and `UsageReported` is emitted from the result frame's usage data including cost (line 254). All three correlate to the same normalizer pass. No code change needed; this item should not have been carried forward as open.

**References:** `crates/runtime/src/adapter/claude/normalize.rs:92,167,254,260`

---

### 15. Codex adapter lifecycle/usage/artifact event extraction — FALSE CLAIM, adapter already implements this

**Status:** Closed — corrected a stale claim (verified 2026-08-01)
**Priority:** — (was Medium)
**Labels:** adapter, codex, documentation-correction

**Description:**
Previously claimed open: "the normalizer does not extract `MessageChunk`, `MessageFinal`, `ToolStarted`, `ToolResult`, `UsageReported`, or `ArtifactProduced`." Verified **false** by direct read of `crates/runtime/src/adapter/codex/normalize.rs`: all six are present — `ToolStarted` (line 78, `commandExecution`), `MessageFinal` (line 90), `ToolResult` (line 105), `ArtifactProduced` (line 112, `fileChange`), `MessageChunk` (line 121, streaming delta), `UsageReported` (line 130). No code change needed; this item should not have been carried forward as open.

**References:** `crates/runtime/src/adapter/codex/normalize.rs:78,90,105,112,121,130`

---

### 16. No JS/TS formatter configured in CI

**Status:** Open
**Priority:** Low
**Labels:** ci, tooling

**Description:**
`.github/workflows/ci.yml`'s `format` job only runs `cargo fmt --all --check`. No prettier/biome (or equivalent) is configured or checked for the TypeScript packages.

**Implementation:**
- Pick a formatter (prettier or biome), add a config file, add a `format:check` script, wire it into the `format` CI job

**References:** `.github/workflows/ci.yml`

---

### 17. Subscription forwarder reaping

**Status:** Open (low priority, harmless)
**Priority:** Low
**Labels:** cleanup, subscription

**Description:**
Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast; harmless in practice since a closed connection's own `events_rx.recv()` loop (`spawn_subscription`) exits on its own `Err` the next time anything is broadcast.

**Implementation:**
- Optional: add explicit reaping logic for closed connections; current behavior is acceptable

**References:** `crates/runtime/src/ipc/connection.rs::spawn_subscription`

---

### 18. Copilot adapter: ACP v1 protocol limitation on usage reporting

**Status:** Permanent (protocol wall, not a code bug)
**Priority:** Low
**Labels:** adapter, copilot, protocol

**Description:**
ACP v1 does not transmit token usage/cost in its session update frames. The adapter honestly declares `usage: none`. No code change is possible until Copilot ships a newer ACP version that adds this.

**References:** `crates/runtime/src/adapter/copilot/client.rs`

---

### 19. Copilot `unexpected_child_observation`: permanent ACP v1 protocol wall

**Status:** Permanent (protocol wall, not a code bug)
**Priority:** Low
**Labels:** adapter, copilot, protocol

**Description:**
ACP protocol v1 has no `session/update` variant for a vendor-spawned subagent at all. `adapter/copilot/compatibility.rs` pins `COPILOT_MIN/MAX_ACP_PROTOCOL_VERSION = 1`; the verified CLI table (`1.0.73`, `1.0.75`) is both v1. `normalize.rs` correctly drops unrecognized updates to zero events rather than fabricate a `NestedWorkerObserved`. A test (`copilot_adapter.rs`) already asserts this stays true if `COPILOT_MAX_ACP_PROTOCOL_VERSION` is ever raised without adding the mapping. Resolvable only by a Copilot ACP v2 release.

**References:** `crates/runtime/src/adapter/copilot/compatibility.rs`, `crates/runtime/tests/copilot_adapter.rs`

---

### 20. Copilot live test requires an authenticated CLI session

**Status:** Environment dependency, not a code gap
**Priority:** Low
**Labels:** testing, environment

**Description:**
`real_binary_initialize_and_session_list_never_invoke_a_model` requires a real, authenticated Copilot CLI session; without one the test is skipped, as designed.

**Implementation:**
- Run with `BATMAN_LIVE_COPILOT=1` and a valid Copilot session to exercise it; no code change needed

**References:** `docs/manual-testing.md` §4c

---

### 21. OMP-RPC: approval and artifact normalization gaps

**Status:** Open
**Priority:** Low
**Labels:** adapter, omp-rpc, conformance

**Description:**
Two genuine (non-vendor-imposed) gaps:
- `omp_rpc/normalize.rs`'s catch-all silently drops the real vendor's `extension_ui_request` frame; `ApprovalsCapability::Observable` is declared but not backed by any observable event or pending-approval state.
- No `ArtifactProduced` path exists for OMP-RPC at all; `snapshot()`'s `artifacts` list stays empty because there is no `normalize_frame` case constructing it (this specific point supersedes an earlier false claim that `snapshot()` "hardcodes" empty artifacts — the real gap is narrower: the field is real and mutable, just never populated).

**Implementation:**
- Add an `extension_ui_request` → `AdapterEventPayload` mapping in `normalize_frame`, plus pending-approval tracking in `SharedRunState`
- Identify the real vendor frame(s) carrying artifact information (needs a live, artifact-producing session to observe) and add the corresponding case

**References:** `crates/runtime/src/adapter/omp_rpc/normalize.rs`, `crates/runtime/src/adapter/omp_rpc/mod.rs`

---

### 22. Codex/Copilot: several capabilities are unprovable in fixture mode — not a bug, requires a gated live run to confirm the positive case

**Status:** Open (expected — resolvable only via a real, billed model call)
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

### 23. OMP-RPC conformance: `probe`/`cancellation_scope`/`follow_up` depend on a genuinely reachable local model, not just a listed one — expect flakiness, not a code defect

**Status:** Open (environment/infrastructure dependency, not fixable in this codebase)
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

