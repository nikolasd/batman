# Known Limitations

This document catalogs all known technical limitations, constraints, and deferred items in the BATMAN system. These are consciously accepted as non-blocking for the current milestone but tracked for later resolution.

**Reference:** File/ADR pointers are given per item — `architecture.md` uses the C4 model (Level 1-4) rather than numbered `§N` sections.

---

## Database and Persistence

### Events table missing columns

**Location:** `crates/runtime/src/db/migrations.rs` (schema), `crates/runtime/src/domain/repository.rs::append_and_apply`

The `events` table still only stores `run_id`, not `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` (`source` is still hardcoded `runtime`). 

- A *live* `events/event` notification's envelope carries `task_id`/`worker_id` (`append_and_apply` sets them from its caller's parameters), but a *replayed* one from `events/replay` always has them `None` — `ipc/connection.rs::replay()` can only reconstruct an envelope from what the `events` table's columns hold.
- The monitor (`packages/extension/src/monitor/model.ts`) is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields.
- **Impact:** Any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` will get silently wrong (empty) results.

**Fix required:** Schema migration plus populating those columns in `append_and_apply`'s insert.

**Status:** Open — tracked in TODO.md.

---

## Security and Redaction

### Redaction regex denylist is intentionally small

**Location:** `crates/runtime/src/security/redaction.rs` (see [ADR-0006](adr/0006-type-enforced-redaction-boundary.md))

The redaction regex denylist is intentionally small (API-key/bearer shapes); classification is the primary boundary.

- **Planned expansion:** `ghp_`, `AKIA…`, JWT shapes
- **Status:** Open — planned for later milestone as defense-in-depth.

---

## IPC and Connection Management

### Subscription forwarder tasks for closed connections are reaped lazily

**Location:** `crates/runtime/src/ipc/connection.rs::spawn_subscription` (see [ADR-0004](adr/0004-json-rpc-2-over-bounded-ndjson-on-a-unix-socket.md))

Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast.

- **Why it's harmless:** A closed connection's own `events_rx.recv()` loop (`spawn_subscription`) exits on its own `Err` the next time anything is broadcast.
- **Status:** Open — low priority, no fix needed.

---

## Worker Adapters and Authorization

### Worker adapters not yet fully wired in production

**Location:** `crates/runtime/src/adapter/registry.rs` (see [ADR-0012](adr/0012-explicit-run-lifecycle-relation-runtime-evidence-only.md), [ADR-0013](adr/0013-injectable-run-driver-seam-fake-by-default.md))

Worker adapters are implemented but not yet fully wired in production.

- The `AdapterRegistry` exists and implements `RunDriver` against Claude/Codex/Copilot/OMP-RPC adapters.
- However, production `ServerConfig::default()` uses `DenyByDefaultAuthorization` until the Hardening plan's `PolicyEvaluator` is wired.
- The credential store for `workerMcp` connections is not yet implemented (`RejectAllWorkerVerifier` by default).

**Status:** Implemented but gated by authorization layer.

---

## Platform and Runtime

### Workspaces, displays, and policy engine require adapter registry

**Location:** `crates/runtime/src/workspace/`, `crates/runtime/src/display/` (see [ADR-0010](adr/0010-platform-binaries-as-npm-optional-leaf-packages.md)), and `packages/extension/src/tools/` (OMP orchestration tools)

Workspaces, displays (Herdr/tmux), and a policy engine are implemented but require the adapter registry to be fully wired.

- The `WorkspaceLeaseService`, `WorkspaceMaterializer`, `DisplayRegistry` (Herdr/Tmux/Terminal), and `PolicyEvaluator` all exist in the codebase.

**Status:** Implemented, ready for production wiring when adapter registry is complete.

---

## Known Gaps

These are gaps reported honestly by the conformance suites (`crates/runtime/src/adapter/*/conformance.rs`). They fall into four categories: proof limits a live run resolves today (not permanent), a protocol-version wall no call can resolve, genuine implementation gaps in this codebase worth fixing, and ordinary environment dependencies that aren't gaps at all.

### Fixture-mode proof limits (resolvable via a real, gated live run — not permanent)

These are not bugs in this codebase. Each one is a real property of the installed vendor binary; proving the *positive* case genuinely requires spending a real, billed model call, which fixture-mode conformance must never do by design. `live_report()` (gated per-adapter on `BATMAN_LIVE_<ADAPTER>=1`) proves each of these for real when a human deliberately runs it — see `docs/manual-testing.md` §4c.

- **Codex: `follow_up`, `session_resume`, `runtime_restart`, `cancellation_scope` (`CancelScope::Turn`)** — the installed `codex-cli` does not write a thread's resumable rollout file to disk until a turn actually runs. A bare `thread/start` with no turn leaves no rollout at all; `Adapter::resume()` against such a thread fails with a real vendor error ("no rollout found for thread id ..."). `turn/start` is exactly what invokes the model, so none of these four can be proven honestly in fixture mode. See `crates/runtime/src/adapter/codex/conformance.rs`'s `unprovable_without_a_live_turn` helper and its `live_report()`.
- **Copilot: `session_resume`, `runtime_restart`** — the installed CLI does not persist a freshly-created, never-prompted session in a form a brand-new process can reach via `session/load` alone; empirically confirmed with a real cross-process probe (`crates/runtime/src/adapter/copilot/conformance.rs::session_resume_probe`). Reaching it that way would require an actual turn — a real model call. A future/different CLI version might persist it without a turn at all; the check is written to pass automatically if that ever changes.

### Protocol wall (not resolvable by any model call — needs a vendor protocol upgrade)

- **Copilot: `unexpected_child_observation`** — ACP protocol **v1** has no `session/update` variant representing a vendor-spawned subagent at all; this is a protocol-version limit, not an adapter bug and not something a live run could prove either. `normalize.rs`'s fallback correctly drops any unrecognized update to zero events rather than fabricate a `NestedWorkerObserved`. Resolvable only by Copilot negotiating a newer ACP protocol version that adds such a variant — not something this codebase controls; revisit if/when this adapter's compatibility table gains a v2 entry.

### Genuine implementation gaps (worth investigating; not vendor-imposed)

These are missing behavior in **this codebase's own adapter implementations** — the vendor protocol/CLI has no fundamental obstacle to fixing them, this milestone simply didn't implement that path yet.

- **OMP-RPC: `approval` never normalized** — `omp_rpc/normalize.rs`'s catch-all silently drops the real vendor's `extension_ui_request` frame to zero events, and `SharedRunState` tracks no pending-approval state for this adapter at all (unlike the Claude adapter's own `PendingApproval` bookkeeping). `ApprovalsCapability::Observable` is currently declared but not actually backed by any observable event or internal state. Confirmed against `fixtures/adapters/omp-rpc/turn.jsonl`'s own real `extension_ui_request` line — see `crates/runtime/src/adapter/omp_rpc/conformance.rs::approval_scenario`. **Fix shape:** add an `extension_ui_request` → `AdapterEventPayload` mapping in `normalize_frame`, and pending-approval tracking in `SharedRunState`/`snapshot()`, mirroring Claude's existing `PendingApproval` design.
- **OMP-RPC: no `ArtifactProduced` path at all** — `normalize.rs` has no case constructing this payload variant, and `snapshot()` hardcodes an empty `artifacts` list unconditionally. Currently scoped as "not applicable" in `result_usage_artifacts_scenario` (passing, with a caveat, since there is genuinely nothing to correlate yet) rather than a hard failure, but this is a real feature gap if OMP ever reports artifact-shaped output over this adapter's RPC channel. **Fix shape:** identify the real vendor frame(s) that carry artifact information (undetermined as of this writing — needs a live, artifact-producing session to observe) and add the corresponding `normalize_frame` case.

### Environment-dependent, not a real gap

- **Every adapter's `probe` scenario** needs its own vendor CLI actually installed and reachable on `PATH` to report a real version — trivially true, listed only so an installer with a missing CLI isn't surprised by a `probe: false` entry.
- **OMP-RPC: `probe`** — depends only on `omp models --json` currently *listing* a `lm-studio`/`omlx` selector in its catalog; the model server itself need not be reachable for this one, only listed. This adapter deliberately never invents tool compatibility for a model it can't see — see `crates/runtime/src/adapter/omp_rpc/conformance.rs::resolve_first_local_selector`.
- **OMP-RPC: `cancellation_scope`, `follow_up`** — a stronger requirement than `probe`'s: both actually spawn a real `omp --mode rpc --model <selector>` process and wait for its `ready` handshake, which needs the selector to be genuinely reachable, not merely listed — empirically observed on this dev machine: `probe` can pass (the catalog still lists a selector) in the same run these two then fail (`spawning/handshaking against the installed omp binary failed, or the selector became unreachable between listing and spawn`), because LM Studio's own catalog entry can outlive the model actually being loaded/reachable. See `spawn_ready_client` in the same file.

Both are purely environment/infrastructure dependencies of *this* dev machine at *this* moment, not a code defect, and not fixable by any change in this codebase; expect flakiness across machines/time until a local model server is reliably both listed and reachable.

---

## Future Milestones

### Remote service integration

**Location:** Out of scope for current milestone

Remote service integration (cloud storage, external APIs) is explicitly out of scope for this milestone.

**Status:** Open — future milestone.

---

## Hardening Release (2026-07-22) Limitations

### PolicyViolationDecide is a stub implementation

**Location:** `crates/runtime/src/ipc/connection.rs::handle` (stub returns `ServiceError::internal("not yet implemented")`)

The `policy/violation/decide` RPC method is registered in `BatmanMethod` but returns a stub error. `OrchestrationService` has no `policy` field, no `decide_violation` fn, and no `DecideOutcome` enum at `crate::policy`. Actual quarantine-decision logic requires building a new service/module analogous to `approval::ApprovalService`.

**Status:** Deferred — requires new policy service module.

---

### Conformance tests are non-functional stubs

**Location:** `tests/conformance/run.ts`, `tests/conformance/assert-report.ts`, `tests/install/private-registry.test.ts`

These files exist but are **stubs** that:
- Write empty reports (no actual adapter checks)
- Only check field presence, not that scenarios ran or passed
- The conformance gate in `.github/workflows/release.yml` always passes

Real implementation would spawn `batcave conformance` commands and validate actual scenario results.

**Status:** Implemented but unverified in CI — marked as "partial" in TODO.md item 14.

---

### No JS/TS formatter configured

**Location:** `.github/workflows/ci.yml`

The CI format job only checks Rust (`cargo fmt`). No prettier/biome configured for JS/TS.

**Status:** Open — can be added in a future milestone.

---
