# BATMAN Codebase Review

**Reviewed:** 2026-08-06 (baseline) · **Updated:** 2026-08-08 (R5-R11 fix verification + PR review of those fixes)
**Baseline commit:** `3907e8fb8d31f5d275293a9e9302600d436cee44` · **Fix commits:** `8331a34` `9720c63` `8457de5` `6bd6a00` `f9e95c4` `797d5e6` `e8204da` `44093d4` `e4befb8` `bcff4ce` `143e1b3` `de07022` `60fd4de` `bb209eb` `e8e5f4b` `5c9444f`

This file consolidates the original codebase review, the implementation-gap tracker
(formerly `TODO.md`), and the PR review of the R5-R11 remediation commits (formerly
`review-summary.md`) into one current document. All three are superseded by this file.

## Scope and method

The committed tree was split across four parallel reviews: runtime core;
adapters/policy/security; TypeScript/OMP integration; and build/docs/release. Every
finding below was re-read against cited source before inclusion, and every Low/Medium item
carried forward from the 2026-08-06 baseline was independently re-verified against the current
tree again on 2026-08-08 (see R19-R32 below — six were found stale/resolved and one overstated
during that re-verification, all corrected in place). The R5-R11 fix commits were then
independently re-reviewed by 16 reviewer agents grouped by locality; their findings appear as
R33+ below, each re-verified against the current tree in this pass.

## Baseline

Verified before this document was last written:

- `cargo check --workspace --all-targets` — clean
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean
- `cargo fmt --all --check` — clean
- `cargo test --workspace` — all suites passed
- `bun run generate --check` — generated artifacts current
- `bun run format:check` — clean
- `bun test packages` — 123 passed, 0 failed
- `bun test tests/conformance` — 10 passed, 0 failed

**Not re-run for this update** — the findings below (R33+) were verified by direct code
reading (`grep`/`read`), not by re-executing the suite. Run the commands above plus
`bunx tsc --noEmit` before merge to confirm they still hold.

## Findings

### Critical

#### R1. Extension identity and task ownership can never match — ✅ RESOLVED

**Location:** `packages/extension/src/runtime.ts:249-258`; `packages/extension/src/tools/tasks.ts:41-47`; `crates/runtime/src/approval/service.rs:145-159`; `crates/runtime/src/policy/violation.rs:487-525`

**Resolution (2026-08-07):** `EnsureRuntimeOptions` gains optional `sessionId` field; `initParams` uses it for `instanceId` when provided. All connection/tool call sites thread `sessionId` from `extCtx.sessionManager.getSessionId()`. Defended by `packages/extension/src/ownership.test.ts`.

#### R2. Concurrency slots are never released in production — ✅ RESOLVED

**Location:** `crates/runtime/src/policy/evaluate.rs:271-275,371-401`; `crates/runtime/src/adapter/registry.rs:48-63,188-268`; `crates/runtime/src/lifecycle.rs:258-270`

**Resolution (2026-08-07):** Added `release()` to `AdapterAuthorization` trait; `PolicyEvaluator::release()` calls `decrement_runs()`. Watcher releases the slot after eviction and on all post-authorize error paths. Defended by `releasing_a_policy_evaluator_slot_frees_the_registry_ceiling` in `crates/runtime/tests/adapter_registry.rs`.

#### R3. Linux ARM64 release builds lack a cross-linker — ✅ RESOLVED

**Location:** `.github/workflows/release.yml:29-50`; `release/targets.json:5`; `Cargo.toml:18`

**Resolution (2026-08-07):** Added `gcc-aarch64-linux-gnu` install step, target-specific linker/CC/AR env vars, and a dry-run CI build workflow (`.github/workflows/ci-release.yml`).

#### R4. GitHub artifact transfer strips the executable bit required by package validation — ✅ RESOLVED

**Location:** `.github/workflows/release.yml:74-78,129-152,167-179`; `crates/xtask/src/main.rs:426-438,620-644`

**Resolution (2026-08-07):** `package-set`/`publish` jobs restore the executable bit with `find ... -exec chmod +x {} +` after artifact download. The executable-mode assertion in `package-set` is preserved.

### High

#### R5. `humanRequired` approvals can be model-approved without a human — ✅ RESOLVED

**Location:** `packages/extension/src/tools/approvals.ts:53-95`; `crates/runtime/src/approval/service.rs:145-216`; `crates/runtime/src/service/query.rs:243-255`

**Resolution (2026-08-08):** `DecidedBy` enum (`Human`/`Model`) added to protocol. `ApprovalService::decide` rejects a `Model` decision on a `human_required` approval. Extension fails closed with no UI. See **R34** for a serialization defect this introduced.

