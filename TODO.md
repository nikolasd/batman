# BATMAN TODO

## Architecture Document Deferred Items (from docs/architecture.md §19)

### 1. Events table missing task_id/worker_id columns

**Status:** Open  
**Priority:** High  
**Labels:** bug, persistence, schema-migration

**Description:**
The `events` table still only stores `run_id`, not `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` (`source` is still hardcoded `runtime`). A *live* `events/event` notification's envelope carries `task_id`/`worker_id` (§11's `append_and_apply` sets them from its caller's parameters), but a *replayed* one from `events/replay` always has them `None` — `ipc/connection.rs::replay()` can only reconstruct an envelope from what the `events` table's columns hold.

The monitor (§17) is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields — but any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` will get silently wrong (empty) results.

**Implementation:**
- Schema migration to add `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns to `events` table
- Update `append_and_apply` in `crates/runtime/src/domain/repository.rs` to populate these columns
- Update `replay()` in `crates/runtime/src/ipc/connection.rs` to use the new columns

**References:** `docs/architecture.md` §4, §11

---

### 2. Worker adapters implemented but authorization layer not wired

**Status:** Implemented (gated)  
**Priority:** Medium  
**Labels:** adapter, authorization, hardening

**Description:**
The `AdapterRegistry` exists and implements `RunDriver` against Claude/Codex/Copilot/OMP-RPC adapters. However, production `ServerConfig::default()` uses `DenyByDefaultAuthorization` until the Hardening plan's `PolicyEvaluator` is wired. **Verified 2026-07-31: `PolicyEvaluator` is fully built and already implements the correct `AdapterAuthorization` trait** (`crates/runtime/src/policy/evaluate.rs:209`) — the gap is narrower than "build it": `crates/runtime/src/lifecycle.rs:193-197` simply constructs `DenyByDefaultAuthorization::from_env()` instead of a real `PolicyEvaluator`. The credential store for `workerMcp` connections is not yet implemented (`RejectAllWorkerVerifier` by default).

**Implementation:**
- Swap `lifecycle.rs`'s `AdapterRegistry::new(...)` call to construct and pass a real `PolicyEvaluator` (loaded from the effective merged config) instead of `DenyByDefaultAuthorization`
- Implement credential store for `workerMcp` connections
- Replace `RejectAllWorkerVerifier` with real credential verification

**References:** `docs/architecture.md` §10, §15, ADR-0013

---

### 3. Redaction regex denylist expansion

**Status:** Open  
**Priority:** Low  
**Labels:** security, defense-in-depth

**Description:**
The redaction regex denylist is intentionally small (API-key/bearer shapes); classification is the primary boundary. Expanding the denylist (`ghp_`, `AKIA…`, JWT shapes) is planned defense-in-depth.

**Implementation:**
- Add regex patterns for GitHub personal access tokens (`ghp_`)
- Add regex patterns for AWS access key IDs (`AKIA…`)
- Add regex patterns for JWT shapes
- Update `crates/runtime/src/security/redaction.rs`

**References:** `docs/architecture.md` §5

---

### 4. Subscription forwarder reaping

**Status:** Open (low priority)  
**Priority:** Low  
**Labels:** cleanup, subscription

**Description:**
Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast; harmless in practice since a closed connection's own `events_rx.recv()` loop (`spawn_subscription`) exits on its own `Err` the next time anything is broadcast.

**Implementation:**
- Optional: add explicit reaping logic for closed connections
- Current behavior is acceptable; no fix needed

**References:** `docs/architecture.md` §6

---

### 5. Remote service integration

**Status:** Open  
**Priority:** Future  
**Labels:** future-milestone, remote-services

**Description:**
Remote service integration (cloud storage, external APIs) is explicitly out of scope for this milestone.

**Implementation:**
- Future milestone work
- No current action required

**References:** `docs/architecture.md` §19

---

## M2/M3 Gap-Closure Discrepancies (found analyzing original plan suite, 2026-07-31)

### 10. `coordination-mcp` CLI subcommand does not exist — worker MCP integration is broken end-to-end

**Status:** Open  
**Priority:** Critical  
**Labels:** bug, adapter, worker-mcp, cli

