# BATMAN TODO

The single source of truth for this project's implementation gaps, verified against the
current codebase rather than inferred from planning documents.

**Pruned 2026-08-06.** Every item previously marked fully `✅ Closed`/`Complete` was removed —
their resolutions are preserved in git history and `docs/journal.md`. Items closed by being
**moved** rather than resolved (Copilot usage reporting, Copilot nested-worker observation,
Org Config URL support, and the four config-backlog ideas) now live in
`docs/future-features.md` with a decision trigger each. Only genuinely open or
partially-closed items remain below.

---

## Low / Environment / Permanent

### 55. Codex/Copilot: several capabilities are unprovable in fixture mode — not a bug, requires a gated live run to confirm the positive case

**Status:** ⚠️ Partially closed 2026-08-05 — split by cause, with per-scenario evidence.
- **Codex (4 scenarios + `result_usage_artifacts`): blocked on account credits, not code.** `follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, and `result_usage_artifacts` all fail with one vendor cause: `usageLimitExceeded: Your workspace is out of credits.` `codex login status` reports `Logged in using ChatGPT`; `initialize`/`thread/start` succeed; the turn is refused server-side after ~3s. Refill and they become provable with no code change. Report: `release/live-codex.json`.
- **Adapter fix this run DID produce:** the `error` notification was previously dropped by `codex/normalize.rs`, so this appeared as an unexplained `never produced a MessageFinal within 60s`. It now normalizes to `ProtocolHealthChanged{healthy:false}` and the live probe fails fast with the vendor's own text (62s → 5s). Defended by `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event`.
- **Copilot `session_resume`/`runtime_restart`: a genuine ACP v1 protocol wall.** A session that completed a real turn cannot be reloaded from a new process — `session/load` answers `Resource not found: Session <id> not found`. Recorded as a CLI limitation, distinct from the Codex account condition.
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

### 68. Unify extension connection identity with task ownership

**Status:** ✅ Closed 2026-08-07 — identity threading complete; integration test passing.
**Priority:** Critical
**Labels:** extension, authorization, ownership

**Description:**
The extension authenticates as constant instance `batman-extension` but writes the OMP session ID as task owner. Approval and policy-violation decisions require an exact owner/principal match, so the creating session cannot decide them; reconciliation can also collapse multiple sessions onto the shared identity.

**Resolution:**
- `EnsureRuntimeOptions` gains optional `sessionId` field carrying the OMP session ID
- `initParams` uses `sessionId` for `instanceId` when provided, falling back to `"batman-extension"`
- `tryConnect`, `connectWithBackoff`, and `ensureRuntime` thread `sessionId` through the connection chain
- `getClient` in `index.ts` passes `extCtx.sessionManager.getSessionId()` as `sessionId`
- `statusContextFor` and status tool/command also pass `sessionId` (fixed the status path gap)
- All tool call sites updated to pass `extCtx` instead of just `cwd` to `getClient`
- Defended by `packages/extension/src/ownership.test.ts`: two live-daemon integration tests proving the sessionId → instanceId → ownerClientInstanceId chain. The positive case seeds a task/approval/violation owned by sessionId A, connects as A, and succeeds on both decide calls. The negative case seeds the same data but connects as sessionId B, confirming both decisions fail with "does not own".

**References:** `REVIEW.md` (R1), `packages/extension/src/runtime.ts`, `packages/extension/src/index.ts`, `packages/extension/src/tools/*.ts`, `packages/extension/src/ownership.test.ts`

---

### 69. Release policy concurrency slots on run settlement

**Status:** ✅ Closed 2026-08-07
**Priority:** Critical
**Labels:** policy, runtime, availability

**Description:**
Each authorization increments the active-run counter, but the production authorization trait has no release operation and the adapter completion watcher never decrements it. After `concurrency_ceiling` cumulative runs, the daemon permanently rejects new runs until restart.

**Resolution:**
- Added `release()` method to `AdapterAuthorization` trait
- `FixtureAuthorization::release()` is a no-op (no slots to release)
- `PolicyEvaluator::release()` calls `self.decrement_runs()`
- The adapter completion watcher clones `authorization` and calls `release()` after evicting the adapter
- `run_one` releases the slot on all post-authorize error paths (availability probe, build_adapter, adapter.start)
- The watcher handles `Lagged` broadcast errors by continuing, and releases only on `Closed`
- Defended with a real-`PolicyEvaluator` registry integration test (`releasing_a_policy_evaluator_slot_frees_the_registry_ceiling` in `crates/runtime/tests/adapter_registry.rs`): books the one slot at `concurrency_ceiling: 1`, proves `registry.start()` denies a second run with "concurrency ceiling", releases through the trait object, and proves a subsequent start is no longer ceiling-denied

**References:** `REVIEW.md` (R2), `crates/runtime/src/adapter/registry.rs`, `crates/runtime/src/policy/evaluate.rs`, `crates/runtime/tests/adapter_registry.rs`

### 70. Provide a Linux ARM64 linker in release CI

**Status:** ✅ Closed 2026-08-07
**Priority:** Critical
**Labels:** release, ci, cross-compilation

**Description:**
Release CI builds `aarch64-unknown-linux-gnu` on x86_64 Ubuntu but installs only the Rust target. No AArch64 GNU linker/C compiler is installed or configured, while bundled SQLite requires target C compilation.

**Resolution:**
- Added `gcc-aarch64-linux-gnu` installation step for linux-arm64-gnu target
- Set `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER`, `CC_aarch64_unknown_linux_gnu`, and `AR_aarch64_unknown_linux_gnu` env vars
- Added dry-run CI build workflow (`.github/workflows/ci-release.yml`) for every release target

**References:** `REVIEW.md` (R3), `.github/workflows/release.yml`, `.github/workflows/ci-release.yml`, `release/targets.json`

### 71. Preserve batcave executable mode across release artifacts

**Status:** ✅ Closed 2026-08-07
**Priority:** Critical
**Labels:** release, packaging, permissions

**Description:**
`xtask package` creates `bin/batcave` as executable, but zipped GitHub artifact transfer restores files as mode `0644`. `package-set` then correctly rejects every downloaded leaf.

**Resolution:**
- Removed broken flatten loops from both `package-set` and `publish` jobs in release.yml
- Both jobs now use `find ... -name batcave -exec chmod +x {} +` to restore executable bits after download
- The `package-set` executable-mode assertion is preserved (it still validates the bit)

**References:** `REVIEW.md` (R4), `.github/workflows/release.yml`, `crates/xtask/src/main.rs`

### 72. Enforce human-required approvals server-side

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** approval, security, extension

**Description:**
The extension shows a human dialog only when UI is available, then falls through to the model-supplied decision in headless mode. The runtime stores `humanRequired` but does not enforce it when deciding.

**Implementation:**
- Require an authenticated human-decision signal in `ApprovalService::decide`.
- Fail closed in the extension when a human-required approval has no interactive UI.

**References:** `REVIEW.md` (R5), `packages/extension/src/tools/approvals.ts:73`, `crates/runtime/src/approval/service.rs:145`

---

### 73. Reconnect orchestration tools after cached client closure

**Status:** ✅ Closed 2026-08-08
**Priority:** High
**Labels:** extension, ipc, recovery

**Description:**
`getClient` returns a cached client without checking its closed state. After idle shutdown or socket failure, every orchestration tool and the monitor keeps using the dead client; only the status tool clears the cache.

**Resolution:**
- Added `BatmanClient.isClosed` getter exposing the private `#closed` flag so a cached instance can be checked for liveness
- Added `resolveClient()` in `status.ts`: returns the cached client while its socket is open; on a closed cache, tears it down and reconnects via `ensureRuntime`
- Made `getRuntimeStatus` and `getClient` (in `index.ts`) use `resolveClient` as the single construction site, eliminating the duplicate `buildStatusContext` call and manual `sessionId` splice
- Let the monitor re-subscribe after its client dies: replaced the one-shot `connected` boolean with a liveness-checked client reference; `/batman` now repairs a dead monitor
- Added `reconnect.test.ts`: live-daemon test that proves the cache self-heals after a daemon restart

**References:** `REVIEW.md` (R6), `packages/extension/src/index.ts`, `packages/extension/src/client.ts`, `packages/extension/src/status.ts`, `packages/extension/src/monitor/controller.ts`

---

### 74. Dispatch retried runs through the adapter driver

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** orchestration, retry, adapter

**Description:**
`run/retry` creates and broadcasts a fresh queued run but never invokes `run_driver`. No scheduler consumes it, so no work executes and recovery later marks it failed.

**Implementation:**
- Persist or require the prompt and route retry through submit's authorization, workspace, display, and driver-start path.
- Add an integration test proving a retry invokes the adapter and leaves queued state.

**References:** `REVIEW.md` (R7), `crates/runtime/src/service/orchestration.rs:742`

---

### 75. Fail releases when conformance reports fail

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** conformance, release, validation

**Description:**
Conformance reports compute aggregate `passed`, but the CLI always exits zero and the release validator never checks that field. A canonical scenario regression can therefore pass release after consistent capability downgrading.

**Implementation:**
- Reject reports whose aggregate `passed` is false and list their failed scenarios.
- Return non-zero from `batcave conformance` when any requested report fails.

**References:** `REVIEW.md` (R8), `tests/conformance/assert-report.ts:120`, `crates/runtime/src/cli.rs:693`

---

### 76. Validate release tags and every npm package version

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** release, npm, versioning

**Description:**
The workflow derives `--version` from the extension package and package-set compares it to the same file. It never checks the leaf packages' published npm versions or the release tag, allowing partial or internally mismatched publication.

**Implementation:**
- Validate all leaf `package.json` versions against the extension version.
- Validate `github.ref_name` equals `v<version>` before build/publish.

**References:** `REVIEW.md` (R9), `.github/workflows/release.yml:144`, `crates/xtask/src/main.rs:578`

---

### 77. Enforce task isolation for artifact list and fetch

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** artifact, authorization, isolation

**Description:**
OMP and worker MCP contracts say artifacts are scoped to the current task, but list returns every artifact in the repository daemon and fetch accepts any artifact ID without task/run/principal filtering.

**Implementation:**
- Carry authenticated task scope into list/fetch and reject cross-task artifact IDs.
- Add two-task isolation tests for both OMP-extension and worker-MCP roles.

**References:** `REVIEW.md` (R10), `crates/runtime/src/service/orchestration.rs:1136`, `crates/runtime/src/workspace/artifact_store.rs:160`

---

### 78. Preserve Copilot ACP stop reasons as runtime outcomes

**Status:** Open (discovered 2026-08-06 during codebase review)
**Priority:** High
**Labels:** adapter, copilot, observability

**Description:**
The Copilot client returns ACP `stopReason`, but initial and follow-up callers discard it. Refusal, cancellation, token exhaustion, and maximum-turn termination are therefore indistinguishable from normal completion.

**Implementation:**
- Map supported stop reasons to explicit final/health events and fail non-success outcomes.
- Add fixtures for refusal, cancellation, and limit termination.

**References:** `REVIEW.md` (R11), `crates/runtime/src/adapter/copilot/client.rs:315`, `crates/runtime/src/adapter/copilot/mod.rs:422`