#### R6. A dead cached runtime client breaks all tools until status is called — ✅ RESOLVED

**Location:** `packages/extension/src/index.ts:44-56`; `packages/extension/src/client.ts:135-167`; `packages/extension/src/status.ts:63-93`; `packages/extension/src/context.ts:14-19`

**Resolution (2026-08-08):** `BatmanClient.isClosed` exposed. `resolveClient()` in `status.ts` reconnects on a closed cache; `getRuntimeStatus`/`getClient` use it as the single construction site. Monitor re-subscribes after its client dies. Defended by `reconnect.test.ts`. See **R39** for a residual defect in the monitor's shutdown path.

#### R7. `run/retry` creates a queued run but never starts its adapter — ✅ RESOLVED

**Location:** `crates/runtime/src/service/orchestration.rs:789-891`

**Verified 2026-08-08:** `run_retry` (line 789) now calls the shared `start_queued_run` helper (line 540, 864) — identical driver-start path to `run_submit` (line 722). Test coverage: `orchestration_rpc.rs:1300-1306,1348,1429`.

*(This finding's original text and Suggested Fix, which predated the resolution, are removed — the fix landed and is verified above. Former TODO #74 is closed accordingly.)*

#### R8. The release conformance gate ignores aggregate failure — ✅ RESOLVED

**Location:** `crates/runtime/src/conformance/report.rs:99-117`; `crates/runtime/src/cli.rs:693-717`; `tests/conformance/assert-report.ts:101-148`; `.github/workflows/release.yml:80-123,160-163`

**Resolution:** `de07022` — `batcave conformance --fixture` gates against `fixtures/conformance/fixture-mode-baseline.json`. Unexpected failures fail the gate; baseline entries that start passing also fail. See **R44** for defects discovered in the capture/scrub machinery that produces these fixtures.

#### R9. Release version checks do not validate the packages npm publishes — ✅ RESOLVED

**Location:** `.github/workflows/release.yml:144-152,195-206`; `crates/xtask/src/main.rs:578-611`

**Resolution:** `bb209eb` — `package-set` verifies each leaf's own `package.json` version. `version-gate` CI job verifies the git tag matches `v<version>` before any build work.

#### R10. Artifact APIs are project-scoped despite claiming task isolation — ✅ RESOLVED, WITH GAPS

**Location:** `packages/extension/src/tools/artifacts.ts:25-42`; `crates/runtime/src/coordination/mcp_protocol.rs:133-156`; `crates/runtime/src/service/orchestration.rs:1136-1177`; `crates/runtime/src/workspace/artifact_store.rs:160-222`

**Resolution:** `44093d4` — `Artifact.run_id` populated by `WorkspaceInspector`/`WorkspaceApplier`. `artifact/list`/`artifact/fetch` scope by `owner_client_instance_id` via `owned_run_ids_op`. Behavioral test `artifact_isolation_enforces_task_ownership_scoping` proves cross-owner isolation.
**Gaps introduced/remaining:** see **R35** (fetch authorizes after reading content — timing oracle) and **R36** (no test asserts producers actually stamp `run_id`, so reverting the fix leaves the suite green).

#### R11. Copilot turn stop reasons are discarded — ✅ RESOLVED, WITH A COSMETIC DEFECT

**Location:** `crates/runtime/src/adapter/copilot/client.rs:315-344`; `crates/runtime/src/adapter/copilot/mod.rs:422-426,462-489`; `crates/runtime/src/adapter/copilot/normalize.rs:18-89`

**Resolution:** `bcff4ce` — `copilot_normalize_stop_reason()` maps every stop reason to `ProtocolHealthChanged` events and a failure disposition; `settle_turn()` fails the turn for non-success reasons. 8 unit tests defend the mapping. See **R42** for a mangled detail string on the unknown-reason path.

### Medium

#### R12. Claude error result subtypes are normalized as usage only

**Location:** `crates/runtime/src/adapter/claude/protocol.rs:183-194`; `crates/runtime/src/adapter/claude/normalize.rs:254-269`; `crates/runtime/tests/claude_adapter.rs:611-639`

`RawResult` omits `subtype`/`is_error`. The committed `error_max_turns` fixture emits only `UsageReported`. Model the failure discriminators and emit an explicit terminal failure. **Open.**

#### R13. Policy cancellation records success after a process-kill failure

**Location:** `crates/runtime/src/policy/violation.rs:446-478`; `crates/runtime/src/adapter/registry.rs:308-321`

A failed cancellation is logged, then the run is durably transitioned to `cancelled` anyway. Distinguish no-running-adapter from kill failure. **Open.**

#### R14. Per-run redactor construction has a fail-open fallback

**Location:** `crates/runtime/src/lifecycle.rs:194-205`; `crates/runtime/src/adapter/event_sink.rs:152-168`

Event-sink construction falls back to built-in patterns on invalid regex rather than propagating. Not presently reachable with current wiring, but the trap remains. **Open.**

#### R15. `batman_task.description` is silently discarded

**Location:** `packages/extension/src/tools/tasks.ts:20-47`; `crates/runtime/src/service/orchestration.rs:300-315`

The tool advertises task text but never sends it; the RPC has no field for it. **Open.**

#### R16. Violation resolution schema accepts prose the runtime rejects

**Location:** `packages/extension/src/tools/violations.ts:14-24`; `crates/runtime/src/policy/violation.rs:487-497`

Tool accepts any string; runtime accepts only `release`/`cancel`. Use a closed Zod enum. **Open.**

#### R17. Generated TypeScript exports and hand-written enums can drift

**Location:** `packages/protocol-ts/src/index.ts:1-50`; `crates/xtask/src/main.rs:206-247`; `packages/extension/src/tools/workspaces.ts:20-24`; `packages/extension/src/tools/artifacts.ts:16-20`

**Evidence (re-verified 2026-08-08):** The barrel (`index.ts:1-39`) exports 35 types through `WorkerId` alphabetically but omits at least `ApplyStrategy`, `DecidedBy`, `DisplayBackend`, `DisplayConfig`, `DisplayPlacement`, `IsolationKind`, and `LeaseMode` — all present under `generated/` (48+ files total). `workspaces.ts:20,21,23` hand-writes `pi.zod.enum(["readOnly", "write"])`, `pi.zod.enum(["shared", "gitWorktree", "copy"])`, and `pi.zod.enum(["applyPatch", "cherryPick"])` — literal-for-literal copies of generated `LeaseMode`, `IsolationKind`, and `ApplyStrategy` respectively, with no import tying them together. A future variant added to any of the three Rust enums silently desyncs the tool schema from the wire type. **Open.**

#### R18. Detached runtime spawn has no `error` listener

**Location:** `packages/extension/src/runtime.ts:101-106`; `packages/extension/src/status.ts:57-72`

An async `ChildProcess` error (`EAGAIN`/`EMFILE`) has no listener. **Open.**

#### R19. Role documentation understates the worker-accessible surface

**Location:** `docs/architecture.md:750-758`; `crates/runtime/src/ipc/mod.rs:226-300`

Document currently says 22/9 methods, code allows 30/12 when this claim was written — [OK] RESOLVED (re-verified 2026-08-08): `architecture.md:758,760` now reads "All 29 mutation/read methods" for `ompExtension` and "12 methods" for `workerMcp`, explicitly naming `coordination/peerWorkspace`, `coordination/artifactList`, and `coordination/artifactFetch` in the worker row — an exact match for `ipc/mod.rs:244-304`'s `allowed_methods()` (29 and 12 respectively, counted directly). Fixed by other work since this item was filed; no doc change needed now.

#### R33. `serde_json/preserve_order` silently breaks two fingerprint invariants and one secret-shape gate

**Location:** `Cargo.toml:22`; `crates/runtime/src/adapter/profile.rs:325-326,364-369`; `crates/runtime/src/config/merge.rs:517-521`; `crates/runtime/src/security/redaction.rs:262-265,304-310`

**Evidence (verified):** `Cargo.toml:22` enables `serde_json`'s `preserve_order` feature — a workspace-wide flip from `BTreeMap` to `IndexMap` key ordering that no plan item requested. Three doc comments (`profile.rs:325-326`, `redaction.rs:262-265`) still assert the workspace does not enable it and that `sanitize_json`/`fingerprint()` are key-order-independent; that guarantee no longer holds. `WorkerProfile::fingerprint()` and `RuntimePolicy::compute_fingerprint()` now vary with caller/merge key order, so no fingerprint an older binary persisted can be reproduced. A determinism test (`sanitize_json_is_deterministic_regardless_of_input_key_order`) was weakened to a tautology rather than fixed. Additionally, `permission_envelope_contains_secret_shape` (`profile.rs:364-369`) compares `serde_json::to_string(value)` (insertion order) against `sanitize_json(value)` (its own order) — under `preserve_order` these now diverge for any envelope whose keys are not already in `sanitize_json`'s emitted order, causing **false rejection of legitimate worker profiles**.

**Fix:** Canonicalize `redact_json_value`'s Object arm to a key-sorted map (restores `sanitize_json` determinism and `fingerprint()`'s content-addressing property), and change `permission_envelope_contains_secret_shape` to a structural comparison (`redactor.redact_json_value(value) != *value`) rather than a serialized-text comparison. Restore the deleted determinism test.

