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
documentation findings that were resolved or already-stale (R19, R21-R28, and — this pass — R20, R31, R32, R43, R46, R58) has been pruned from
this document. That history — what broke, the fix commit, the test that proved it, and which
still-open items below exist *because* of that fix — now lives in
[`docs/journal.md` Part X](journal.md#part-x--reviewmds-second-pass-seven-more-fixes-eleven-doc-corrections-and-the-residue-that-outlived-them)
(R1-R11, R47), [Part XI](journal.md#part-xi--halving-the-critical-pair-a-ceiling-that-could-not-be-enforced) (R48),
[Part XII](journal.md#part-xii--closing-the-last-critical-a-denylist-blind-to-its-own-vendor) (R49),
[Part XIII](journal.md#part-xiii--two-leaks-one-lease-releasing-what-a-failed-start-acquired) (R41, R50),
[Part XIV](journal.md#part-xiv--fixture-modes-broken-promise-a-kill-switch-only-one-caller-ever-asked-about) (R52),
[Part XV](journal.md#part-xv--crash-recoverys-five-minute-blind-spot-the-one-crash-it-could-not-see) (R51), [Part XVI](journal.md#part-xvi--a-state-machine-with-no-production-writer-closing-the-last-critical) (R69), [Part XVII](journal.md#part-xvii--skipped-is-not-fail-the-discriminator-r68-asked-for) (R68), [Part XVIII](journal.md#part-xviii--one-guard-three-doors-the-two-coordination-calls-that-journaled-unmetered) (R53), [Part XIX](journal.md#part-xix--two-decisions-one-violation-the-guard-that-lived-outside-the-transaction) (R54), [Part XX](journal.md#part-xx--the-same-race-one-service-over-the-approval-that-could-be-decided-twice) (R70), [Part XXI](journal.md#part-xxi--a-feature-flag-for-one-tool-three-broken-content-addresses) (R33), [Part XXII](journal.md#part-xxii--the-capture-pipeline-that-graded-its-own-homework) (R44), [Part XXIII](journal.md#part-xxiii--the-same-guarded-write-one-interleaving-further-the-decider-that-no-longer-owned-the-task) (R71), [Part XXIV](journal.md#part-xxiv--the-same-guarded-write-one-service-over-the-violation-that-no-longer-had-an-owner) (R72), [Part XXV](journal.md#part-xxv--not-a-conflict-either-side-detects-the-flag-write-that-clobbered-its-neighbor) (R73), [Part XXVI](journal.md#part-xxvi--a-guard-that-overreached-the-rebind-that-couldnt-be-resumed) (R74), [Part XXVII](journal.md#part-xxvii--whoever-committed-first-the-ownership-guard-that-arrived-in-someone-elses-commit) (R76), [Part XXVIII](journal.md#part-xxviii--two-clocks-one-flag-the-quarantine-race-that-closed-into-three-more-findings) (R75), [Part XXIX](journal.md#part-xxix--six-doors-one-owner-the-run-lifecycle-gets-the-same-lock-as-task-upsert) (R77), [Part XXX](journal.md#part-xxx--four-gates-one-helper-the-chain-that-stops-here) (R81), [Part XXXI](journal.md#part-xxxi--the-map-corrects-itself-six-documentation-lies-and-one-new-medium) (R20, R31, R32, R43, R46, R58), [Part XXXII](journal.md#part-xxxii--strict-true-was-a-decoration-wiring-the-compiler-gate) (R30, R37, R45, R61), [Part XXXIII](journal.md#part-xxxiii--tool-contracts-that-lied-about-themselves) (R15, R16, R18, R29, R39, R40, R56), [Part XXXIV](journal.md#part-xxxiv--the-generator-that-only-generates-what-its-told) (R17, R60, R64), [Part XXXV](journal.md#part-xxxv--making-invariant-2-true-instead-of-aspirational) (R55), [Part XXXVI](journal.md#part-xxxvi--three-adapters-three-honesty-fixes) (R12, R42, R57), [Part XXXVII](journal.md#part-xxxvii--the-audit-trail-that-threw-away-its-own-rationale) (R34, R59), and [Part XXXVIII](journal.md#part-xxxviii--seven-kinds-of-dishonest-error-classified-honestly) (R13, R14, R35, R62, R63, R66, R84).
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


### Low

#### R38. `install_frame_tap` exported on the crate's public API surface

**Location:** `crates/runtime/src/supervisor/mod.rs:14-16`

`pub use output::{..., install_frame_tap};` re-exports a raw-content capture bypass beyond the crate boundary; a comment states production never installs one, but nothing prevents an external caller in the same binary from doing so. Narrow to `pub(crate)` unless a cross-crate consumer is intended.

#### R65. `RateLimiter`'s per-sender map grows unboundedly

**Location:** `crates/runtime/src/coordination/rate_limit.rs:42-55`

Stale timestamps are pruned, but the `HashMap` key itself (the sender) is never removed on worker/run retirement — unlike `ScopeTokenStore::revoke_for_run`. Slow, unbounded memory growth proportional to total distinct workers ever spawned over a long-running daemon's uptime.


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

#### R88. `batman_message.kind` accepts prose the runtime rejects — R16's class, one door over

**Location:** `packages/extension/src/tools/messages.ts:24`; `crates/protocol/src/message.rs:43-57` (`MessageKind`, closed serde enum); `crates/runtime/src/service/orchestration.rs` (`parse_message_kind`)

`kind` is `pi.zod.string()` with a describe-string enumerating nine valid tokens; the runtime rejects anything outside the closed `MessageKind` enum. Same defect class as R16/R29 (fixed 2026-08-19): the model burns a round trip to learn what the schema already knew. Swept during R16's adversarial review: this is the only remaining open-string enum among the eleven tool schemas (`workers.ts`/`profiles.ts` `adapter` are deliberately open — the runtime accepts unknown adapters). Close with `pi.zod.enum([...])` over the nine tokens.

**Priority:** Low — found during R16's adversarial review (2026-08-19).

#### R89. `run/submit`'s response echoes `workspaceMode: "isolated"` for a copy workspace

**Location:** `crates/runtime/src/service/orchestration.rs:897` (`json!("isolated")` literal), `:930-933` (`IsolationKind::Copy => "isolated"`), `:1050` (retry)

The request side now speaks the closed vocabulary shared|isolated|copy (R29), but the response side collapses `copy` to `"isolated"` in both `run/submit` and `run/get`. A caller submitting `workspaceMode: "copy"` reads back `"isolated"`. Derive the echoed string from the resolved `IsolationKind` (Shared→"shared", GitWorktree→"isolated", Copy→"copy"), matching the `isolationKind` mapping at `:1405`/`:1470` which already distinguishes all three. Wire-shape change: needs its own commit and test.

**Priority:** Low — found during R29's adversarial review (2026-08-19).

#### R90. Generated TS bindings and the JSON Schema disagree on numeric width and Option presence

**Location:** `packages/protocol-ts/src/generated/Artifact.ts` (`byteLength: bigint`, `runId: string | null` required), `ArtifactFetchResult.ts` (`nextOffset: bigint | null`); `packages/protocol-ts/schema/batman.schema.json` (same fields as `{"type":"integer","format":"uint64"}` numbers, `runId` absent from `required`)

ts-rs maps Rust `u64` to TypeScript `bigint` while schemars types the same field as a JSON integer that `JSON.parse` yields as a `number`, and ts-rs emits every `Option<T>` as a required `T | null` property while schemars leaves it out of `required`. No validation failure is possible (the Ajv formats are registered as always-passing and both null-and-absent pass), but `result as ArtifactFetchResult` casts now advertise static types that are wrong at runtime (`typeof byteLength === "number"`). Pre-existing generator convention (`EventEnvelope.sequence`, `RuntimeStatus.uptimeSeconds` were already `bigint`), surfaced now because R55's validators are the first code acting on these defs. Fix shape: `#[ts(type = "number")]` on `u64` wire fields (or a documented bigint reviver), and reconcile the Option-presence asymmetry between the two generators.

**Priority:** Low — found during R55's adversarial review (2026-08-19); static-type trap only, no runtime defect.

#### R91. `ProtocolHealthChanged`'s detail never reaches an operator surface

**Location:** `crates/runtime/src/lifecycle.rs:~725` (status-row mapping collapses the event to the constant string "protocol health changed"); `packages/extension/src/monitor/model.ts` (no `adapterProtocolHealthEvent` handler at all)

R12/R42/R57 all invest in a precise `detail` (the vendor's error subtype, the raw stop reason), and the event reaches the journal and `events/subscribe` — but the `batcave status` row mapping discards the detail for a constant label, and the `/batman` monitor model has no handler for the event kind, so an operator sees neither which run's protocol went unhealthy nor why. Bind and render the detail in the status-row mapping, and teach the monitor model the event. Related open question (no repo evidence either way): whether a Claude `is_error: true` run terminalizes as `succeeded` or `failed` from OMP's perspective — the CLI's exit code for error result arms is not pinned by any test or fixture.

**Priority:** Low — found during R12's adversarial review (2026-08-19); observability residue of a correctly-journaled event.

#### R92. Approval decision provenance is persisted but has no RPC read surface

**Location:** `crates/runtime/src/service/query.rs:242-252` (`approval_list_op`'s SELECTs stop at `decision`); `crates/protocol/src/approval.rs:19-49` (`ApprovalRequest` carries no `decidedBy`/`reason` fields); `docs/plugin-usage.md` (documents the current result shape)

`decided_by` has been write-only since MIGRATION_7, and R59 (2026-08-19) added `reason` to the same blind spot: both are durably persisted and carried on `ApprovalDecided` events, but `approval/list` returns neither, so the rationale is observable only via `events/replay` or `batcave audit export`. R34's user-facing scenario (querying decisions by decider) works only for someone opening `runtime.db` by hand. Fix shape: extend `approval_list_op`'s projection and `ApprovalRequest` (or the list result) with both fields — a wire-shape decision, so its own batch. Same class as R80's registered gap for policy violations.

**Priority:** Low — found during R34/R59's adversarial review (2026-08-19).

#### R93. `run/cancel` still reports success after a genuine kill failure

**Location:** `crates/runtime/src/service/orchestration.rs:1087-1091` (`run_cancel`'s `cancel_run` error arm)

R13 (2026-08-19) made `RunDriver::cancel_run`'s `Err` unambiguous — an absent adapter is the clean `CancelOutcome::NoRunningAdapter`, so `Err` now always means a live vendor process a kill actually failed against. The policy-violation path raises `flags.degradedControl` on that condition; `run_cancel` still only `tracing::warn!`s and returns unqualified success to the caller. Apply the same ten-line `set_run_flag(DegradedControl)` treatment (guarded write, journaled, broadcast).

**Priority:** Low — found during R13's adversarial review (2026-08-19); same defect class one door over.

#### R94. `require_live_run` is an advisory pre-check outside the writes it guards — R78's class, one door over

**Location:** `crates/runtime/src/coordination/broker.rs:134-149` (`require_live_run`), `:235-255` (the same-task check with the identical shape); `crates/runtime/src/domain/repository.rs::record_message` (no in-tx run-state guard)

`require_live_run` reads the run's terminal state in its own `run_domain_op`, then the caller writes in a later round trip — a run settling between the check and the write journals a message against a terminal run, across `coordination/send`, `publishArtifact`, `requestChild`, `reportBlocked`, and `askPolicy`. The broker's own doc claims a live-token connection must "never be able to mutate ... state for a run that is no longer active". R78's `enforce_quarantine` parameter (2026-08-19) is the ready-made pattern: an `enforce_live` sibling checked inside `record_message`'s guarded transaction.

**Priority:** Low — found during R78's adversarial review (2026-08-19); bounded to one racing write per settling run.

#### R95. A terminal-adapter run that settles without `ProcessExited` pins its slot and the daemon's idle state forever

**Location:** `crates/runtime/src/adapter/registry.rs:353-359` (`watch_settlement`'s `Err` arm), `crates/runtime/src/adapter/terminal.rs` (emits no `ProcessExited` of its own); `crates/runtime/src/ipc/server.rs:45-52` (`active_run_count` consumers)

**Evidence:** `watch_settlement` deliberately never releases on a dropped-sink `Err` — releasing without a settlement would hand the run's slot to another run (fails safe, correct direction). But the consequence is unbounded: the run stays in the registry map, so `active_run_count()` never drops, the daemon can never idle-shut-down again, and unforced `runtime/shutdown` is refused permanently. The boot recovery sweep fixes the *run state* on the next start, but nothing inside the live process ever frees the slot. `force: true` and `batcave stop`'s SIGTERM remain as operator escapes.

**Fix:** either make the terminal adapter emit a synthetic `ProcessExited` when its run reaches a terminal state, or teach the recovery sweep's live-process arm to evict registry entries whose runs the journal already shows terminal.

**Priority:** Low — terminal adapter only, operator escapes exist; found during R67's adversarial review (2026-08-19).

#### R96. Expired scope-token records leak when an adapter dies before its settlement hook

**Location:** `crates/runtime/src/coordination/scope_token.rs:236-257` (`verify` detects expiry without removing), `:228-234` (`revoke_for_run`, the only remover); settlement-guarded call sites `crates/runtime/src/adapter/claude/mod.rs:447,694`, `codex/mod.rs:257,634,684`, `copilot/mod.rs:325,655`

**Evidence:** R65's defect class, one door over: `revoke_for_run` fires solely from adapter settlement paths guarded by `if let Some(mcp)`, and `verify` returns `InvalidToken` for an expired record without removing it, so any run whose adapter task dies before its settlement hook leaks one `ScopeTokenRecord` for the process lifetime.

**Fix:** sweep expired records inside `bind` or `verify` (`tokens.retain(|_, r| now <= r.expires_at)`), mirroring `RateLimiter::check`'s sweep.

**Priority:** Low — found during R65's adversarial review (2026-08-19).

#### R97. `concurrent_cancelling_violations_are_both_idempotent_successes` flaked once under full-workspace parallel load

**Location:** `crates/runtime/tests/orchestration_rpc.rs` (`concurrent_cancelling_violations_are_both_idempotent_successes`, R79's race test)

**Evidence:** one failure observed during a full `cargo test --workspace --no-fail-fast` run (2026-08-19); green 5/5 in exact isolation, green in the suite alone, and green in two subsequent full-workspace runs. The failing assertion text was not captured. The test's determinism argument rests on the FIFO database actor plus `biased` join under a `current_thread` runtime; a host-load perturbation may expose a hole in that argument or an over-tight assertion on the audited-ack pair.

**Fix:** reproduce under load with output capture (`--nocapture` in a loop concurrent with heavy suites), then either repair the determinism argument or loosen the assertion to the legal outcome set.

**Priority:** Low — single observation, not reproduced; watch item.

## Known Environment Limitations

**Not a bug — requires a gated live run to confirm the positive case. Reconfirmed 2026-08-12; code-side citations still match current source.**

- **Codex** (`follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, `result_usage_artifacts`): blocked on account credits, not code. `codex login status` reports authenticated; `initialize`/`thread/start` succeed; the turn is refused server-side. `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event` (`crates/runtime/tests/codex_adapter.rs:83`) defends the adapter's handling of the vendor's own error notification.
- **Copilot** (`session_resume`, `runtime_restart`): a genuine ACP v1 protocol wall — `session/load` answers "Resource not found" for a session that completed a real turn in a prior process. `session_resume_probe` (`crates/runtime/src/adapter/copilot/conformance.rs:421`) is written to pass automatically if a future CLI version persists sessions differently.

Prove these via `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance runs when a licensed, billed run is acceptable. No code change needed.

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c

## Open Item Count

*(2026-08-12: every item below independently re-verified against current source; R87 added and the resolved documentation findings pruned 2026-08-19.)*

- **Critical:** 0 — R48 resolved 2026-08-13 (see docs/journal.md Part XI), R49 resolved 2026-08-13 (see docs/journal.md Part XII), R69 resolved 2026-08-16 (see docs/journal.md Part XVI)
- **High:** 0 — R41, R50 resolved 2026-08-13 (see docs/journal.md Part XIII), R52 resolved 2026-08-14 (see docs/journal.md Part XIV), R51 resolved 2026-08-14 (see docs/journal.md Part XV), R68 resolved 2026-08-16 (see docs/journal.md Part XVII), R53 resolved 2026-08-16 (see docs/journal.md Part XVIII), R54 resolved 2026-08-17 (see docs/journal.md Part XIX), R70 resolved 2026-08-18 (see docs/journal.md Part XX), R33 resolved 2026-08-18 (see docs/journal.md Part XXI), R44 resolved 2026-08-18 (see docs/journal.md Part XXII), R71 resolved 2026-08-18 (see docs/journal.md Part XXIII), R72 resolved 2026-08-18 (see docs/journal.md Part XXIV), R73 resolved 2026-08-18 (see docs/journal.md Part XXV), R74 resolved 2026-08-18 (see docs/journal.md Part XXVI), R76 resolved 2026-08-18 (see docs/journal.md Part XXVII), R75 resolved 2026-08-18 (see docs/journal.md Part XXVIII), R77 resolved 2026-08-19 (see docs/journal.md Part XXIX), R81 resolved 2026-08-19 (see docs/journal.md Part XXX)
- **Medium:** 0 (R36 resolved 2026-08-19, see docs/journal.md Part XLI; R12-R16, R34, R35, R37, R42, R45, R55-R60 resolved 2026-08-19, see docs/journal.md Parts XXXI-XXXVIII; R78, R79, R82, R87 resolved 2026-08-19, see docs/journal.md Part XXXIX; R83 resolved 2026-08-19, see docs/journal.md Part XL)
- **Low:** 11 (R38, R65 — carried forward/new 2026-08-12; R67 resolved 2026-08-19, see docs/journal.md Part XLI; R85, R86 — new, found during R81's adversarial review; R88, R89 — new, found during R16/R29's adversarial review; R90-R96 — new, found during R55/R12/R34/R13/R78/R67/R65's adversarial reviews; R97 — new, flake watch from the 2026-08-19 full-suite run; R17, R18, R20, R29-R32, R39, R40, R43, R46, R61-R64, R66, R84 resolved 2026-08-19, see docs/journal.md Parts XXXI-XXXVIII; R80 resolved 2026-08-19, see docs/journal.md Part XL)
- **Environment (not actionable in-repo):** Codex account credits, Copilot ACP v1 protocol wall — reconfirmed, unchanged
