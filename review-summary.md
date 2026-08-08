# PR Review: origin/main..main

## Summary
64 files changed, +5655/-1903 lines. Closes review findings R5-R11 / TODO 72-78.

**Verdict: NEEDS CHANGES**

## Per-Reviewer Counts

| Reviewer | Verdict | Errors | Warnings |
|----------|---------|--------|----------|
| ReviewerIPC | PASS | 0 | 0 |
| ReviewerExtensionCore | PASS WITH WARNINGS | 0 | 6 |
| ReviewerMonitor | PASS WITH WARNINGS | 0 | 3 |
| ReviewerApproval | PASS WITH WARNINGS | 0 | 5 |
| ReviewerWorkspace | NEEDS CHANGES | 1 | 3 |
| ReviewerProtocol | NEEDS CHANGES | 1 | 3 |
| ReviewerAdapters | NEEDS CHANGES | 1 | 5 |
| ReviewerDatabase | NEEDS CHANGES | 1 | 5 |
| ReviewerOrchestration | NEEDS CHANGES | 1 | 6 |
| ReviewerExtensionTools | NEEDS CHANGES | 1 | 3 |
| ReviewerConfig | NEEDS CHANGES | 5 | 8 |
| ReviewerConformance | NEEDS CHANGES | 8 | 11 |
| ReviewerDocs | NEEDS CHANGES | 2 | 13 |
| ReviewerProtocolTS | NEEDS CHANGES | 5 | 4 |
| ReviewerSecurity | NEEDS CHANGES | 3 | 4 |
| ReviewerTests | NEEDS CHANGES | 3 | 6 |

**Sum of reviewer errors: 32**

## Evidence-Backed Errors (Itemized from Reviewer Reports)

These are the errors explicitly enumerated with file:line in the reviewer reports.

### E1. `serde_json/preserve_order` breaks `WorkerProfile::fingerprint()`
**File:** `Cargo.toml:22`, `crates/runtime/src/adapter/profile.rs:325-326`
**Found by:** ReviewerConfig, ReviewerProtocolTS, ReviewerSecurity
**Issue:** `preserve_order` flips `serde_json` from `BTreeMap` to `IndexMap`. `fingerprint()` hashes `sanitize_json` output which now varies with caller key order. Documented "identical content, one fingerprint" contract broken.

### E4. `decided_by` persisted as JSON with quotes
**File:** `crates/runtime/src/domain/repository.rs:805`
**Found by:** ReviewerDatabase
**Issue:** `serde_json::to_string(&decided_by)` produces `"human"` instead of bare `human`. Every other scalar enum writes bare tokens. `SELECT * FROM approvals WHERE decided_by = 'human'` returns zero rows permanently.

### E5. `artifact/fetch` authorizes after reading content
**File:** `crates/runtime/src/service/orchestration.rs:1327-1346`
**Found by:** ReviewerOrchestration, ReviewerTests
**Issue:** `fetch_chunked` called before `in_scope` check. SHA-256 hashes entire content before authorization. Latency distinguishes unknown vs out-of-scope IDs.

### E6. `ProtocolHealthChanged` detail uses mangled string
**File:** `crates/runtime/src/adapter/copilot/normalize.rs:193`
**Found by:** ReviewerAdapters, ReviewerTests
**Issue:** Unknown stop reason detail interpolates normalized `other` (`copilotquotaexhausted`) instead of raw `stop_reason` (`_copilot_quota_exhausted`).

### E7. `model.test.ts` type errors: `decidedBy` required but omitted
**File:** `packages/extension/src/monitor/model.test.ts:240,247`
**Found by:** ReviewerProtocol, ReviewerProtocolTS, ReviewerTests
**Issue:** `DecidedBy` is required in `RuntimeEvent` but omitted in test literals. `tsconfig.json` declares `strict: true` but no `tsc` runs in CI.

### E8. `artifacts.ts` doc contradicts actual scope
**File:** `packages/extension/src/tools/artifacts.ts:27`
**Found by:** ReviewerExtensionTools
**Issue:** Description states "scoped to the current task" but runtime scopes by task ownership. Internal contradiction within the same tool definition.

### E9. `plugin-usage.md` false artifact scope claim
**File:** `docs/plugin-usage.md:121-122`
**Found by:** ReviewerDocs
**Issue:** States "Artifacts are scoped to the current task" but runtime scopes by session ownership.

### E10. `manual-testing.md` wrong expected failure
**File:** `docs/manual-testing.md` §4b exception table, row 1
**Found by:** ReviewerDocs
**Issue:** Lists `ompRpc/approval` as expected fixture-mode failure. Baseline records `ompRpc: []` (zero failures).

### E11. TODO.md item #74 still "Open"
**File:** `TODO.md:157`
**Found by:** ReviewerConfig
**Issue:** R7 retry fix is implemented (`orchestration.rs:540,864`) and tested. TODO.md declares itself single source of truth.

### E12. REVIEW.md R7 still carries wrong "Suggested fix"
**File:** `REVIEW.md`
**Found by:** ReviewerConfig
**Issue:** Still says "do not describe retry as execution" when retry now does execute.

### E13. AGENTS.md/CLAUDE.md publish wrong CLI flags
**File:** `AGENTS.md:91`, `CLAUDE.md:61`
**Found by:** ReviewerConfig
**Issue:** Publish `batcave status [--recover]` but no such flag exists (`cli.rs:60-70`). Reopens tracked finding R21.

