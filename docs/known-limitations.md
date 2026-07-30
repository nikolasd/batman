# Known Limitations

This document catalogs all known technical limitations, constraints, and deferred items in the BATMAN system. These are consciously accepted as non-blocking for the current milestone but tracked for later resolution.

**Reference:** See [Architecture](architecture.md) for the sections where each limitation is discussed in context.

---

## Database and Persistence

### Events table missing columns

**Location:** §4 (The SQLite journal and the database actor)

The `events` table still only stores `run_id`, not `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` (`source` is still hardcoded `runtime`). 

- A *live* `events/event` notification's envelope carries `task_id`/`worker_id` (§11's `append_and_apply` sets them from its caller's parameters), but a *replayed* one from `events/replay` always has them `None` — `ipc/connection.rs::replay()` can only reconstruct an envelope from what the `events` table's columns hold.
- The monitor (§17) is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields.
- **Impact:** Any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` will get silently wrong (empty) results.

**Fix required:** Schema migration plus populating those columns in `append_and_apply`'s insert.

**Status:** Open — tracked in TODO.md.

---

## Security and Redaction

### Redaction regex denylist is intentionally small

**Location:** §5 (The redaction boundary)

The redaction regex denylist is intentionally small (API-key/bearer shapes); classification is the primary boundary.

- **Planned expansion:** `ghp_`, `AKIA…`, JWT shapes
- **Status:** Open — planned for later milestone as defense-in-depth.

---

## IPC and Connection Management

### Subscription forwarder tasks for closed connections are reaped lazily

**Location:** §6 (IPC: JSON-RPC 2.0 over bounded NDJSON)

Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast.

- **Why it's harmless:** A closed connection's own `events_rx.recv()` loop (`spawn_subscription`) exits on its own `Err` the next time anything is broadcast.
- **Status:** Open — low priority, no fix needed.

---

## Worker Adapters and Authorization

### Worker adapters not yet fully wired in production

**Location:** §10 (Domain records and lifecycle)

Worker adapters are implemented but not yet fully wired in production.

- The `AdapterRegistry` exists and implements `RunDriver` against Claude/Codex/Copilot/OMP-RPC adapters.
- However, production `ServerConfig::default()` uses `DenyByDefaultAuthorization` until the Hardening plan's `PolicyEvaluator` is wired.
- The credential store for `workerMcp` connections is not yet implemented (`RejectAllWorkerVerifier` by default).

**Status:** Implemented but gated by authorization layer.

---

## Platform and Runtime

### Workspaces, displays, and policy engine require adapter registry

**Location:** §8 (Platform packaging), §13 (OMP orchestration tools)

Workspaces, displays (Herdr/tmux), and a policy engine are implemented but require the adapter registry to be fully wired.

- The `WorkspaceLeaseService`, `WorkspaceMaterializer`, `DisplayRegistry` (Herdr/Tmux/Terminal), and `PolicyEvaluator` all exist in the codebase.

**Status:** Implemented, ready for production wiring when adapter registry is complete.

---

## Future Milestones

### Remote service integration

**Location:** Out of scope for current milestone

Remote service integration (cloud storage, external APIs) is explicitly out of scope for this milestone.

**Status:** Open — future milestone.
