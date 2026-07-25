# Known gaps

Every worker adapter's conformance suite (`crates/runtime/src/adapter/*/conformance.rs`) reports
scenario failures **honestly** — a scenario that cannot be proven without breaking the "zero paid
model calls in fixture mode" invariant reports `passed: false` with a concrete reason, never a
fabricated pass. This document catalogs every such gap as of the Worker Adapters milestone, in
four categories: proof limits a live run resolves today (not permanent), a protocol-version wall
no call can resolve, genuine implementation gaps in this codebase worth fixing, and ordinary
environment dependencies that aren't gaps at all.

Verify the current state directly rather than trusting this document blindly — it describes one
point in time, and vendor CLIs update themselves:

```bash
cargo build -p batman-runtime
./target/debug/batcave conformance --adapter all --fixture --output /tmp/gaps-check.json
python3 -c '
import json
for r in json.load(open("/tmp/gaps-check.json")):
    fails = [s["name"] for s in r["scenarios"] if not s["passed"]]
    if fails: print(r["adapter"], fails)
'
```

## Fixture-mode proof limits (resolvable via a real, gated live run — not permanent)

These are not bugs in this codebase. Each one is a real property of the installed vendor binary;
proving the *positive* case genuinely requires spending a real, billed model call, which
fixture-mode conformance must never do by design. `live_report()` (gated per-adapter on
`BATMAN_LIVE_<ADAPTER>=1`) proves each of these for real when a human deliberately runs it — see
`docs/manual-testing.md` §4c. None of these are permanent: a real live run resolves them today,
right now, for whoever sets the gate.