**Priority:** High — breaks two documented invariants and introduces a false-rejection security regression.

#### R34. `decided_by` persisted as a JSON-quoted string, not a bare token

**Location:** `crates/runtime/src/domain/repository.rs:805`

**Evidence (verified):** `serde_json::params![... , serde_json::to_string(&decided_by).expect(...) , ...]` stores `"human"` (with quotes) instead of bare `human`. Every other scalar enum column in this file writes a bare token. `SELECT * FROM approvals WHERE decided_by = 'human'` returns zero rows permanently.

**Fix:** Add `DecidedBy::as_str()` returning the bare token and use it in the `UPDATE` at line 805.

**Priority:** Medium — breaks any future query or audit tooling against this column; no runtime behavior currently reads it back.

#### R35. `artifact/fetch` authorizes after reading and hashing content

**Location:** `crates/runtime/src/service/orchestration.rs:1288-1346`

**Evidence (verified):** `fetch_chunked` (line 1329) runs and hashes/reads full content before the ownership check (`in_scope`, line 1337-1341). The single shared refusal message (line 1334 vs 1344) prevents an *unknown-vs-out-of-scope* content oracle, but the two paths differ in latency — fetch does real I/O and hashing before an out-of-scope caller is rejected, an unknown ID is rejected immediately. This is a timing side-channel distinguishing "exists but not yours" from "doesn't exist."

