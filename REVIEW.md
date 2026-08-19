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

**Close-out:** 2026-08-19 — every open item (44 at the start of the pass: R12-R18, R20, R29-R32,
R34-R40, R42, R43, R45, R46, R55-R67, R78-R86, plus R87-R96 registered by the pass's own
adversarial reviews) resolved across twelve reviewed batches, journal Parts XXXI-XLIII. One watch
item (R97) remains.

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

**Baseline, last run 2026-08-19** (after the close-out pass; results apply to this snapshot):

- `cargo test --workspace` (`BATMAN_DISABLE_VENDOR_CLI=1`) — 853 passed across 59 suites; the one
  failure is the pre-existing, environment-specific
  `copilot_adapter::real_binary_initialize_and_session_list_never_invoke_a_model`, which fails
  because the local machine's installed Copilot CLI (1.0.80) isn't in `COPILOT_KNOWN_CLI_VERSIONS`
  yet — a local-environment gap, not a code defect. R79's race test flaked once under
  full-workspace load during the pass and is tracked as R97.
- `bun test packages` — 150 passed, 0 failed.
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `bun run typecheck`,
  `biome format .`, `bun run generate --check` — all clean.
- E2E smoke: `batcave serve --foreground` → `status` (live `activeRuns`, `protocolHealthy: true`)
  → `stop` (graceful, socket removed) against a scratch repository.

**How to read priority:** ranked against end-to-end functionality completeness — the full task →
run → worker → adapter → events → completion lifecycle across all four adapters, plus the
release/install/distribution path and documentation a user or contributor actually depends on to
operate the system correctly.

## Findings

### High

None open — see the resolution-history paragraph above for what closed and when.

### Medium

None open.

### Low

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
- **Low:** 1 (R97 — flake watch from the 2026-08-19 full-suite run; R38, R65, R85 resolved 2026-08-19, see docs/journal.md Part XLII; R67 resolved 2026-08-19, see docs/journal.md Part XLI; R86, R88-R96 resolved 2026-08-19, see docs/journal.md Parts XLII-XLIII; R80 resolved 2026-08-19, see docs/journal.md Part XL; R17, R18, R20, R29-R32, R39, R40, R43, R46, R61-R64, R66, R84 resolved 2026-08-19, see docs/journal.md Parts XXXI-XXXVIII)
- **Environment (not actionable in-repo):** Codex account credits, Copilot ACP v1 protocol wall — reconfirmed, unchanged
