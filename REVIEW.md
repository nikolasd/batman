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

**Resolution history moved:** everything that was Critical/High and is now resolved (R1-R11, R33, R41, R44, R47-R54, R68-R77, R81) plus the
eleven documentation findings that were resolved or already-stale (R19, R21-R28) has been pruned from
this document. That history — what broke, the fix commit, the test that proved it, and which
still-open items below exist *because* of that fix — now lives in
[`docs/journal.md` Part X](journal.md#part-x--reviewmds-second-pass-seven-more-fixes-eleven-doc-corrections-and-the-residue-that-outlived-them)
(R1-R11, R47), [Part XI](journal.md#part-xi--halving-the-critical-pair-a-ceiling-that-could-not-be-enforced) (R48),
[Part XII](journal.md#part-xii--closing-the-last-critical-a-denylist-blind-to-its-own-vendor) (R49),
[Part XIII](journal.md#part-xiii--two-leaks-one-lease-releasing-what-a-failed-start-acquired) (R41, R50),
[Part XIV](journal.md#part-xiv--fixture-modes-broken-promise-a-kill-switch-only-one-caller-ever-asked-about) (R52),
[Part XV](journal.md#part-xv--crash-recoverys-five-minute-blind-spot-the-one-crash-it-could-not-see) (R51), [Part XVI](journal.md#part-xvi--a-state-machine-with-no-production-writer-closing-the-last-critical) (R69), [Part XVII](journal.md#part-xvii--skipped-is-not-fail-the-discriminator-r68-asked-for) (R68), [Part XVIII](journal.md#part-xviii--one-guard-three-doors-the-two-coordination-calls-that-journaled-unmetered) (R53), [Part XIX](journal.md#part-xix--two-decisions-one-violation-the-guard-that-lived-outside-the-transaction) (R54), [Part XX](journal.md#part-xx--the-same-race-one-service-over-the-approval-that-could-be-decided-twice) (R70), [Part XXI](journal.md#part-xxi--a-feature-flag-for-one-tool-three-broken-content-addresses) (R33), [Part XXII](journal.md#part-xxii--the-capture-pipeline-that-graded-its-own-homework) (R44), [Part XXIII](journal.md#part-xxiii--the-same-guarded-write-one-interleaving-further-the-decider-that-no-longer-owned-the-task) (R71), [Part XXIV](journal.md#part-xxiv--the-same-guarded-write-one-service-over-the-violation-that-no-longer-had-an-owner) (R72), [Part XXV](journal.md#part-xxv--not-a-conflict-either-side-detects-the-flag-write-that-clobbered-its-neighbor) (R73), [Part XXVI](journal.md#part-xxvi--a-guard-that-overreached-the-rebind-that-couldnt-be-resumed) (R74), [Part XXVII](journal.md#part-xxvii--whoever-committed-first-the-ownership-guard-that-arrived-in-someone-elses-commit) (R76), [Part XXVIII](journal.md#part-xxviii--two-clocks-one-flag-the-quarantine-race-that-closed-into-three-more-findings) (R75), [Part XXIX](journal.md#part-xxix--six-doors-one-owner-the-run-lifecycle-gets-the-same-lock-as-task-upsert) (R77), and [Part XXX](journal.md#part-xxx--four-gates-one-helper-the-chain-that-stops-here) (R81).
This document only tracks what's still broken.

**Baseline, last run 2026-08-19** (after the R44/R70-R77/R81 closure pass; results apply to this
snapshot):

- `cargo test --workspace` (`BATMAN_DISABLE_VENDOR_CLI=1`) — 813 passed across 56 suites; the one
  skipped test is the pre-existing, environment-specific
  `copilot_adapter::real_binary_initialize_and_session_list_never_invoke_a_model`, which fails
  because the local machine's installed Copilot CLI (1.0.80) isn't in `COPILOT_KNOWN_CLI_VERSIONS`
  yet — a local-environment gap, not a code defect (see R57 for a related but distinct gap in the
  same version-check machinery).
- `bun test packages` — 139 passed, 0 failed.
- `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `biome format .`, `bun run generate --check` — all clean.

**How to read priority:** ranked against end-to-end functionality completeness — the full task →
run → worker → adapter → events → completion lifecycle across all four adapters, plus the
release/install/distribution path and documentation a user or contributor actually depends on to
operate the system correctly.

## Findings

### High

None open — see the resolution-history paragraph above for what closed and when.

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

#### R78. Quarantine RPC gates are advisory pre-checks outside the writes they guard

**Location:** `crates/runtime/src/service/orchestration.rs::ensure_not_quarantined` (~290-299; callers `message_send` ~1673, `workspace_apply` ~1478, `workspace_inspect` ~1421); `crates/runtime/src/coordination/broker.rs::require_not_quarantined` (~144-167, gating `coordination/publishArtifact`)

**Evidence:** both gates read the run's `policy_quarantined` flag in their own `run_domain_op` round trip, then the caller acts on that stale read in a later, separate round trip. A quarantine committing between the gate read and the gated operation admits one in-flight mutation past a run that is, by the time it lands, already quarantined. `message/send`'s check is foldable into `record_message`'s own transaction; the `workspace/apply`/`workspace/inspect` and `coordination/publishArtifact` paths gate a non-SQL operation (a working-tree mutation, or a broker-side artifact publish) with no write to fold the check into, so they need a different mechanism than the guarded-write doctrine used elsewhere. Carved out of R75's fix rather than fixed there — R75's own registered `ensure_not_quarantined` location is unchanged by that fix (see its journal Part). Found during R75's adversarial review, 2026-08-18.

**Fix:** fold the `message/send` gate into `record_message`'s transaction, ordered so the fold runs *after* R77's owner re-read — a non-owner must not be able to distinguish a quarantined run from a non-quarantined one via the error code; design a lease- or applier-level gate for the workspace and broker paths that closes the same window without a SQL write to guard.

**Priority:** Medium — bounded to one racing operation per gate; the quarantine still holds for everything after it lands.

#### R79. Violation cancel side effects are decided from a pre-effect discriminator

**Location:** `crates/runtime/src/policy/violation.rs::apply_action` (~355-408, the `Cancel`/`QuarantineAndCancel` arms consuming `already_actioned`); `::create_cancellation_intent`/`cancel_and_transition` (~433-497)

**Evidence:** `already_actioned` is read inside the same call as the violation's journal commit (R75), which closes the race for the quarantine flag, but the `Cancel`/`QuarantineAndCancel` side effects are still decided from that same pre-effect discriminator: a sibling observation's cancel is only visible to `already_actioned` after that sibling's terminal transition commits, two round trips later. Two concurrent violations on one run with a cancelling action can therefore both see `already_actioned = false`, both create an audited `policyViolationCancel` intent, and both attempt `transition_run(cancelled)` — the loser's transition fails, so `record_nested_worker`/`record_cost_ceiling` returns `Err` and the event sink logs a warning instead of the documented idempotent success. The run still cancels exactly once because the terminal-transition guard is itself correct; only the audit trail (a duplicate cancellation intent) and the caller-visible result (an error instead of success) are wrong. Contradicts `violation.rs`'s own doc comment (~216-224) and `docs/journal.md`'s claim that "the quarantine/cancel action applies exactly once." Found during R75's adversarial review, 2026-08-18.

**Fix:** move the cancel-side arbitration into the guarded write, mirroring R70-R76 — classify the terminal transition's refusal as an idempotent success (mirroring `AlreadyResolved`/`AlreadyDecided` elsewhere) rather than an error, or make the cancellation intent itself conditional on being the observation that actually performs the transition.

**Priority:** Medium — the run still cancels exactly once; the residue is a duplicate audit-trail row and a misleading error instead of a functional double-cancel.

#### R82. `runtime/shutdown` has no arbitration at all

**Location:** `crates/runtime/src/ipc/connection.rs:400-409`

**Evidence:** any `ompExtension` client can send `runtime/shutdown`; the handler unconditionally calls `shared.shutdown.notify_one()`, stopping the daemon that serves every connected instance's runs. The inline comment claims the method is "Role-gated to ompExtension" — exactly the disproven assumption R81 found elsewhere: role admission is not ownership. Largest blast radius of the whole class, since it affects every instance, not just one lease or run.

**Fix:** this is a policy decision, not another `run_owner_op` call — either refuse the shutdown while runs owned by other live instances are still active, or require the caller to be the daemon's only connected instance.

**Priority:** Medium — no ownership check exists at all; mitigated only by the fact that a legitimate `ompExtension` client rarely has a reason to call it while other instances are working.

#### R83. An accepted child request is indistinguishable from the request itself

**Location:** `crates/runtime/src/domain/repository.rs:1841-1848` (`request_child`) and `:1965-1971` (`decide_child`'s accept arm); `crates/protocol/src/event.rs:246-249`; `packages/extension/src/monitor/model.ts:214`

**Evidence:** both `request_child` and `decide_child`'s accept arm emit `RuntimeEventKind::ChildWorkerRequested`; only two child event kinds exist in the wire protocol (`ChildWorkerRequested`, `ChildWorkerRequestDenied`), so nothing distinguishes "a child was requested" from "a child request was accepted" except by inspecting whether the child ids are populated. The monitor labels an acceptance "child worker requested" (`model.ts:214`), the same discriminator class R68 already found and fixed for a different event pair.

**Fix:** add a distinct `ChildWorkerAccepted` (or equivalent) event kind, or otherwise make the accept arm's event self-describing so a consumer never has to infer "accepted" from field presence.

**Priority:** Medium — cosmetic/observability defect today (the run still proceeds correctly), but it hides a real state transition from every event consumer, same class as R68.

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

#### R80. No discovery surface for which violation still holds a quarantine

**Location:** `packages/extension/skills/batman-approvals/SKILL.md` (deliberately no `policy/violation/list` op); no `policy/violation/list` RPC exists anywhere in `crates/runtime/src/service` (repo-wide search for `PolicyViolationList`/`policy/violation/list` returns no matches); `packages/extension/src/monitor/model.ts` has no violation handling, only `RunFlagsChanged` → `policyQuarantined`

**Evidence:** `policy/violation/decide`'s response now reports whether *this* decision cleared the run's quarantine flag (`quarantineCleared`, `orchestration.rs:1874`, added by R75's `aee8e82`) — that half of this finding is resolved. What remains: when `quarantineCleared` is `false` (or absent, on `"cancel"`/an `alreadyDecided` replay), there is no way to find *which other* violation is still open short of diffing `PolicyViolationRecorded` against `PolicyViolationDecided` in the raw event stream — there is deliberately no violation-listing op, and the monitor's model tracks only the derived `policyQuarantined` flag, never violations themselves. The user-visible refusal a caller sees next (e.g. `message/send`) still reads "pending policy/violation/decide," which gives no hint how many violations remain or which they are. Found during R75's adversarial review, 2026-08-18; narrowed after R75's `aee8e82` answered the response-shape half.

**Fix:** add a `policy/violation/list` op and teach the monitor to display open violations.

**Priority:** Low — a real operator-visibility gap, but the run's correctness is unaffected; the operator can still recover by reading the raw event stream.

#### R84. Unknown `leaseId` reports `-32603` while an unowned one reports `-32602`

**Location:** `crates/runtime/src/workspace/lease.rs:341,354` (`LeaseError::NotFound`) → `ServiceError::internal` at the four gated handlers in `crates/runtime/src/service/orchestration.rs:1437,1480,1553,1630` (`workspace/get`, `workspace/release`, `workspace/inspect`, `workspace/apply`)

**Evidence:** a caller-supplied `leaseId` that doesn't resolve to a known row is a plain caller error, but `lease_service.get`'s `LeaseError` is mapped through `ServiceError::internal(e.to_string())` at all four gated handlers, reporting `-32603` (internal error) — the same misclassification class R54's review flagged elsewhere — while an owned-by-someone-else lease reaches the R81 ownership gate and correctly reports `-32602`. The pair is also an existence oracle sitting immediately in front of the authorization gate: a caller can distinguish "no such lease" from "not yours" by error code alone. Exploitability is low because lease ids are UUIDs, making the oracle hard to use productively; the classification bug is the actionable half.

**Fix:** map `LeaseError::NotFound` to `ServiceError::invalid_params` (or an equivalent caller-error code) at all four call sites instead of `internal`.

**Priority:** Low — a real classification defect and a minor oracle, but bounded by UUID-space lease ids.

#### R85. Project-scoped reads are open by design while `workspace/get` alone is gated

**Location:** `crates/runtime/src/service/orchestration.rs:408` (`task_get`), `:573`/`:580` (`worker_list`/`worker_get`), `:902` (`run_list`), `:915` (`run_get`), `:1922` (`message_list`), `:1932` (`approval_list`); `crates/runtime/src/ipc/connection.rs:385-395` (`events/replay`)

**Evidence:** none of the read handlers above take a principal; all are deliberately project-wide so any same-user `ompExtension`/`Display` client can see the whole project's state (`coordination_child_list`'s own doc makes the same intent explicit for that read). `run/get`'s `workspacePath` (`orchestration.rs:923-927`) is precisely the disclosure route R81's evidence names as the attack's entry point into the now-gated `workspace/*` mutations, and it stays open — as does `events/replay`'s `LeaseAcquired` payload. This is not a bug so much as an undocumented asymmetry: `workspace/get` is now the only ownership-gated read in the surface, beside reads that disclose the same facts.

**Fix:** record the read-side policy as an explicit decision rather than leaving it implicit — project-scoped reads (task/worker/run/message/approval reads, `events/replay`) are intentionally open to any same-user client; `workspace/get` is gated for surface uniformity with the other three `workspace/*` mutations, not because it protects any confidentiality boundary the other reads don't already cross.

**Priority:** Low — a documentation/decision gap, not a functional one; no read here discloses more than `run/get` already does today.

#### R86. Cross-session lease cleanup has no remedy when a correlation was never persisted

**Location:** `packages/extension/src/runtime.ts:262`; `crates/runtime/src/service/orchestration.rs:1479` (owner-gated `workspace/release`); `crates/runtime/src/doctor.rs:480-508` (report-only); `crates/runtime/src/cli.rs` (no lease subcommand exists); `crates/runtime/src/service/orchestration.rs:1130` (`abandon_lease`, internal-only release path)

**Evidence:** `instanceId` is the OMP session id, so a new session is a different principal from whichever session acquired a lease. `workspace/release` is now owner-gated (R81), the doctor only reports stale leases without releasing them, no CLI command releases a lease directly, and nothing auto-releases at run settlement. A lease left active by a prior session is therefore unreleasable by RPC until `reconcile/omp` rebinds its task to a new session.

**Mitigation:** startup reconciliation replays every persisted task/session correlation (`packages/extension/src/index.ts:213-222`; correlations are recorded on every upsert, `tools/tasks.ts:48-52`), which recovers the common case — hence Low. The residue is narrower: a lease whose correlation was never persisted (e.g. the extension crashed before the upsert that would have recorded it) leaves a worktree the doctor can only report, with no command able to clear it.

**Fix:** add a CLI or RPC path to force-release a lease by id (with appropriate confirmation/audit), or have the doctor's report include a suggested remedy command once one exists.

**Priority:** Low — narrow residual window behind an already-effective reconciliation mitigation.

## Known Environment Limitations

**Not a bug — requires a gated live run to confirm the positive case. Reconfirmed 2026-08-12; code-side citations still match current source.**

- **Codex** (`follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, `result_usage_artifacts`): blocked on account credits, not code. `codex login status` reports authenticated; `initialize`/`thread/start` succeed; the turn is refused server-side. `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event` (`crates/runtime/tests/codex_adapter.rs:83`) defends the adapter's handling of the vendor's own error notification.
- **Copilot** (`session_resume`, `runtime_restart`): a genuine ACP v1 protocol wall — `session/load` answers "Resource not found" for a session that completed a real turn in a prior process. `session_resume_probe` (`crates/runtime/src/adapter/copilot/conformance.rs:421`) is written to pass automatically if a future CLI version persists sessions differently.

Prove these via `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance runs when a licensed, billed run is acceptable. No code change needed.

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c

## Open Item Count

*(2026-08-12: every item below independently re-verified or newly discovered against current source in this pass.)*

- **Critical:** 0 — R48 resolved 2026-08-13 (see docs/journal.md Part XI), R49 resolved 2026-08-13 (see docs/journal.md Part XII), R69 resolved 2026-08-16 (see docs/journal.md Part XVI)
- **High:** 0 — R41, R50 resolved 2026-08-13 (see docs/journal.md Part XIII), R52 resolved 2026-08-14 (see docs/journal.md Part XIV), R51 resolved 2026-08-14 (see docs/journal.md Part XV), R68 resolved 2026-08-16 (see docs/journal.md Part XVII), R53 resolved 2026-08-16 (see docs/journal.md Part XVIII), R54 resolved 2026-08-17 (see docs/journal.md Part XIX), R70 resolved 2026-08-18 (see docs/journal.md Part XX), R33 resolved 2026-08-18 (see docs/journal.md Part XXI), R44 resolved 2026-08-18 (see docs/journal.md Part XXII), R71 resolved 2026-08-18 (see docs/journal.md Part XXIII), R72 resolved 2026-08-18 (see docs/journal.md Part XXIV), R73 resolved 2026-08-18 (see docs/journal.md Part XXV), R74 resolved 2026-08-18 (see docs/journal.md Part XXVI), R76 resolved 2026-08-18 (see docs/journal.md Part XXVII), R75 resolved 2026-08-18 (see docs/journal.md Part XXVIII), R77 resolved 2026-08-19 (see docs/journal.md Part XXIX), R81 resolved 2026-08-19 (see docs/journal.md Part XXX)
- **Medium:** 21 (R12, R13, R14, R15, R16, R34, R35, R36, R37, R42, R45 — carried forward; R55-R60 — new; R78, R79 — new, found during R75's adversarial review; R82, R83 — new, found during R81's adversarial review)
- **Low:** 23 (R17, R18, R20, R29, R30, R31, R32, R38, R39, R40, R43, R46 — carried forward, four corrected in place this pass; R61-R67 — new; R80 — new, found during R75's adversarial review; R84, R85, R86 — new, found during R81's adversarial review)
- **Environment (not actionable in-repo):** Codex account credits, Copilot ACP v1 protocol wall — reconfirmed, unchanged
