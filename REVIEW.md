# BATMAN Open Items

**Purpose:** the single source of truth for unfinished implementation gaps against current source —
an open-items backlog, not an audit trail. Every item below was verified against the current tree in
this pass; nothing here is carried forward on trust from an earlier snapshot.

**Reviewed:** 2026-08-06 (baseline) · **Re-verified/expanded:** 2026-08-12 — full re-verification of
every previously-open item plus a fresh sweep for new gaps across runtime core, adapters/conformance,
TypeScript/workspace, and build/docs/release, split across four parallel reviewers by locality
(matching the original review's method). Twenty new findings surfaced (R47-R66); four existing
findings were corrected in place (R17, R20, R43, R46) where the mechanism had changed since last
verified.

**Resolution history moved:** everything that was Critical/High and is now resolved (R1-R11, R33, R41, R47-R54, R68-R70) plus the
eleven documentation findings that were resolved or already-stale (R19, R21-R28) has been pruned from
this document. That history — what broke, the fix commit, the test that proved it, and which
still-open items below exist *because* of that fix — now lives in
[`docs/journal.md` Part X](journal.md#part-x--reviewmds-second-pass-seven-more-fixes-eleven-doc-corrections-and-the-residue-that-outlived-them)
(R1-R11, R47), [Part XI](journal.md#part-xi--halving-the-critical-pair-a-ceiling-that-could-not-be-enforced) (R48),
[Part XII](journal.md#part-xii--closing-the-last-critical-a-denylist-blind-to-its-own-vendor) (R49),
[Part XIII](journal.md#part-xiii--two-leaks-one-lease-releasing-what-a-failed-start-acquired) (R41, R50),
[Part XIV](journal.md#part-xiv--fixture-modes-broken-promise-a-kill-switch-only-one-caller-ever-asked-about) (R52),
[Part XV](journal.md#part-xv--crash-recoverys-five-minute-blind-spot-the-one-crash-it-could-not-see) (R51), [Part XVI](journal.md#part-xvi--a-state-machine-with-no-production-writer-closing-the-last-critical) (R69), [Part XVII](journal.md#part-xvii--skipped-is-not-fail-the-discriminator-r68-asked-for) (R68), [Part XVIII](journal.md#part-xviii--one-guard-three-doors-the-two-coordination-calls-that-journaled-unmetered) (R53), [Part XIX](journal.md#part-xix--two-decisions-one-violation-the-guard-that-lived-outside-the-transaction) (R54), [Part XX](journal.md#part-xx--the-same-race-one-service-over-the-approval-that-could-be-decided-twice) (R70), and [Part XXI](journal.md#part-xxi--a-feature-flag-for-one-tool-three-broken-content-addresses) (R33).
This document only tracks what's still broken.

**Baseline, last run 2026-08-12** (during an unrelated state-root rename; results apply to this
snapshot):

- `cargo test --workspace` (`BATMAN_DISABLE_VENDOR_CLI=1`) — all suites pass except one pre-existing,
  environment-specific failure unrelated to any item below: `copilot_adapter::real_binary_initialize_and_session_list_never_invoke_a_model`
  fails because the local machine's installed Copilot CLI (1.0.79) isn't in `COPILOT_KNOWN_CLI_VERSIONS`
  yet — a local-environment gap, not a code defect (see R57 for a related but distinct gap in the same
  version-check machinery).
- `bun test packages` — 139 passed, 0 failed.
- `cargo fmt --all --check`, `biome format .`, `bun run generate --check` — all clean.

**How to read priority:** ranked against end-to-end functionality completeness — the full task →
run → worker → adapter → events → completion lifecycle across all four adapters, plus the
release/install/distribution path and documentation a user or contributor actually depends on to
operate the system correctly.

## Findings

### High

#### R44. Conformance fixture-capture pipeline: scrubber calibrated to one fixture, `unchanged` flag computed after the write it's supposed to guard

**Location:** `crates/runtime/src/conformance/scrub.rs:182-245`; `crates/runtime/src/conformance/capture.rs:67-69,151-158`

**Evidence (sub-claim 1 — scrubber over-fit):** `stable_session_id`/`stable_uuid` (`scrub.rs:182-205`) only recognize `claude/initialize.jsonl`'s `11111111-…`/`a0000000-…` placeholder family as already-canonical. The other ~10 committed fixtures use distinct ID families (`claude/subagent.jsonl`, `claude/approval.jsonl`, `claude/result.jsonl` each use a different prefix; codex/copilot/omp-rpc fixtures use raw-looking IDs). The one round-trip test, `scrubbing_scrubbed_fixture_is_identity` (`scrub.rs:227-245`), is hardcoded to `claude/initialize.jsonl` only — no test proves idempotence against any of the other fixtures, meaning a real `batcave capture` run today would rewrite (not reproduce) most of the fixture set.

**Evidence (sub-claim 2 — inverted `unchanged` flag):** `capture.rs:151-158` writes the file (`fs::write`) first, then computes `unchanged` by reading that same just-written file back and comparing it to the content just written — always `true` on a real capture, always `false` on a dry run, regardless of whether the captured content actually matches what was committed before the write. The field's own doc comment (`capture.rs:67-69`) says "before write," which is not what the code does.

**Fix:** calibrate the scrubber against all eleven fixtures' actual ID families, not one; compute `unchanged` by comparing against the pre-write committed content (read the file before `fs::write`, or diff against `git show HEAD:<path>`).

**Priority:** High — the tool that produces every committed conformance fixture is unproven correct beyond the single fixture it was built against.

#### R71. `ApprovalService::decide`'s ownership pre-check races `reconcile/omp`'s task ownership rebind

**Location:** `crates/runtime/src/approval/service.rs:172-179` (`ApprovalService::decide`'s ownership check); `crates/runtime/src/domain/repository.rs:1124` (`UPDATE tasks SET owner_client_instance_id ...`, the `reconcile/omp` ownership rebind); `crates/runtime/src/domain/repository.rs:792-883` (`decide_approval`'s guarded write, which never re-checks the task's current owner)

**Evidence:** `decide` (`service.rs:172-179`) reads a snapshot once, then compares `snapshot.owner_client_instance_id` to the caller's `principal_instance_id` entirely in memory, before ever reaching the guarded domain write. `decide_approval`'s transaction (`repository.rs:792-883`, hardened for R70) guards only `decision IS NULL` and the run's terminal state — it never re-reads `tasks.owner_client_instance_id`. Between the snapshot read and that write, `reconcile/omp` can rebind the task to a new owner via the unguarded `UPDATE tasks SET owner_client_instance_id = ?1, revision = ?2, updated_at = ?3 WHERE task_id = ?4` (`repository.rs:1124`) and commit. The old owner, having already passed the stale pre-check, still reaches and wins the guarded write — deciding an approval for a task it no longer owns. This is distinct from R70's decision-row race (two callers racing the same decision): here exactly one decider races a *rebind*, not another decider. Noted but deliberately left unfixed by R70's own adversarial review (`docs/journal.md` Part XX: "`tasks.owner_client_instance_id` is separately mutated by the reconcile path's ownership rebind, a real, reachable interleaving between reconcile and decide, but not R70's mechanism and out of this fix's scope").

**Fix:** move ownership authorization into `decide_approval`'s guarded transaction instead of trusting the pre-check snapshot — either re-read `tasks.owner_client_instance_id` inside the same transaction that performs the `UPDATE approvals` and reject on mismatch, or thread the caller's `principal_instance_id` into `decide_approval` and add it as a `WHERE` condition (joined against `tasks`) so a rebind that lands before the write invalidates it. Add a deterministic regression test that interleaves a `reconcile/omp` ownership rebind between the snapshot read and the guarded write and asserts the stale owner's `decide` is refused, mirroring `approval_decide_race.rs`'s `join!(biased; ...)` pattern.

**Priority:** High — a real, reachable race that lets a caller decide an approval for a task it no longer owns, violating the owner-only decision contract; same severity class as R70/R54 (found during R70's adversarial review, 2026-08-18; not fixed as part of that change since it is a distinct interleaving — reconcile vs. decide, not decide vs. decide — outside R70's stated mechanism).

### Medium

#### R12. Claude error result subtypes are normalized as usage only

**Location:** `crates/runtime/src/adapter/claude/protocol.rs:183-194`; `crates/runtime/src/adapter/claude/normalize.rs:254-269`; `crates/runtime/tests/claude_adapter.rs:606-635`

`RawResult` omits `subtype`/`is_error`. The committed `error_max_turns` fixture emits only `UsageReported` — `result_fixture_error_arm_reports_usage_without_a_final_message` asserts exactly one payload. Model the failure discriminators and emit an explicit terminal failure event.

#### R13. Policy cancellation records success after a process-kill failure

**Location:** `crates/runtime/src/policy/violation.rs:446-472`; `crates/runtime/src/adapter/registry.rs:333-348`

A failed cancellation (including `RegistryError::NoRunningAdapter`, which is not a kill failure at all) is logged via `tracing::warn!`, then the run is unconditionally transitioned to `cancelled` anyway. Distinguish "no running adapter" from "kill failed" and only the latter should avoid a clean `cancelled` transition.

#### R14. Per-run redactor construction has a fail-open fallback (currently dead code, trap remains)

**Location:** `crates/runtime/src/lifecycle.rs:194-205`; `crates/runtime/src/adapter/event_sink.rs:164-167`

`DomainAdapterEventSink::new` falls back to `Redactor::new()` (built-in patterns only) via `unwrap_or_else` if org regex compilation fails, rather than propagating. Confirmed unreachable today — `lifecycle.rs` validates `policy.org_security_patterns` once at startup and fails closed, and the one call site (`registry.rs:480`) only ever receives that pre-validated policy — but the trap remains in source and would silently activate the moment any future path (config reload, an alternate constructor) feeds it unvalidated patterns.

#### R15. `batman_task.description` is silently discarded

**Location:** `packages/extension/src/tools/tasks.ts:20-47`; `crates/runtime/src/service/orchestration.rs:300-330`

The tool schema advertises `description`, but `task/upsert`'s RPC payload never includes it, and the Rust handler has no field to receive it — `runs.ts:15` documents the workaround (pass task text into `batman_run.prompt` instead).

#### R16. Violation resolution schema accepts prose the runtime rejects

**Location:** `packages/extension/src/tools/violations.ts:20`; `crates/runtime/src/policy/violation.rs:491-495`

Tool schema is an unconstrained `pi.zod.string()`; runtime accepts only the literal strings `"release"`/`"cancel"`. Use a closed Zod enum.

#### R34. `decided_by` persisted as a JSON-quoted string, not a bare token

**Location:** `crates/runtime/src/domain/repository.rs:805`

`serde_json::to_string(&decided_by)` stores `"human"` with quotes instead of bare `human`, unlike every other scalar-enum column in this file. `SELECT * FROM approvals WHERE decided_by = 'human'` returns zero rows permanently. Add `DecidedBy::as_str()` and use it in the `UPDATE`.

#### R35. `artifact/fetch` authorizes after reading and hashing content

**Location:** `crates/runtime/src/service/orchestration.rs:1327-1346`

`fetch_chunked` runs and hashes/reads full content before the ownership check. The shared refusal message prevents a content-based oracle, but the two paths differ in latency — a timing side-channel distinguishing "exists but not yours" from "doesn't exist." Look up metadata only, authorize against `run_id`, then fetch content.

#### R36. No test asserts artifact producers actually stamp `run_id`

**Location:** `crates/runtime/tests/workspace_apply.rs:241,292,357,452,486`; `crates/runtime/src/workspace/apply.rs:105`; `crates/runtime/src/workspace/inspect.rs:77`

Every isolation test hand-seeds `run_id` on its input fixture rather than asserting the value the real `WorkspaceApplier`/`WorkspaceInspector` actually stamps on production. Reverting the real stamping code to `run_id: None` leaves the whole suite green — R10's fix would silently regress with no failing test.

#### R37. `model.test.ts` fails to typecheck against the required `decidedBy` field (confirmed by a real compile)

**Location:** `packages/extension/src/monitor/model.test.ts:77,240,247`

A scoped `tsc --noEmit` run reproduces three real errors: `decidedBy` missing at lines 240/247 (required, not optional, on the generated `RuntimeEvent` union), and an unrelated `pendingApprovalCount` property that doesn't exist on `MonitorState` at line 77. All three ship uncaught because no compiler gate runs anywhere (R45).

#### R42. `ProtocolHealthChanged` detail interpolates the normalized string, not the raw stop reason

**Location:** `crates/runtime/src/adapter/copilot/normalize.rs:153-199`

The unknown-reason arm does `format!("unknownStopReason: {other}")` where `other` is the already-lowercased, `_`/`-`-stripped match binding — not the original `stop_reason` parameter, which remains correctly used two lines later in a different message. Interpolate `stop_reason` instead.

#### R45. No TypeScript compiler gate anywhere in CI or local scripts

**Location:** root `package.json` scripts; `packages/extension/package.json` scripts

Neither script list, nor any GitHub Actions workflow, invokes `tsc`. `tsconfig.json` declares `strict: true` and the generated protocol package is the authoritative wire contract, but nothing proves a single TypeScript consumer still compiles against it — the direct cause of R37, R61, and the `pendingApprovalCount` error above shipping uncaught. Note when fixing: the root `tsconfig.json`'s `"module": "Bundler"` is rejected by the currently-pinned `typescript` version for a standalone `tsc --noEmit` invocation — a scoped override config was needed to even run the check manually; resolve that as part of wiring the gate in.

#### R55. Nearly every orchestration RPC result is never Ajv-validated, contradicting the codebase's documented validation invariant

**Location:** `packages/extension/src/client.ts:96-105` (`request()`); `packages/extension/src/tools/shared.ts:33`; `packages/protocol-ts/schema/batman.schema.json` (`JsonRpcResponse.result`)

**Evidence:** `client.ts`'s own header comment claims every inbound message is Ajv-validated before reaching caller code, but `request()` only special-cases `runtime/status`. Every other method — `task/upsert`, `run/submit`, `workspace/acquire`, `artifact/fetch`, all 20+ others — returns `message.result` guarded only by `validateJsonRpcResponse`, whose generated schema for `result` is the JSON-Schema `true` node: always passes, no shape constraint. `packages/protocol-ts/src/validate.ts` exports validators for exactly `InitializeResult`, `RuntimeStatus`, `EventEnvelope`(+array), and the three JSON-RPC envelope shapes — none for any orchestration-method result. Root cause: most orchestration RPC results are built as ad hoc `json!({...})` in `orchestration.rs` rather than `#[derive(TS)]` structs in `crates/protocol/`, so there's no canonical type to generate a schema from.

**Fix:** define real protocol types for each RPC method's result and generate/wire per-method validators, or at minimum validate structurally against a hand-written schema per method.

**Priority:** Medium-High — systemic, currently masked only by daemon and extension coming from the same trusted build; a malformed result (missing `path`, truncated `contentBase64`) would reach tool logic completely unchecked.

#### R56. `/batman-status` never triggers a widget render — the monitor stays invisible until a run event fires

**Location:** `packages/extension/src/monitor/controller.ts:100-146`; `packages/extension/src/index.ts:65-84`

**Evidence:** `/batman`'s command handler calls `connect(cmdCtx)` then unconditionally `refresh(cmdCtx)` (`controller.ts:143-144`). `session_start` (`controller.ts:130-132`) calls only `connect(extCtx)`, which registers `refresh` as the *future* event callback (`controller.ts:121`) but never calls it immediately — so if no runs are active, no events fire, and no render occurs. `/batman-status` (`index.ts:65-84`) is a fully separate `registerCommand` from `registerMonitor` (`index.ts:84`) with zero interaction with the monitor controller, `refresh`, or `setWidget` at all.

**Fix:** call `refresh(extCtx)` once immediately after a successful `connect()` in the `session_start` handler, so the widget renders on startup (showing "No BATMAN runs yet." if empty) rather than only after the first event.

**Priority:** Medium — a real, user-visible completeness gap: a healthy runtime after `/batman-status` gives no indication the monitor exists until something happens to trigger it.

#### R57. Copilot's CLI-version verification gate is silently bypassed when the vendor omits `agentInfo.version`

**Location:** `crates/runtime/src/adapter/copilot/mod.rs:202-260` (`ensure_client`)

**Evidence:** `ensure_client`'s doc comment claims the version check is unconditional — there's no opt-in to skip it. But the guard is `if let Some(version) = &negotiated.agent_version && !copilot_cli_version_known(version) { ...refuse... }` — when the real `initialize` response omits `agentInfo.version` (an ordinary optional field, never required per `client.rs:497-501`), `agent_version` is `None`, the guard body never runs, and the adapter proceeds against a completely unverified CLI with no error. Contrast with `probe()` (`mod.rs:384-406`), which correctly treats `None` as unknown via `.is_some_and(copilot_cli_version_known)` and sets `inventory_incomplete: true`. No test in `copilot_adapter.rs` exercises `ensure_client` with `agent_version: None`.

**Fix:** treat a missing `agentInfo.version` the same way `probe()` does — as unknown, not as implicitly verified — in `ensure_client`.

**Priority:** Medium — an unguarded, untested gap in a safety check whose own doc comment claims unconditional coverage.

#### R58. `CLAUDE.md`/`AGENTS.md`'s `audit export` example silently produces an empty export

**Location:** `CLAUDE.md:64`; `AGENTS.md:93`

Both read `batcave audit export --repo /path/to/repo --state-dir ~/.batman/state --output /tmp/audit.jsonl`. `docs/cli-reference.md`'s own reference explicitly warns that `audit export`'s `--state-dir` must be the actual per-repository runtime directory (`<state-root>/repos/<repository-id>/`), not a top-level state root, "or you'll silently get an empty, freshly-migrated database with zero events rather than an error." `~/.batman/state` is a flat path matching neither shape, and doesn't even use the correct `.omp/batman` state-root prefix. A contributor copy-pasting this line gets exactly the silent-empty-export footgun `cli-reference.md` was written to warn against, in the project's own primary quick-reference files.

**Fix:** correct both examples to a real per-repository runtime directory, matching `docs/getting-started.md`'s already-correct example.

#### R59. Approval `reason` is accepted end-to-end then silently discarded — never persisted

**Location:** `crates/runtime/src/domain/repository.rs:833` (`decide_approval`)

`let _ = reason;` inside the closure — the parameter is threaded from the RPC boundary through the service layer and then thrown away. No `approvals` column and no `RuntimeEvent::ApprovalEvent` field carries it. Permanent, silent audit-trail data loss on every approval decision that supplies a rationale.

**Fix:** add a `reason` column to `approvals` and persist it, or drop the parameter from the RPC contract if it's genuinely never meant to be kept.

#### R60. `Artifact`/`ArtifactKind` and related types have zero generated TypeScript bindings — not merely omitted from the barrel

**Location:** `crates/xtask/src/main.rs:197-235` (`export_bindings`'s explicit type list); `crates/protocol/src/artifact.rs:11-20`

Unlike `LeaseMode`/`IsolationKind`/`ApplyStrategy`/`WorkspaceEvent` (pulled in transitively because `RuntimeEvent`'s `WorkspaceEvent` variant references them), nothing in the explicitly-exported type set ever references `Artifact`/`ArtifactKind`, so `bun run generate` never produces bindings for them at all — confirmed clean (`generate --check` passes; this is the generator's intended steady state, not drift). `artifacts.ts`'s hand-rolled `pi.zod.enum([...])` for artifact kinds has no generated source of truth to diff against, at all — a future `ArtifactKind` variant added in Rust would silently compile, silently pass `generate --check`, and the TS tool schema would simply never learn about it. Narrower and worse than R17's barrel-omission pattern, since even the "regenerate and hand-diff" mitigation R17 implies doesn't apply here.

**Fix:** add `Artifact`, `ArtifactKind`, and the artifact request/result types to `export_bindings`'s explicit list (or a type that references them) so they generate.

### Low

#### R17. Generated TypeScript exports and hand-written enums can drift

**Location:** `packages/protocol-ts/src/index.ts` (barrel); `crates/xtask/src/main.rs:206-247`; `packages/extension/src/tools/workspaces.ts:18,20-21,23`

The barrel exports 35 types; `generated/` currently has 48 files. Confirmed still missing from the barrel: `ApplyStrategy`, `DecidedBy`, `DisplayBackend`, `DisplayConfig`, `DisplayPlacement`, `DisplayStatus`, `IsolationKind`, `LeaseMode`, `PolicyViolationId`, `RunFlags`, `RuntimeEventKind`, `WorkspaceEvent`. `workspaces.ts` hand-writes `pi.zod.enum([...])` literal copies of `LeaseMode`/`IsolationKind`/`ApplyStrategy` with no import tying them together — confirmed still byte-for-byte matching their generated definitions today, but nothing would catch a future drift. (`ArtifactKind` is no longer part of this finding — see R60, a strictly worse variant of the same root problem for that specific type.)

#### R18. Detached runtime spawn has no `error` listener

**Location:** `packages/extension/src/runtime.ts:108-113`

`spawn(binary.path, ..., { detached: true, stdio: "ignore" })` followed by `child.unref()` — no `.on("error", ...)` attached, so an async spawn failure (`EAGAIN`/`EMFILE`) is silently lost.

#### R20. Documented bare `batcave` commands assume a `PATH` placement nothing establishes

**Location:** `docs/operations.md:16,37,49,52,64,88,150`; `docs/cli-reference.md`

The original mechanism this finding cited (a missing npm `bin` shim in leaf packages) no longer applies — leaf packages are entirely gitignored and `crates/xtask`'s `package_leaf` writes only `bin/batcave` + `manifest.json`, no `package.json` at all (confirmed by running the packaging command directly). Distribution moved to raw GitHub Release assets fetched to `<stateRoot>/bin/<version>/batcave` and called by absolute path — never placed on `PATH`. But `docs/operations.md` and `docs/cli-reference.md` still show every example as bare `batcave serve/stop/monitor/doctor/status ...`, and neither explains how a reader gets there from the downloaded/cached binary or a locally-built one.

**Fix:** add a line to `operations.md`/`cli-reference.md` clarifying the binary's actual location (`<stateRoot>/bin/<version>/batcave` for the installed path, `target/debug/batcave`/`target/release/batcave` for a local build) and that examples assume it's been aliased or symlinked onto `PATH`.

#### R29. `workspaceMode` is an open string

**Location:** `packages/extension/src/tools/runs.ts:18`; `crates/runtime/src/service/orchestration.rs:648-656`

Runtime rejects an unknown value safely via `and_then(Value::as_str)`; a closed Zod enum on the tool side would avoid the round trip.

#### R30. Local Bun scripts omit tests CI runs

**Location:** root `package.json:10,13`; `.github/workflows/ci.yml:81-82`

`test`/`check` both run `bun test packages`, scoped only to `packages/`. CI's `test` job runs bare `bun test` with no path restriction, which additionally picks up `tests/conformance/assert-report.test.ts` — a contributor running local scripts doesn't exercise what CI does.

#### R31. CONTRIBUTING references a cargo-features example with no real features to substitute

**Location:** `CONTRIBUTING.md:45`

`cargo test --features "feature1,feature2"` names placeholder features; no crate in the workspace declares a `[features]` table (confirmed via `grep -rn "^\[features\]" **/Cargo.toml`, zero hits), so this line errors if run literally. Delete or replace with a real caveat that none currently exist.

#### R32. The extension header lists only six of eleven orchestration tools

**Location:** `packages/extension/src/index.ts:3-4`

Names `batman_task, batman_worker, batman_run, batman_message, batman_approval, batman_reconcile`; `tools/index.ts:38-48` registers all 11, missing `profile`, `workspace`, `artifact`, `child`, `violation` from the header comment.

#### R38. `install_frame_tap` exported on the crate's public API surface

**Location:** `crates/runtime/src/supervisor/mod.rs:14-16`

`pub use output::{..., install_frame_tap};` re-exports a raw-content capture bypass beyond the crate boundary; a comment states production never installs one, but nothing prevents an external caller in the same binary from doing so. Narrow to `pub(crate)` unless a cross-crate consumer is intended.

#### R39. `session_shutdown` doesn't clear `subscribedClient`, breaking future monitor reconnects

**Location:** `packages/extension/src/monitor/controller.ts:148-150`

The repair path pairs `controller.stop()` with `subscribedClient = undefined`; the `session_shutdown` handler calls only `controller.stop()`. After shutdown the client is unsubscribed but not marked closed, so a later `connect()` early-returns into a permanently dead monitor.

#### R40. `reconnect.test.ts` passes a `revision` param the tool schema doesn't define

**Location:** `packages/extension/src/reconnect.test.ts:129,137`; `packages/extension/src/tools/tasks.ts:23-27`

`batman_task`'s Zod schema has only `op`, `description`, `taskId` — no `revision`. The test still proves what it's named for; the field is just dead weight that could mislead a future reader. Remove it from the test's tool-call arguments.

#### R43. `artifacts.ts` tool description contradicts its actual ownership-based scope

**Location:** `packages/extension/src/tools/artifacts.ts:27`

States artifacts are "scoped to the current task," but the real scope (`orchestration.rs:1253-1271,1307-1325`) is session/owner-based (`owned_run_ids_op` keyed on `principal.instance_id`), with `taskId` only an optional narrowing filter. (`docs/plugin-usage.md` no longer repeats this claim — that half is resolved.) Update the tool description to state session-ownership scoping with optional task narrowing.

#### R46. Stale exception-table row should be deleted, not corrected

**Location:** `docs/manual-testing.md:433`

Lists `ompRpc/approval` as an expected fixture-mode failure. `fixtures/conformance/fixture-mode-baseline.json` already records `"ompRpc": []` — zero expected failures — and the claimed gap no longer exists in code: `omp_rpc/normalize.rs` has `extension_ui_request_to_pending_approval`, and `omp_rpc/conformance.rs:329-336`'s own doc comment says this scenario backs `ApprovalsCapability::Observable` with real, checkable state. Delete the row.

#### R61. `shared.ts` references the unimported type `ExtensionContext`

**Location:** `packages/extension/src/tools/shared.ts:8-14`

`OrchestrationToolContext`'s `getClient(extCtx: ExtensionContext)` uses a type never imported in the file. A scoped `tsc --noEmit` confirms: `error TS2304: Cannot find name 'ExtensionContext'`. Ships silently today because `bun build` transpiles without type-checking and no `tsc` gate exists (R45) — every orchestration tool file implements against this interface, so it's load-bearing, not incidental.

#### R62. `LeaseService::active_for_run` silently converts genuine database errors into "no active lease"

**Location:** `crates/runtime/src/workspace/lease.rs:195-217`; consumed at `crates/runtime/src/service/orchestration.rs:772`

`.ok()` on the query result collapses every `rusqlite::Error` variant — not just "no rows" — into `None`. A locked/corrupted DB or a schema mismatch makes a real workspace's existence silently invisible to `run/get`/`coordination/peerWorkspace` callers instead of surfacing an error.

#### R63. `LeaseError::Conflict`'s doc comment promises a same-run guard that doesn't exist

**Location:** `crates/runtime/src/workspace/lease.rs:14-17` vs. `:78-169` (`acquire`)

The doc comment describes `Conflict` as firing when one run requests a second workspace; the actual code only keys off isolation/mode conflicts against any run's active shared-write lease, never `run_id`. A single run can acquire unbounded concurrent `GitWorktree`/`Copy` leases for itself with no guard. No live-exploit evidence given the current single caller (OMP) — primarily a doc/code mismatch.

#### R64. `marketplace.json`'s version has no automated coherence check

**Location:** `.claude-plugin/marketplace.json`; `crates/xtask/src/main.rs:448-484` (`check_version_coherence`)

`CONTRIBUTING.md` requires `marketplace.json`'s two version fields to match `packages/extension/package.json` as a manual pre-tag checklist item, but `check_version_coherence` (run by `generate --check` and CI) only checks the two `Cargo.toml`s against `package.json` — `marketplace.json` is never mentioned in any tooling (confirmed via repo-wide grep). A maintainer can bump the extension version, pass all of CI, and tag/release with a stale `marketplace.json` undetected.

#### R65. `RateLimiter`'s per-sender map grows unboundedly

**Location:** `crates/runtime/src/coordination/rate_limit.rs:42-55`

Stale timestamps are pruned, but the `HashMap` key itself (the sender) is never removed on worker/run retirement — unlike `ScopeTokenStore::revoke_for_run`. Slow, unbounded memory growth proportional to total distinct workers ever spawned over a long-running daemon's uptime.

#### R66. `DatabaseHandle::shutdown()` skips joining the actor thread when the actor died abnormally

**Location:** `crates/runtime/src/db/actor.rs:284-304`

`rx.await.map_err(|_| DbError::ActorUnavailable)?` short-circuits via `?` when the actor's reply channel is dropped without a value (the actor panicked mid-command), skipping the `worker.join()` step below it — unlike the adjacent `sent.is_err()` branch, which still falls through to the join. Leaks the `JoinHandle` and loses the chance to observe the panic; only reachable after a prior actor crash.

#### R67. No end-to-end integration test proves a Claude or Codex run releases its concurrency slot

**Location:** `crates/runtime/tests/claude_adapter.rs`, `crates/runtime/tests/codex_adapter.rs`, `crates/runtime/tests/adapter_registry.rs`

**Evidence:** R47's fix added component-level tests proving `ProcessExited` emission from the adapter drivers and a mocked `SettlementSink` release, but no integration test drives a full Claude or Codex run through the real adapter registry's completion watcher and asserts the concurrency slot is returned. `adapter_registry.rs`'s existing slot test (`releasing_a_policy_evaluator_slot_frees_the_registry_ceiling`) calls `authorization.release()` directly, bypassing the event stream entirely.

**Fix:** extend the existing `settlement_tests` in `adapter/registry.rs` to assert that a synthetic `ProcessExited` event flows through `SettlementSink` → `watch_settlement` → `PolicyEvaluator::release()` and actually frees the concurrency ceiling, closing the gap between component emission tests and the real registry path.

**Priority:** Low — the implementation fix for R47 is complete and defended by component tests; this is a coverage gap in the integration layer, not a functional defect.

## Known Environment Limitations

**Not a bug — requires a gated live run to confirm the positive case. Reconfirmed 2026-08-12; code-side citations still match current source.**

- **Codex** (`follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, `result_usage_artifacts`): blocked on account credits, not code. `codex login status` reports authenticated; `initialize`/`thread/start` succeed; the turn is refused server-side. `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event` (`crates/runtime/tests/codex_adapter.rs:83`) defends the adapter's handling of the vendor's own error notification.
- **Copilot** (`session_resume`, `runtime_restart`): a genuine ACP v1 protocol wall — `session/load` answers "Resource not found" for a session that completed a real turn in a prior process. `session_resume_probe` (`crates/runtime/src/adapter/copilot/conformance.rs:421`) is written to pass automatically if a future CLI version persists sessions differently.

Prove these via `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance runs when a licensed, billed run is acceptable. No code change needed.

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c

## Open Item Count

*(2026-08-12: every item below independently re-verified or newly discovered against current source in this pass.)*

- **Critical:** 0 — R48 resolved 2026-08-13 (see docs/journal.md Part XI), R49 resolved 2026-08-13 (see docs/journal.md Part XII), R69 resolved 2026-08-16 (see docs/journal.md Part XVI)
- **High:** 2 (R44 — carried forward from earlier rounds; R71 — new, found during R70's adversarial review) — R41, R50 resolved 2026-08-13 (see docs/journal.md Part XIII), R52 resolved 2026-08-14 (see docs/journal.md Part XIV), R51 resolved 2026-08-14 (see docs/journal.md Part XV), R68 resolved 2026-08-16 (see docs/journal.md Part XVII), R53 resolved 2026-08-16 (see docs/journal.md Part XVIII), R54 resolved 2026-08-17 (see docs/journal.md Part XIX), R70 resolved 2026-08-18 (see docs/journal.md Part XX), R33 resolved 2026-08-18 (see docs/journal.md Part XXI)
- **Medium:** 17 (R12, R13, R14, R15, R16, R34, R35, R36, R37, R42, R45 — carried forward; R55-R60 — new)
- **Low:** 19 (R17, R18, R20, R29, R30, R31, R32, R38, R39, R40, R43, R46 — carried forward, four corrected in place this pass; R61-R67 — new)
- **Environment (not actionable in-repo):** Codex account credits, Copilot ACP v1 protocol wall — reconfirmed, unchanged
