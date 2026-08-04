# BATMAN TODO

Every item below was verified against the current codebase (not inferred from prior docs). Superseded/false claims from earlier sessions are corrected inline. Priority order reflects what blocks core functionality first, then Hardening/release readiness, then polish. Last full re-verification pass: 2026-08-03 (full validation sweep of every open item, plus a fresh `bun test` run — not previously included in prior sweeps). No Critical-severity items remain open. Zero regressions found among previously-tracked items; one stale/false claim corrected (item 54); two new gaps discovered (items 9, 57); **item 6 root-cause corrected** (was misdiagnosed as an adapter-side omission; the actual bug is `scenario::ALL` omitting two constants plus Copilot missing the `une…

**Extended sweep (2026-08-04):** items 1, 10, and 11 closed since the prior sweep (nested-worker policy violations implemented; workspace/artifact RPC surface wired to real handlers; run/cancel now terminates the live vendor subprocess). Item 15 (no OMP tool wraps `profile/register`) discovered while preparing a live demo. A follow-up full re-read of all 8 Obsidian vault planning documents (each dispatched to an independent reviewer, cross-checked against the current code rather than trusting any plan doc's own prose) surfaced 18 further previously-untracked gaps (items 17-25, 31-38, and 62) and one addendum to item 59. The Foundation (M0) plan doc was re-verified in full and confirmed to have no remaining gaps — everything in it is implemented.

**Cross-agent scenario sweep (2026-08-04):** items 63-67 added after tracing the "OMP starts Claude + Codex in parallel worktrees, then cross-reviews" scenario end-to-end. Five new gaps: `run/submit` silently discards `workspaceMode` (item 63); no OMP tool wraps `workspace/acquire` (item 64); no OMP tool wraps `profile/register` (item 65, previously noted in header only); worker MCP tools lack artifact list/fetch (item 66); no coordination primitive for cross-workspace access (item 67). Items 63-65 block parallel isolated execution. Items 66-67 block substantive cross-review.

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

---

### 63. `run/submit` silently discards `workspaceMode` — adapter always uses `repo_root` as its working directory

**Status:** ✅ Closed 2026-08-04
**Priority:** Critical
**Labels:** bug, workspace-isolation, cross-agent, orchestrator

**Description:**
The OMP `batman_run` tool (`packages/extension/src/tools/runs.ts:36-42`) sends `workspaceMode` to `run/submit`. The runtime handler (`crates/runtime/src/service/orchestration.rs:467-533`) parses `taskId`, `workerId`, `prompt` from params — but never reads `workspaceMode`. The adapter is constructed with `repo_root` as its `cwd` (`crates/runtime/src/adapter/registry.rs:399` — `build_adapter(&profile, repo_root, ...)`), so every run executes in the shared repository directory regardless of what `workspaceMode` was requested.

The `workspace/acquire` RPC exists and correctly creates git worktrees (`crates/runtime/src/service/orchestration.rs:642-671`), but it is never called during `run/submit`. The `RunDriverContext` carries no workspace path — only `db`, `project_id`, `run_id`, `task_id`, `worker_id`, `prompt`, `events_tx`, `violation_service` (`crates/runtime/src/service/run_driver.rs:25-37`).

**Impact:** Two parallel runs (e.g., Claude + Codex on the same task) both execute in the same `repo_root`, overwriting each other's changes. Workspace isolation is impossible despite the `workspaceMode` parameter existing in the OMP tool schema.

**Fix:** Parse `workspaceMode` in `run_submit`, call `workspace_acquire` internally when `isolated` is requested, thread the resolved workspace path through `RunDriverContext` to `run_one`/`build_adapter`, and use it as the adapter's `cwd` instead of `repo_root`.

**Resolution:** `run_submit` now parses `workspaceMode`; when `"isolated"`, it acquires a two-phase lease (`allocating` → `materialize` → `activate`), passes the real `PathBuf` into `RunDriverContext.workspace_path`, and `run_one` uses `ctx.workspace_path.as_deref().unwrap_or(repo_root)` as the adapter's `cwd`. `run/get` now also surfaces `workspacePath`/`workspaceMode` via `LeaseService::active_for_run`.

**References:** `crates/runtime/src/service/orchestration.rs:467-533`, `crates/runtime/src/service/run_driver.rs:25-37`, `crates/runtime/src/adapter/registry.rs:365-429`, `packages/extension/src/tools/runs.ts:36-42`

---

### 64. No OMP tool wraps `workspace/acquire`, `workspace/get`, `workspace/release` — OMP cannot manually create isolated workspaces

**Status:** ✅ Closed 2026-08-04
**Priority:** High
**Labels:** missing-tool, workspace-isolation, cross-agent, orchestrator

**Description:**
The `workspace/acquire`, `workspace/get`, `workspace/release` RPC methods are implemented and wired to real handlers (items 10-11 closed). The OMP extension tool registry (`packages/extension/src/tools/index.ts:26-33`) registers `batman_task`, `batman_worker`, `batman_run`, `batman_message`, `batman_approval`, `batman_reconcile` — but no `batman_workspace` tool. OMP has no way to manually acquire isolated workspaces, inspect existing leases, or release them.

**Impact:** OMP cannot create separate git worktrees for parallel workers. Even if `run/submit` were fixed to use `workspaceMode`, OMP has no tool to pre-acquire workspaces or manage workspace lifecycle independently of a run.

**Fix:** Add `batman_workspace` OMP tool wrapping `workspace/acquire`, `workspace/get`, `workspace/release`, `workspace/inspect`. Register it in `tools/index.ts`. Follow the same pattern as `batman_run` (enum `op` field, conditional approval tier).

**Resolution:** Added `packages/extension/src/tools/workspaces.ts` (`batman_workspace`), registered in `tools/index.ts`. Wraps `acquire`/`get`/`release`/`inspect`; `acquire`/`release` are tier `exec`, `get`/`inspect` are tier `read`.

**References:** `packages/extension/src/tools/index.ts`, `packages/extension/src/tools/runs.ts`, `crates/runtime/src/service/orchestration.rs:642-719`, `crates/protocol/src/method.rs:100-109`

---

### 65. No OMP tool wraps `profile/register` — workers cannot be provisioned through OMP tools

**Status:** ✅ Closed 2026-08-04
**Priority:** High
**Labels:** missing-tool, worker-provisioning, orchestrator

**Description:**
The `profile/register` RPC method exists (`crates/protocol/src/method.rs:96-97`) and is dispatched through `OrchestrationService` (`crates/runtime/src/ipc/connection.rs:431`). Discovered while preparing a live demo (item 15 in prior sweep header). The OMP extension tool registry has no `batman_profile` tool — workers cannot be registered with profiles through OMP tools.

**Impact:** Workers provisioned through `batman_worker { op: 'create' }` create a worker identity, but without a registered profile, `run/submit` cannot resolve startup options (adapter kind, model, binary path, MCP config) at submit time. The `resolve_profile` function in the adapter registry (`crates/runtime/src/adapter/registry.rs:431-451`) looks up profiles by adapter/model from the database — if no profile exists, the run fails to start.

**Fix:** Add `batman_profile` OMP tool wrapping `profile/register`. Follow the `batman_worker` pattern: `op` enum with `register`, `list`, `get` operations. Wire the required profile fields (adapter kind, model, startup options, permission envelope, environment allowlist) from the tool's input schema to the RPC params.

**Resolution:** Added `packages/extension/src/tools/profiles.ts` (`batman_profile`), registered in `tools/index.ts`. Wraps `profile/register` with `adapter`, `model`, `startupOptions`, `environmentAllowlist`, `permissionEnvelope`; tier `exec`. (No `list`/`get` ops exist on the RPC surface, so the tool covers `register` only, matching `profile/register`'s actual shape rather than the originally-guessed `WorkerProfile` field set.)

**References:** `crates/protocol/src/method.rs:96-97`, `crates/runtime/src/ipc/connection.rs:431`, `crates/runtime/src/adapter/registry.rs:431-451`, `packages/extension/src/tools/index.ts`

---

### 66. Worker MCP tools lack `batman_artifact_list` and `batman_artifact_fetch` — workers cannot read peer artifacts

**Status:** Open
**Priority:** High
**Labels:** missing-tool, cross-agent, artifact, worker-mcp

**Description:**
The worker coordination MCP server (`crates/runtime/src/coordination/mcp_protocol.rs:63-219`) advertises 6 tools: `batman_task`, `batman_peers`, `batman_send`, `batman_request_child`, `batman_publish_artifact`, `batman_report_blocked`, `batman_ask_policy`. Workers can publish their own artifacts via `batman_publish_artifact` (maps to `coordination/publishArtifact`), but have no tool to list or fetch artifacts.

The runtime-side `artifact/list` and `artifact/fetch` RPC methods exist and are NOT scoped to a specific worker — they operate on the shared artifact store (`crates/runtime/src/service/orchestration.rs:746-778`). A worker with a `batman_artifact_list`/`batman_artifact_fetch` tool could discover and read peer artifacts.

**Impact:** In a cross-review scenario, the reviewing worker cannot discover what artifacts its peer published, nor fetch them for analysis. OMP could fetch artifacts and inject them into a steer message, but that pushes artifact routing logic into OMP rather than keeping it in the coordination layer where it belongs.

**Fix:** Add `batman_artifact_list` and `batman_artifact_fetch` to the worker MCP tool specs in `mcp_protocol.rs`. Map them to `coordination/publishArtifact`-style broker calls (the existing `CoordinationBroker` has no artifact methods — these would need new `coordination/artifact/list` and `coordination/artifact/fetch` RPC methods, or the existing `artifact/list`/`artifact/fetch` could be scoped to the worker's run).

**References:** `crates/runtime/src/coordination/mcp_protocol.rs:63-219`, `crates/runtime/src/service/orchestration.rs:746-778`, `crates/runtime/src/coordination/broker.rs`

---

### 67. No coordination primitive for cross-workspace access — workers cannot inspect peer workspaces for code review

**Status:** ✅ Closed 2026-08-04
**Priority:** High
**Labels:** missing-feature, cross-agent, workspace-isolation, coordination

**Description:**
The current worker coordination tools (`batman_peers`, `batman_send`, `batman_publish_artifact`, `batman_report_blocked`, `batman_ask_policy`) provide messaging, artifact publishing, and policy questions. There is no primitive for a worker to request access to a peer's workspace or receive the peer's workspace path.

In a cross-review scenario, OMP would need to: (1) instruct Claude to review Codex's work, (2) provide Claude with access to Codex's workspace. Without a coordination primitive, OMP's only option is to fetch Codex's artifacts through `artifact/fetch` and paste them into a steer message to Claude — which is lossy (artifacts are typically diffs/patches, not full source) and pushes orchestration logic into OMP.

A cleaner model: OMP sends a `steer` message including a `peerWorkspaceAccess` grant. Batman's coordination layer resolves the peer's workspace path and makes it available to the requesting worker through a `coordination/grantWorkspaceAccess` method. The worker then has a bounded, auditable view of the peer's workspace for review purposes.

**Impact:** Cross-review is degraded to "OMP fetches artifacts, pastes into prompt" rather than "worker accesses peer workspace directly". This loses context (full source, directory structure) and breaks the separation between orchestration (OMP decides WHO reviews WHAT) and execution (the worker does the actual review).

**Fix:** Add `coordination/grantWorkspaceAccess` RPC method (OMP-facing) and a corresponding `batman_grant_workspace_access` or `batman_peer_workspace` worker MCP tool. The method takes a `recipientWorkerId` and `peerRunId`, resolves the peer's workspace lease, and grants the recipient a read-only view. The worker's MCP tool returns the peer's workspace path and lease state.

**Resolution:** Implemented as `coordination/peerWorkspace` (worker-initiated pull, not an OMP-initiated grant): a worker calls `batman_peer_workspace { peerRunId }` (new 8th worker MCP tool); `CoordinationBroker::peer_workspace` verifies `peerRunId` shares the caller's `task_id` (rejecting cross-task lookups), then resolves the peer's active lease via `LeaseService::active_for_run`, returning `{ path, mode, isolationKind, state }`. `batman_peers` now also returns each peer's `runId`, so a worker discovers peers then queries their workspace directly — no separate grant/ACL step, since coordination is already scoped to same-task peers. Two-phase lease semantics (`allocating` → `activate`) ensure `active_for_run` only ever returns a materialized, real path.

**References:** `crates/runtime/src/coordination/mcp_protocol.rs:63-219`, `crates/runtime/src/coordination/broker.rs`, `crates/runtime/src/workspace/lease.rs`, `crates/runtime/src/service/orchestration.rs:642-671`