### E14. AGENTS.md/CLAUDE.md omit required `--repo` flag
**File:** `AGENTS.md:90-93`, `CLAUDE.md:60-63`
**Found by:** ReviewerConfig
**Issue:** Every example fails clap parsing. Reopens R22.

### E15. `scrub.rs` calibrated to one fixture
**File:** `crates/runtime/src/conformance/scrub.rs:182-205`
**Found by:** ReviewerConformance
**Issue:** Placeholder prefixes match `claude/initialize.jsonl` only. `batcave capture` would rewrite the other 10 committed fixtures.

### E16. `capture.rs` `unchanged` guard is constant
**File:** `crates/runtime/src/conformance/capture.rs:156-158`
**Found by:** ReviewerConformance
**Issue:** Computed after `fs::write`, compares file to what was just written: always `true` on capture, always `false` on dry run.

### E17. `fixture-mode-baseline.json` has one consumer, not two
**File:** `fixtures/conformance/fixture-mode-baseline.json`
**Found by:** ReviewerConformance
**Issue:** CLI gate is sound but TS gate (plan step 6.3) was never written. `run.ts:40` throws on CLI exit code before report is validated.

### E18. `install_frame_tap` on public API surface
**File:** `crates/runtime/src/supervisor/mod.rs:15`
**Found by:** ReviewerSecurity
**Issue:** Raw-content bypass exported on crate's public API. Comment says "production never installs one" but no structural enforcement.

### E19. No `tsc` in CI
**File:** `package.json:13`, `packages/extension/package.json:10`
**Found by:** ReviewerProtocolTS
**Issue:** `tsconfig.json` declares `strict: true` but no `tsc` runs anywhere. Generated types are authoritative wire contract with no gate proving consumers compile.

### E22. No test asserts producer stamps `run_id`
**File:** `crates/runtime/tests/workspace_apply.rs:241,292,357,452,486`
**Found by:** ReviewerWorkspace
**Issue:** Reverting `apply.rs:105` and `inspect.rs:77` to `run_id: None` leaves whole suite green. R10 regression fails closed silently.

## Unitemized Reviewer Errors (Counted but Not Detailed)

These errors exist in reviewer counts but were not itemized with file:line in the reports:

- **ReviewerConformance**: 5 further errors (8 total minus 3 itemized above as E15-E17)
- **ReviewerProtocolTS**: 2 further errors (5 total minus 3 itemized above as E1, E7, E19)
- **ReviewerSecurity**: 1 further error (3 total minus 2 itemized above as E1, E18)

**Unitemized total: 8 errors** across 3 reviewers.

## Promotions (Warning/Suggestion → Error)

Three findings were classified as warnings or suggestions by their reviewers but are promoted to Error:

- **P1** (`session_shutdown` doesn't clear `subscribedClient`): ReviewerMonitor classified as W1. `controller.ts:148-150` calls `controller.stop()` without clearing `subscribedClient`. After shutdown, `isClosed` stays false, permanently breaking monitor reconnect.
- **P2** (`reconnect.test.ts` uses non-existent `revision` param): ReviewerExtensionCore classified as W4. `reconnect.test.ts:129,137` passes `revision: INITIAL_TASK_REVISION` but `task/upsert` schema has no `revision` field.
- **P3** (`start_queued_run` lease leak on error): ReviewerOrchestration classified as S4. `orchestration.rs:613-615` — if `driver.start` fails after `lease_service.activate`, lease stays active and materialized worktree stays on disk.

## Deduplication

The 32 reviewer-classified errors contain cross-reviewer overlap for the itemized findings:

| Error | Reviewers | Duplicates |
|-------|-----------|------------|
| E1 | Config, ProtocolTS, Security | 2 |
| E5 | Orchestration, Tests | 1 |
| E6 | Adapters, Tests | 1 |
| E7 | Protocol, ProtocolTS, Tests | 2 |

**Itemized unique from reviewers: 18** (24 itemized mentions minus 6 duplicates)
**Unitemized from reviewers: 8**
**Promotions: 3**
**Total unique errors: 29**

## Key Warnings (Selected)

- W1: `findPendingApproval` unhandled rejection (`approvals.ts:75`)
- W2: `run/retry` silently drops `policyOverrides` (`orchestration.rs:789-891`)
- W3: `owned_run_ids_op` swallows decode errors (`query.rs:285,292`)
- W4: `run/retry` no ownership check on `priorRunId` (`orchestration.rs:817-819`)
- W5: `conformance_profile` over-broad visibility (`conformance.rs:38`)
- W6: `dispose` doesn't clear `sink` (`copilot/mod.rs:640-651`)
- W7: `connect` no in-flight guard (`controller.ts:106-122`)
- W8: `decided_by` column has no reader (`query.rs:224-250`)
- W9: `owned_run_ids_op` missing doc comment (`query.rs:264`)

## Pass

- IPC test hook properly isolated (ReviewerIPC)
- Frame tap correctly implemented (ReviewerSecurity)
- Protocol types correct (ReviewerProtocol)
- Generated bindings correct (ReviewerProtocolTS)
- Tests added for retry, reconnection, approval enforcement (ReviewerTests)
- Documentation updated (ReviewerDocs)

## Verdict
**NEEDS CHANGES** — 29 unique errors (18 itemized + 8 unitemized + 3 promotions) must be fixed before merge.