**Fix:** Look up artifact metadata only, authorize against `run_id`, then call `fetch_chunked` for content.

**Priority:** Medium — genuine but low-severity side channel (requires an attacker capable of measuring server-side artifact-store latency).

#### R36. No test asserts artifact producers actually stamp `run_id`

**Location:** `crates/runtime/tests/workspace_apply.rs:241,292,357,452,486`; `crates/runtime/src/workspace/apply.rs:105`; `crates/runtime/src/workspace/inspect.rs:77`

**Evidence (verified):** Both isolation tests hand-seed `run_id` via `seed_artifact` rather than exercising the actual producers. Reverting `apply.rs:105` and `inspect.rs:77` to `run_id: None` leaves the whole suite green — R10's fix would silently regress with no failing test, since `run_id: None` artifacts are invisible to every principal (fails closed, but silently).

**Fix:** Add `a_worker_sees_the_conflict_artifact_its_own_run_produced` and `workspace_inspect_stores_a_fetchable_patch_artifact`, asserting `report.run_id` on the artifact actually produced by `WorkspaceApplier`/`WorkspaceInspector`, not a hand-seeded one.

**Priority:** Medium — test-coverage gap on a security-relevant fix, not a live defect.

### Low

#### R20. Installed users cannot run the documented bare `batcave` commands

**Location:** `packages/batman-linux-x64-gnu/package.json:1-17` and peer leaves; `README.md:28-47`; `docs/operations.md:7-75`