**Description:**
`crates/runtime/src/adapter/mcp_config.rs` unconditionally configures every spawned worker CLI (Claude, Codex, Copilot, etc.) to launch its MCP server via `<batcave_path> coordination-mcp --state-dir <dir> --repo <repo>` — confirmed by `coordination_mcp_argv`/`coordination_mcp_server_config`/`coordination_mcp_config_document`, each with passing unit tests asserting this exact argv shape (`["coordination-mcp", "--state-dir", ..., "--repo", ...]`).

But `crates/runtime/src/cli.rs`'s `Command` enum has exactly 7 variants — `Serve`, `Status`, `Stop`, `Monitor`, `Version`, `Schema`, `Audit` — no `CoordinationMcp`/`coordination-mcp` subcommand exists. This is confirmed by the exhaustive `match cli.command { ... }` block (no wildcard arm; the code compiles, so no 8th variant exists). Any worker whose harness spawns this MCP server entry will fail immediately with an unrecognized-subcommand error, breaking `batman_task`/`batman_send` tool access for every adapter that relies on `workerMcp` — the entire mechanism `crate::coordination::mcp` and `crate::coordination::mcp_protocol` implement (and test) has no CLI entry point to actually invoke it as a subprocess.

**Implementation:**
- Add a `CoordinationMcp { state_dir: Option<PathBuf>, repo: PathBuf, ... }` variant to `cli.rs`'s `Command` enum
- Wire it to the existing, tested `crate::coordination::mcp` stdio MCP server implementation
- Add an integration test that actually spawns `batcave coordination-mcp` as a subprocess and confirms it serves the tool schemas in `crate::coordination::mcp_protocol`

**References:** `crates/runtime/src/adapter/mcp_config.rs`, `crates/runtime/src/cli.rs`, `crates/runtime/src/coordination/mcp.rs`, `crates/runtime/src/coordination/mcp_protocol.rs`

---

### 11. `batcave display probe` subcommand does not exist despite being marked "Closed" in the M2/M3 gap-closure doc

**Status:** Open  
**Priority:** Medium  
**Labels:** bug, display, cli, documentation

**Description:**
The `2026-07-27-batman-m2-m3-gap-closure.md` plan doc's readiness matrix claims: "`batcave` has no `display probe` subcommand... Resolution: Add `Display { Probe { backend, json } }` subcommand... Status: Closed (2026-07-27)." This is directly contradicted by the actual code: `cli.rs`'s `Command` enum (verified exhaustively, same 7 variants as item 10) has no `Display` variant at all. The underlying probe logic likely exists in `display/herdr.rs`/`display/tmux.rs` (both substantial, pane-level implementations with real test coverage per `crates/runtime/tests/display_registry.rs`, `herdr_display.rs`, `tmux_display.rs`), but it is not exposed as a CLI entry point.

**Implementation:**
- Add the `Display { Probe { backend, json } }` subcommand as originally specified in the gap-closure doc, wired to the existing herdr/tmux probe logic
- Or, if this was intentionally descoped, correct the gap-closure doc's "Closed" status to avoid future confusion (the doc itself warns three other claims in prior docs were "provably false" — this is a fourth)

**References:** `.../2026-07-27-batman-m2-m3-gap-closure.md`, `crates/runtime/src/cli.rs`, `crates/runtime/src/display/herdr.rs`, `crates/runtime/src/display/tmux.rs`

---

### 12. Crash recovery is a single untested file, far short of the plan's multi-module kill-point-tested coordinator

**Status:** Open  
**Priority:** High  
**Labels:** testing, recovery, hardening