- **Codex: `follow_up`, `session_resume`, `runtime_restart`, `cancellation_scope`
  (`CancelScope::Turn`)** — the installed `codex-cli` (0.145.0) does not write a thread's resumable
  rollout file to disk until a turn actually runs. A bare `thread/start` with no turn leaves no
  rollout at all; `Adapter::resume()` against such a thread fails with a real vendor error ("no
  rollout found for thread id ..."). `turn/start` is exactly what invokes the model, so none of
  these four can be proven honestly in fixture mode. See
  `crates/runtime/src/adapter/codex/conformance.rs`'s `unprovable_without_a_live_turn` helper and
  its `live_report()`, which runs one real turn, a real follow-up, a real mid-flight cancellation,
  and a real cross-instance resume to prove all four for real when its gate is set.
- **Copilot: `session_resume`, `runtime_restart`** — the installed CLI (1.0.75) does not persist a
  freshly-created, never-prompted session in a form a brand-new process can reach via
  `session/load` alone; empirically confirmed with a real cross-process probe
  (`crates/runtime/src/adapter/copilot/conformance.rs::session_resume_probe`). Reaching it that way
  would require an actual turn — a real model call, exactly like Codex above. A future/different
  CLI version might persist it without a turn at all; the check is written to pass automatically
  if that ever changes, it currently just doesn't with 1.0.75.

## Protocol wall (not resolvable by any model call — needs a vendor protocol upgrade)

- **Copilot: `unexpected_child_observation`** — ACP protocol **v1** has no `session/update` variant
  representing a vendor-spawned subagent at all; this is a protocol-version limit, not an adapter
  bug and not something a live run could prove either. `normalize.rs`'s fallback correctly drops
  any unrecognized update to zero events rather than fabricate a `NestedWorkerObserved`. Resolvable
  only by Copilot negotiating a newer ACP protocol version that adds such a variant — not something
  this codebase controls; revisit if/when this adapter's compatibility table gains a v2 entry.

## Genuine implementation gaps (worth investigating; not vendor-imposed)

These are missing behavior in **this codebase's own adapter implementations** — the vendor
protocol/CLI has no fundamental obstacle to fixing them, this milestone simply didn't implement
that path yet.

- **OMP-RPC: `approval` never normalized** — `omp_rpc/normalize.rs`'s catch-all silently drops the
  real vendor's `extension_ui_request` frame to zero events, and `SharedRunState` tracks no
  pending-approval state for this adapter at all (unlike the Claude adapter's own `PendingApproval`
  bookkeeping). `ApprovalsCapability::Observable` is currently declared but not actually backed by
  any observable event or internal state. Confirmed against `fixtures/adapters/omp-rpc/turn.jsonl`'s
  own real `extension_ui_request` line — see
  `crates/runtime/src/adapter/omp_rpc/conformance.rs::approval_scenario`. **Fix shape:** add an
  `extension_ui_request` → `AdapterEventPayload` mapping in `normalize_frame`, and pending-approval
  tracking in `SharedRunState`/`snapshot()`, mirroring Claude's existing `PendingApproval` design.
- **OMP-RPC: no `ArtifactProduced` path at all** — `normalize.rs` has no case constructing this
  payload variant, and `snapshot()` hardcodes an empty `artifacts` list unconditionally. Currently
  scoped as "not applicable" in `result_usage_artifacts_scenario` (passing, with a caveat, since
  there is genuinely nothing to correlate yet) rather than a hard failure, but this is a real
  feature gap if OMP ever reports artifact-shaped output over this adapter's RPC channel. **Fix
  shape:** identify the real vendor frame(s) that carry artifact information (undetermined as of
  this writing — needs a live, artifact-producing session to observe) and add the corresponding
  `normalize_frame` case.
- **`AdapterRegistry` is not wired into the running daemon** — `lifecycle::serve()`'s
  `ServerConfig::default()` still leaves `run_driver: None` in production; `AdapterRegistry` exists
  and is fully tested (`cargo test -p batman-runtime --test adapter_registry`) but nothing
  constructs and installs one at daemon startup. Two sub-gaps block this, both documented in
  `crates/runtime/src/adapter/registry.rs`'s own module doc:
  - `RunDriverContext` (frozen, Task 1-era protocol) carries no prompt/message payload — by
    design, `run/submit` only ever carries `taskId`/`workerId`, never task content (OMP owns task
    content, this runtime never does). Delivering a run's actual instructions to an already-started
    adapter needs a message-forwarding seam translating a journaled `message/send` into a live
    `Adapter::send(AdapterMessage::FollowUp(..))` call — `RunDriver` has no method for this today.
    **Fix shape:** either add a method to `RunDriver`, or have `AdapterRegistry::start` itself
    subscribe to `events_tx` and forward matching message events to the adapter instance it's
    holding (no protocol change needed for that second option — see the module doc's own reasoning
    for why this is tractable without touching frozen types).
  - Adapters `AdapterRegistry` constructs never receive worker-coordination MCP config (`mcp: None`
    unconditionally) — wiring `crate::adapter::mcp_config::McpLaunchContext` needs a resolved
    `batcave` binary path, state directory, and repository root the registry is not currently
    constructed with. **Fix shape:** thread these three paths into `AdapterRegistry::new`
    (available at `lifecycle::serve()`'s own call site via `RuntimePaths`) and build the per-adapter
    `AdapterMcpConfig`/`Arc<CoordinationBroker>` (for OMP-RPC) before constructing each adapter in
    `build_adapter`.

## Environment-dependent, not a real gap

- **Every adapter's `probe` scenario** needs its own vendor CLI actually installed and reachable on
  `PATH` to report a real version — trivially true, listed only so an installer with a missing CLI
  isn't surprised by a `probe: false` entry.
- **OMP-RPC: `probe`** — depends only on `omp models --json` currently *listing* a `lm-studio`/
  `omlx` selector in its catalog; the model server itself need not be reachable for this one, only
  listed. This adapter deliberately never invents tool compatibility for a model it can't see —
  see `crates/runtime/src/adapter/omp_rpc/conformance.rs::resolve_first_local_selector`.
- **OMP-RPC: `cancellation_scope`, `follow_up`** — a stronger requirement than `probe`'s: both
  actually spawn a real `omp --mode rpc --model <selector>` process and wait for its `ready`
  handshake, which needs the selector to be genuinely reachable, not merely listed — empirically
  observed on this dev machine: `probe` can pass (the catalog still lists a selector) in the same
  run these two then fail (`spawning/handshaking against the installed omp binary failed, or the
  selector became unreachable between listing and spawn`), because LM Studio's own catalog entry
  can outlive the model actually being loaded/reachable. See `spawn_ready_client` in the same file.

Both are purely environment/infrastructure dependencies of *this* dev machine at *this* moment,
not a code defect, and not fixable by any change in this codebase; expect flakiness across
machines/time until a local model server is reliably both listed and reachable.