Leaf packages declare no npm `bin` shim (`packages/batman-linux-x64-gnu/package.json`'s `exports` maps only `"."`/`"./package.json"`, no `bin` field) and every doc example invokes bare `batcave ...` (`operations.md:16,37,49,52,64`; `README.md`) as if it were on `PATH`. **Open** — re-verified 2026-08-08.

#### R21. Documentation names CLI flags that do not exist — ✅ RESOLVED (fixed during this consolidation)

**Location:** `docs/getting-started.md:148,239,355,390`; `docs/code-walkthrough.md:396-405`; `docs/operations.md:66`; `AGENTS.md:91`; `CLAUDE.md:61`; `crates/runtime/src/cli.rs:29-152`

**Evidence (re-verified 2026-08-08):** `docs/getting-started.md` and `docs/operations.md` already correctly documented that no `--recover`/`--port`/`--live` flags exist. Two regressions had crept back in since the original fix: `AGENTS.md:91`/`CLAUDE.md:61` (generated after the original review) reintroduced `batcave status [--recover]`, and `docs/code-walkthrough.md:401-403` carried both a false claim ("RecoveryCoordinator is now dead code" — contradicted by `lifecycle.rs:150`, which calls it on every `serve`) and the same nonexistent `--recover` flag.

**Fix applied:** `AGENTS.md`/`CLAUDE.md` CLI examples corrected; `code-walkthrough.md`'s recovery paragraph rewritten to match `docs/getting-started.md`'s existing correct explanation (no on-demand recovery trigger; use `doctor`'s `stale_runs` check instead).

#### R22. Getting-started command examples omit required `--repo` — ✅ RESOLVED (fixed during this consolidation)

**Location:** `AGENTS.md:90-93`; `CLAUDE.md:60-63`; `crates/runtime/src/cli.rs:29-165`

**Evidence (re-verified 2026-08-08):** `docs/getting-started.md`, `docs/operations.md`, and `docs/manual-testing.md` already include `--repo` in every example. Only `AGENTS.md:90-93`/`CLAUDE.md:60-63` still omitted it, reintroducing the original defect after the docs/ fix.

**Fix applied:** Added `--repo /path/to/repo` to every `serve`/`status`/`stop`/`audit export` example in `AGENTS.md`/`CLAUDE.md`.

#### R23. Tool documentation describes eight tools while eleven are registered — ✅ RESOLVED (already fixed by other work, not this pass)

**Location:** `docs/architecture.md:196-207`; `docs/code-walkthrough.md:124-135`; `docs/manual-testing.md:202`; `packages/extension/src/tools/index.ts:38-51`

**Evidence (re-verified 2026-08-08):** `architecture.md:200` lists all 11 tools by name (`batman_profile`, `batman_worker`, `batman_task`, `batman_run`, `batman_workspace`, `batman_artifact`, `batman_child`, `batman_violation`, `batman_message`, `batman_approval`, `batman_reconcile`), matching `index.ts:41-51`'s registration order exactly. `code-walkthrough.md:138` says "Registers all 11 tools with OMP"; `manual-testing.md:206` lists the same 11 by name. No doc omits artifact/child/violation. Stale when filed; already corrected elsewhere.

#### R24. Current docs name two deleted TypeScript modules — ✅ RESOLVED (already fixed by other work, not this pass)

**Location:** `docs/code-walkthrough.md:125`; `docs/architecture.md:207`

**Evidence (re-verified 2026-08-08):** Neither `config.ts` nor `conformance/index.ts` is referenced at the cited lines or anywhere in the current `code-walkthrough.md`/`architecture.md` extension-component tables; both files are confirmed absent from `packages/extension/src/`. Stale when filed; already corrected elsewhere.

#### R25. The release checklist and compatibility guide retain disproven stub claims — narrowed (partially stale)

**Location:** `release/0.1.0-checklist.json`; `docs/compatibility.md:189`

**Evidence (re-verified 2026-08-08):** `docs/compatibility.md` is now only 136 lines — line 189 does not exist. The file was narrowed to exactly two tables (supported platforms, adapter conformance versions) and explicitly defers everything else to other docs (`compatibility.md:3-8`); it makes none of the stub claims this item originally cited. That half is moot. `release/0.1.0-checklist.json` is a dated, timestamped snapshot (`"generated": "2026-08-01"`) of a specific hardening pass, structurally like `docs/journal.md` — a point-in-time record, not a current-state doc; its stale entries (e.g. `task_3_recovery_tests` calling `recovery.rs` "still a stub") describe what was true on 2026-08-01, not a live claim about today's `RecoveryCoordinator` (confirmed non-stub, wired into `lifecycle.rs:150`). **Open** only if this file is meant to be a living doc rather than a dated snapshot — recommend explicitly labeling it historical (matching `journal.md`'s convention) to close this permanently, or deleting it if the 0.1.0 release already shipped.

#### R26. Compatibility docs omit three shipped coordination methods — ✅ RESOLVED (moot — already fixed by other work, not this pass)

**Location:** `docs/compatibility.md:172-189`; `crates/protocol/src/method.rs:79-84`

**Evidence (re-verified 2026-08-08):** `docs/compatibility.md` (136 lines total) no longer contains a coordination-method list of any kind — it was narrowed to platform support + adapter conformance tables only, with protocol methods explicitly deferred to `architecture.md` (line 7). There is nothing left to omit. Stale when filed; already corrected elsewhere.

#### R27. Uninstall and rollback instructions use nonexistent distribution channels — ✅ RESOLVED (already fixed by other work, not this pass)

**Location:** `docs/operations.md:180-231`; `README.md:28-47`

**Evidence (re-verified 2026-08-08):** `operations.md`'s Install/Upgrade/Uninstall section (92 lines total in the file; cited range 180-231 is past EOF) explicitly states "**There is no Homebrew formula, apt/deb/rpm package, or any other system package**" and documents only `omp install @nikolasd/batman` / `omp plugin uninstall @nikolasd/batman`. It cites no Homebrew/apt path as real. Stale when filed; already corrected elsewhere.

#### R28. Manual-testing guidance contradicts itself about CLI conformance — ✅ RESOLVED (moot — no contradiction found, not this pass)

**Location:** `docs/manual-testing.md:341-342`; `crates/runtime/src/cli.rs:139-152`

**Evidence (re-verified 2026-08-08):** `manual-testing.md:129-132` says the CLI's `serve`/`status`/`stop`/`schema` subcommands are "fully implemented"; `:344-350` says `batcave conformance`/`batcave adapters` "run the same fixture/live suites as the `cargo test` commands below" — consistent with `cli.rs:139-153`'s real `Conformance`/`Adapters` subcommand definitions (not stubs). No internal contradiction found in the current text. Stale when filed; already corrected elsewhere or never reproduced.

#### R29. `workspaceMode` is an open string

**Location:** `packages/extension/src/tools/runs.ts:13-21`; `crates/runtime/src/service/orchestration.rs:649-656`

Runtime rejects unknown values safely; a closed Zod enum would avoid round trips. **Open.**

#### R30. Local Bun scripts omit top-level conformance and install tests

**Location:** `package.json:10-13`; `.github/workflows/ci.yml:75-83`

**Open.**

#### R31. CONTRIBUTING references a cargo-features example with no real features to substitute — narrowed (2 of 3 original sub-claims false)

**Location:** `CONTRIBUTING.md:34-46,143-146`

**Evidence (re-verified 2026-08-08):** Two of the three things this item claimed were "nonexistent" are not: `cargo test --test adapter_contract` / `--test approval` / `--test audit` (`:39-41`) name real integration test binaries — all three exist under `crates/runtime/tests/`. And `:144` already reads "There is no PR template — write a clear description..." — the doc correctly discloses the absence rather than falsely referencing one. The one real issue: `:45`'s `cargo test --features "feature1,feature2"` example names placeholder features that don't exist — no crate in the workspace declares a `[features]` table, so this line would error (`error: none of the selected packages contains these features`) if run literally.

**Fix:** Delete the `cargo test --features "feature1,feature2"` example (or replace with a real, currently-empty-features caveat) — no fix needed for the test-binary or PR-template lines, which are already accurate.

#### R32. The extension header lists only six of eleven orchestration tools

**Location:** `packages/extension/src/index.ts:1-4,83-99`

**Open.**

#### R37. `model.test.ts` fails to typecheck against the now-required `decidedBy` field

**Location:** `packages/extension/src/monitor/model.test.ts:240,247`; `packages/protocol-ts/src/generated/RuntimeEvent.ts`

**Evidence (verified):** The generated `RuntimeEvent` union requires `decidedBy: DecidedBy | null` on `approvalEvent` payloads (nullable, but not optional). `model.test.ts:240` and `:247` construct `approvalRequested`/`approvalDecided` payloads without the field. No `tsc` gate runs in CI (`package.json:13`, `packages/extension/package.json:10` — neither invokes `tsc`; see also **R45**), so this currently ships uncaught.

**Fix:** Add `decidedBy: null` at :240 and `decidedBy: "human"` at :247.

#### R38. `install_frame_tap` exported on the crate's public API surface

**Location:** `crates/runtime/src/supervisor/mod.rs:14-16`

**Evidence (verified):** `pub use output::{..., install_frame_tap};` re-exports the raw-content capture bypass beyond the crate boundary. A comment states "production never installs one," but nothing prevents an external caller in the same binary from doing so.

**Fix:** Narrow the re-export to `pub(crate)` unless a cross-crate consumer is intended.

#### R39. `session_shutdown` doesn't clear `subscribedClient`, breaking future monitor reconnects

**Location:** `packages/extension/src/monitor/controller.ts:148-150`

**Evidence (verified):** The repair path (lines 109-112) correctly pairs `controller.stop()` with `subscribedClient = undefined`. The `session_shutdown` handler (lines 148-150) calls only `controller.stop()`. After shutdown the client is unsubscribed but not closed, so `isClosed` stays false and a later `connect()` early-returns at line 106 into a permanently dead monitor.

**Fix:** `pi.on("session_shutdown", async () => { controller.stop(); subscribedClient = undefined; });`

#### R40. `reconnect.test.ts` passes a `revision` param the tool schema doesn't define

**Location:** `packages/extension/src/reconnect.test.ts:129,137`; `packages/extension/src/tools/tasks.ts:23-27`

**Evidence (verified):** `batman_task`'s Zod schema (`tasks.ts:23-27`) has only `op`, `description`, `taskId` — no `revision`. `reconnect.test.ts:129,137` passes `revision: 1`/`revision: 2`, which is silently dropped; the tool always sends `INITIAL_TASK_REVISION` (0) internally regardless. The test still proves what it's named for (cache self-heals after daemon restart) but the `revision` field is dead weight that could mislead a future reader into thinking revision handling is exercised.

**Fix:** Remove `revision` from the test's tool-call arguments.

#### R41. `start_queued_run` can leak a lease and materialized worktree on adapter-start failure

**Location:** `crates/runtime/src/service/orchestration.rs:613-615`

**Evidence:** reported by ReviewerOrchestration (S4); not independently re-verified line-by-line in this pass. If `driver.start` fails after `lease_service.activate`, the lease remains active and the materialized worktree remains on disk. Promoted from Suggestion to a tracked finding because `run/retry` (R7) multiplies how often this path executes.

**Fix:** Release the lease and clean up the worktree on `driver.start` failure, mirroring the existing post-authorize error paths from R2.

#### R42. `ProtocolHealthChanged` detail interpolates the normalized string, not the raw stop reason

**Location:** `crates/runtime/src/adapter/copilot/normalize.rs:190-194`

**Evidence (verified):** The unknown-reason arm does `format!("unknownStopReason: {other}")` where `other` is the already-lowercased/stripped match value (e.g. `copilotquotaexhausted`), not the original vendor `stop_reason` string (e.g. `_copilot_quota_exhausted`). The detail is harder to grep against vendor docs than it needs to be.

**Fix:** Interpolate the original `stop_reason` parameter instead of the normalized match binding.

#### R43. `artifacts.ts` tool description contradicts its actual ownership-based scope

**Location:** `packages/extension/src/tools/artifacts.ts:27`; `docs/plugin-usage.md:121-122`

**Evidence (verified):** Tool description states artifacts are "scoped to the current task," but R10's fix scopes by `owner_client_instance_id` (session ownership), with `taskId` only as optional narrowing. `docs/plugin-usage.md:121-122` repeats the same false claim.

**Fix:** Update both to state session-ownership scoping with optional task narrowing.

#### R44. Conformance fixture-capture pipeline: scrubber over-fit, `unchanged` flag inverted, TS baseline gate never written

**Location:** `crates/runtime/src/conformance/scrub.rs:182-205`; `crates/runtime/src/conformance/capture.rs:156-158,68-69`; `fixtures/conformance/fixture-mode-baseline.json`

**Evidence (verified by ReviewerConformance, quoted directly):**
1. "The scrubber is calibrated to one fixture. Its placeholder prefixes match `claude/initialize.jsonl` and nothing else, so `batcave capture` would rewrite the other 10 committed fixtures instead of reproducing them, flattening codex's monotonic timestamps to a single constant and turning `claude/result.jsonl`'s `error_max_turns` values ... into the success fixture's."
2. "The guard against exactly that is a constant. `unchanged` is computed after `fs::write`, so it compares the file to what was just written: `true` on every capture, `false` on every dry run. Inverted and useless." (Doc at `capture.rs:68-69` says "before write," contradicting the actual computation point.)
3. "`fixture-mode-baseline.json` has one consumer, not two. The CLI gate is sound ... but `run.ts:40` throws on the CLI's exit code before the report is ever validated, so the TS gate that plan step 6.3 required ... was never written."

ReviewerConformance reported **8 errors total**; the three above are the ones quoted verbatim in the collected report. **5 further conformance errors were counted but not itemized with file:line in the collected report and are not restated here to avoid fabricating detail** — re-dispatch ReviewerConformance or re-read its full output before treating the count as closed.

**Fix:** Calibrate the scrubber against all 11 fixtures, not one. Compute `unchanged` before `fs::write`. Write the TS-side baseline validation gate (`tests/conformance/assert-report.ts` or equivalent) so a hand-edited `passed: true` report is caught independent of the CLI's exit code.

**Priority:** High — the capture tool that produces committed fixtures is not proven correct beyond the one fixture it was built against.

#### R45. No TypeScript compiler gate anywhere in CI or local scripts

**Location:** `package.json:13`; `packages/extension/package.json:10`

**Evidence (verified):** Neither script list invokes `tsc`. `tsconfig.json` declares `strict: true`, and `packages/protocol-ts/src/generated/` is the authoritative wire contract for the whole protocol, but nothing proves a single TypeScript consumer still compiles against it. This is why **R37** shipped uncaught.

**Fix:** Add `bunx tsc --noEmit` to `bun run check` and to CI.

#### R46. Documentation drift introduced alongside the R5-R11 fixes

**Location:** `TODO.md:157` (former); `REVIEW.md` R7 (former); `docs/manual-testing.md` §4b exception table row 1

**Evidence (verified):** Former `TODO.md` item #74 was still marked "Open" although R7's fix (`start_queued_run`) landed and is tested — corrected by folding TODO.md into this file with R7 marked resolved above. Former `REVIEW.md` R7 carried a stale "Suggested fix" instructing not to describe retry as execution — removed above. `docs/manual-testing.md` §4b lists `ompRpc/approval` as an expected fixture-mode failure; the committed baseline records `ompRpc: []` (zero expected failures) — correct the exception table.

**Fix:** Correct `docs/manual-testing.md` §4b. The TODO.md/REVIEW.md staleness is resolved by this consolidation.

## Known Environment Limitations (folded in from former TODO.md #55)

**Not a bug — requires a gated live run to confirm the positive case.**

- **Codex** (`follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, `result_usage_artifacts`): blocked on account credits (`usageLimitExceeded: Your workspace is out of credits.`), not code. `codex login status` reports authenticated; `initialize`/`thread/start` succeed; the turn is refused server-side after ~3s. Refill credits and these become provable with no code change. Report: `release/live-codex.json`.
- **Adapter fix already shipped:** the vendor `error` notification was previously dropped by `codex/normalize.rs`; it now normalizes to `ProtocolHealthChanged{healthy:false}` and the live probe fails fast with the vendor's own text (62s → 5s). Defended by `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event`.
- **Copilot** (`session_resume`, `runtime_restart`): a genuine ACP v1 protocol wall — `session/load` answers "Resource not found" for a session that completed a real turn in a prior process. Distinct from the Codex account condition; the check (`copilot/conformance.rs::session_resume_probe`) is written to pass automatically if a future CLI version persists sessions differently.

Prove these via `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance runs when a licensed, billed run is acceptable. No code change needed.

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c

## Strengths

- `DomainRepository::append_and_apply` keeps event append and projection updates in one SQLite transaction; migration ordering is sequential and atomic.
- Recovery completes before the runtime binds its socket, so clients cannot mutate pre-recovery state.
- Subscription forwarders exit on writer closure, with a regression test that fails against the prior behavior.
- Binary selection verifies package version and SHA-256 before spawn.
- TypeScript and Rust derive repository IDs from the same fixtures, including worktree and broken-symlink cases.
- Inbound client frames are size-checked and schema-validated before dispatch.
- OMP-native reconciliation prevents stale coalesce timers from overwriting terminal facts and marks orphaned non-terminal runs as lost.
- The policy evaluator checks model, adapter, required-capability, nested-discovery, cost, and concurrency dimensions; the slot-release lifecycle defect (R2) is fixed and defended.
- Release targets and reproducible manifest timestamps have single sources of truth; package-set independently verifies target, checksum, schema fingerprint, and executable mode.
- Committed live conformance evidence matches the compatibility table: Claude 14/14, Codex 9/14, Copilot 11/14, OMP-RPC 14/14.
- R5-R11 (approval enforcement, client reconnection, retry execution, conformance gating, release versioning, artifact isolation, Copilot stop reasons) are all functionally fixed and defended by new tests; the R33-R44 findings above are regressions or gaps introduced by those fixes, not evidence the original defects remain open.

## Areas reviewed

- Rust runtime: IPC, services, domain repository, database migrations, recovery, lifecycle, workspace/artifacts, adapters, policy, security, supervisor, conformance, CLI, xtask.
- TypeScript: extension runtime/client/status, all orchestration tools, OMP-native persistence/reconciliation, generated protocol package, conformance/install tests, monitor.
- Delivery: CI/release workflows, target matrix, npm leaf packages, provenance/package-set logic.
- Current documentation: README, CONTRIBUTING, AGENTS.md, CLAUDE.md, architecture, operations, getting started, compatibility, manual testing, plugin-usage, code walkthrough. ADRs and journal entries were treated as immutable point-in-time records rather than current-state docs.

## Open Item Count

- **Critical:** 0 open (R1-R4 resolved)
- **High:** 0 fully open; R5-R11 resolved with 4 follow-on findings (R33, R35, R36, R44)
- **Medium:** R12-R18, R34, R37, R38, R39, R41, R42, R45 — 14 open
- **Low:** R20, R25 (narrowed), R29, R30, R31 (narrowed), R32, R40, R43, R46 — 9 open, mostly documentation (R19, R23, R24, R26, R27, R28 resolved 2026-08-08 — already fixed by other work, not this pass; see entries above for evidence)
- **Environment (not actionable in-repo):** former TODO #55, folded in above