**Description:**
The Hardening plan (Task 3) specifies a `crates/runtime/src/recovery/` module with separate `mod.rs`, `operations.rs`, `workers.rs`, `orphans.rs`, `workspaces.rs` files, a `RecoveryCoordinator::run()` that blocks all mutation methods until it completes, and a deterministic kill-point test matrix covering 6 distinct crash points per operation (intent / identity allocation / child spawn / vendor acknowledgement / event append / projection update) plus 8 specific invariants (no duplicated prompts, `unknown` for unacknowledged messages, vendor-resumable sessions resuming via new process, PID+start-identity+executable verification before reconnect, parent-scoped runs becoming `lost`, orphaned runs pausing with no new children, protected active workspace leases, quarantined stale materialization). **Verified 2026-07-31:** only a single flat `crates/runtime/src/recovery.rs` (210 lines) exists — confirmed via `glob crates/runtime/src/recovery/*` returning no matches — with zero test coverage (no inline `#[cfg(test)]`, no `crates/runtime/tests/recovery.rs`). `docs/getting-started.md` documents a `RecoveryCoordinator` and `RecoveryConfig` from a contributor/build perspective, but that doesn't substitute for the missing kill-point test matrix. `doctor.rs` (252 lines, also flat rather than the planned `doctor/{mod,check,report}.rs` submodule) is equally untested.

**Implementation:**
- Audit the flat `recovery.rs` against the 6 kill-points / 8 invariants above; restructure into the specified submodules if it doesn't already cover them
- Add `crates/runtime/tests/recovery.rs` implementing the deterministic kill-point matrix (`--test-threads=1` per the plan, since it manipulates real process state)
- Add `crates/runtime/tests/doctor.rs` covering each check function and the aggregate report shape

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 3, Task 4), `crates/runtime/src/recovery.rs`, `crates/runtime/src/doctor.rs`, `docs/getting-started.md`

---

### 13. No CI workflow runs on ordinary pushes/PRs — only the release-tag-triggered workflow exists

**Status:** Open  
**Priority:** High  
**Labels:** ci, testing, release

**Description:**
The Hardening plan (Task 5) requires a `.github/workflows/ci.yml` separate from `release.yml`, running formatting, Clippy with warnings denied, Rust/Bun tests, generation-drift checks, package tests, a secret scan, and a dependency audit on every push — before any release artifact is ever built. **Verified 2026-07-31:** `.github/workflows/ci.yml` does not exist (confirmed via glob); `.github/workflows/` contains only the tag-triggered `release.yml`, which itself has no test step at all (`cargo build --release` straight into packaging — see the release-pipeline fix from 2026-07-31's session). This means nothing currently blocks a broken commit from being merged, let alone released.

**Implementation:**
- Add `.github/workflows/ci.yml` triggered on push/PR: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `bun run generate --check`, `bun test`
- Add a dependency-audit step (`cargo audit`/`pip-audit`-equivalent for the JS side) and a secret-scan step

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 5), `.github/workflows/`

---

### 14. Releases are not gated on adapter conformance — the root-level conformance/install test suites don't exist

**Status:** Partially implemented — structural gate wired, but conformance runner is a stub that doesn't invoke real adapter checks  
**Priority:** High  
**Labels:** ci, testing, conformance, release

**Description:**
The Hardening plan (Task 6) requires `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, and `tests/install/private-registry.test.ts` at the repo root, wired into `release.yml` so the workflow "refuses publish unless every advertised capability has a passing scenario on the target build." **Implemented 2026-08-01 (partial):** `tests/conformance/run.ts` and `tests/conformance/assert-report.ts` exist as **non-functional stubs** that write empty reports and only check field presence (not that scenarios actually ran or passed). `tests/install/private-registry.test.ts` is also a stub. The conformance job is wired into `release.yml` before publish, but it always passes because the stubs never fail. Real implementation would spawn `batcave conformance` commands and validate actual scenario results. Marked as "implemented but unverified in CI" per partial-verification approach.

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 6), `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, `tests/install/private-registry.test.ts`, `.github/workflows/release.yml`

---

### 15. `batcave doctor` CLI command and `/batman-doctor` OMP command implemented

**Status:** Implemented (COMPLETED 2026-08-01)
**Priority:** Medium  
**Labels:** cli, doctor, extension

**Description:**
The Hardening plan (Task 4) specifies `batcave doctor --json` as a standalone CLI command returning structured checks (`id`, `status`, evidence, remediation, `productionBlocking`), plus a `packages/extension/src/doctor.ts` rendering the same report via an OMP `/batman-doctor` command. **Implemented 2026-08-01:** `cli.rs` now has a `Doctor` variant (lines 92-103) wired to `run_doctor()` (lines 361-458). The underlying `doctor.rs` check logic (252 lines) is invoked by the CLI. `packages/extension/src/doctor.ts` provides `runDoctorCommand()` and `buildDoctorContext()` for direct CLI invocation (no runtime connection). `index.ts` registers both `batman_doctor` tool and `/batman-doctor` slash command. Integration tests at `crates/runtime/tests/doctor.rs` verify the CLI behavior (4 tests, all passing). Manual smoke test confirms JSON output format.

**Implementation:**
- Added `Doctor { state_dir, repo, json }` variant to `cli.rs`'s `Command` enum
- Wired `Command::Doctor` to `run_doctor()` in the match block (line 167-171)
- Fixed corrupted `run_doctor()` function (removed duplicate match blocks)
- Added `Serialize` derive to `DoctorResult` and `FailedCheck` in `doctor.rs`
- Created `packages/extension/src/doctor.ts` with `runDoctorCommand()` and `buildDoctorContext()`
- Registered `batman_doctor` tool and `/batman-doctor` command in `index.ts`
- Added integration tests at `crates/runtime/tests/doctor.rs` (4 tests, all passing)
- Manual smoke test confirms JSON output: `{"error":"...","healthy":false}`

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 4), `crates/runtime/src/cli.rs`, `crates/runtime/src/doctor.rs`, `packages/extension/src/doctor.ts`, `packages/extension/src/index.ts`
---

### 16. Operator-facing docs (Tasks 7-8) aren't split out as the plan specifies, and no release-candidate checklist exists

**Status:** Implemented (COMPLETED 2026-08-01)
**Priority:** Low  
**Labels:** documentation, release

**Description:**
The Hardening plan's Tasks 7-8 specify six standalone operator docs (`docs/installation.md`, `configuration.md`, `operations.md`, `security.md`, `compatibility.md`, `recovery.md`) generated from verified command output, plus `release/0.1.0-checklist.json` recording every gate's pass/blocked status with evidence digests. **Verified 2026-07-31:** none of the six named files or the checklist exist as separate files (glob, zero matches) — but this is *not* a documentation vacuum: `docs/getting-started.md` already covers installation, configuration (including the layered-precedence system), security/redaction, and a `RecoveryCoordinator`/`RecoveryConfig` section in real detail, and `docs/architecture.md` independently documents the exact `PolicyEvaluator`-not-wired gap from item 2. The real gaps are narrower: (a) that content is written for contributors building from source, not operators running a packaged release, and getting-started.md says so explicitly; (b) there is no `compatibility.md` generated from actual conformance-report data (tested CLI versions, unsupported operations, degraded fallback, evidence date); (c) there is no dedicated `operations.md` covering daemon lifecycle commands, coordinated Herdr-restart warnings, package rollback, or uninstall-preserves-state behavior; (d) no release-candidate checklist artifact exists at all.

**Implementation:**
- Once items 10-15 close, generate `docs/compatibility.md` from a real passing conformance report (per-harness protocol/version, tested capabilities, unsupported operations, evidence date)
- Write `docs/operations.md` covering daemon start/stop/restart, Herdr coordinated-restart warning, package rollback, and uninstall semantics
- Decide whether to split `getting-started.md`'s existing installation/configuration/security/recovery content into the remaining four named docs, or keep it consolidated and update the plan's expectation
- Generate `release/0.1.0-checklist.json` once the above gates are closed

**References:** `.../2026-07-22-batman-hardening-release.md` (Task 7, Task 8), `docs/getting-started.md`, `docs/architecture.md`

---

## Feature Requests

### Org Config: URL or File Path Support

**Status:** Not Started  
**Priority:** Medium  
**Labels:** enhancement, configuration

**Description:**
Currently, org config is loaded only from file paths. This should be enhanced to support either:
- A file path (current behavior)
- A URL (HTTP/HTTPS) for remote configuration

**Implementation Notes:**
- Modify `crates/runtime/src/config/merge.rs` `load_layer` function
- Detect if the path is a URL (starts with `http://` or `https://`)
- If URL, fetch the content and parse as YAML
- If file path, load from disk (current behavior)
- Add appropriate error handling for network failures
- Consider caching fetched URLs to avoid repeated network calls

**Example Usage:**
```bash
# File path (current)
batman serve --org-config /etc/batman/org.yaml

# URL (new)
batman serve --org-config https://config.example.com/org.yaml
```

**Dependencies:**
- Network access for URL fetching
- TLS certificate validation for HTTPS
- Timeout handling for network requests

---

## Other Potential Features

- [ ] Add support for config templates
- [ ] Add config validation against schema before loading
- [ ] Add config versioning and migration support
- [ ] Add config encryption for sensitive values

---

## Adapter Implementation Gaps (from conformance test failures)

### 6. Claude adapter: missing lifecycle/usage/result event extraction

**Status:** Open  
**Priority:** Medium  
**Labels:** adapter, claude, conformance

**Description:**
The Claude adapter's normalizer reads `initialize.jsonl` (a real Claude CLI session's stdout containing session start, streaming text, tool calls, and final result with usage/cost data) but does not extract three critical signals:
- `VendorSessionEstablished` — proves which session is being tracked
- `UsageReported` — proves token consumption and cost data
- `MessageFinal(role="result")` — proves the turn is complete

Without these, the runtime cannot track session lifecycle, report usage to OMP, or know when a turn is truly complete.

**Implementation:**
- Enhance `ClaudeNormalizer` to extract `VendorSessionEstablished` from the initialize frame
- Extract `UsageReported` from the result frame's usage data (input/output tokens, cost)
- Extract `MessageFinal(role="result")` from the result frame's text content
- Ensure all three correlate to the same session ID

**References:** `fixtures/adapters/claude/initialize.jsonl`, `crates/runtime/src/adapter/claude/normalize.rs`

---

### 7. Codex adapter: missing lifecycle/usage/artifact event extraction

**Status:** Open  
**Priority:** Medium  
**Labels:** adapter, codex, conformance

**Description:**
The Codex adapter's normalizer reads `thread-turn.jsonl` (a real Codex thread transcript containing text chunks, tool calls, token usage, and file change artifacts) but does not extract six critical event types:
- `MessageChunk` — streaming text chunks
- `MessageFinal` — final message
- `ToolStarted` — when a command begins
- `ToolResult` — when a command finishes
- `UsageReported` — token counts
- `ArtifactProduced` — file changes

Without these, OMP cannot track costs, know which files were changed by the worker, or correlate the full turn lifecycle.

**Implementation:**
- Enhance `CodexNormalizer` to extract all six event types from the thread transcript
- Ensure all events correlate to the same thread/run/task/worker IDs
- Verify the hidden `reasoning` content is still dropped before reaching visible events

**References:** `fixtures/adapters/codex/thread-turn.jsonl`, `crates/runtime/src/adapter/codex/normalize.rs`

---

### 8. Copilot adapter: ACP v1 protocol limitation on usage reporting

**Status:** Known gap (protocol limitation)  
**Priority:** Low  
**Labels:** adapter, copilot, protocol

**Description:**
ACP v1 does not transmit token usage or cost information in its session update frames. The Copilot adapter honestly declares `usage: none` rather than pretending to report something it cannot see. This is a protocol limitation, not a code bug.

**What can be done:**
- Wait for ACP v2 (or future protocol version) that transmits usage data
- No code changes possible until the protocol evolves

**References:** `fixtures/adapters/copilot/session-updates.jsonl`, `crates/runtime/src/adapter/copilot/client.rs`

---

### 9. Copilot live test: requires authenticated CLI session

**Status:** Environment dependency  
**Priority:** Low  
**Labels:** testing, environment

**Description:**
The `real_binary_initialize_and_session_list_never_invoke_a_model` test requires a real, authenticated Copilot CLI session. Without one, the test cannot run — that's expected and documented. This is not a code gap; it's an environment gap.

**What can be done:**
- Run tests with `BATMAN_LIVE_COPILOT=1` and a valid Copilot session
- No code changes needed

**References:** `docs/manual-testing.md` §4c
