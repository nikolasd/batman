# Using the `@nikolasd/batman` OMP Extension

**This is the BATMAN user manual.** Audience: anyone using BATMAN through OMP — you never need to
touch the source or build anything. This is the user-facing guide to the OMP extension itself:
what it registers, what each tool is for, and the recommended flow for running a task through an
external harness. For installing the extension see the [README](../README.md#installation); for
running/troubleshooting the daemon directly, see [`operations.md`](operations.md) and
[`cli-reference.md`](cli-reference.md); for whether your platform/adapter is supported, see
[`compatibility.md`](compatibility.md); for the wire protocol and internal architecture (a
contributor concern, not a usage one), see [`architecture.md`](architecture.md).

Once installed, the extension registers **11 tools** an LLM (or you, via slash commands) can call,
plus **3 slash commands**. Every tool shares one runtime connection per OMP session — the first
call connects to (or spawns) the repository's `batcave` daemon; every later call in the same session
reuses that connection.

## Health checks — start here if something seems wrong

### `batman_status` / `/batman-status`

Connects to the daemon (spawning it if needed) and reports whether it's reachable: protocol
version, project id, active run count, schema version, uptime, and which binary was used (a
development override via `OMP_BATMAN_BINARY`, or the installed platform package). Call this first
whenever you're unsure the daemon is up, or right after a connection failure.

### `batman_doctor` / `/batman-doctor`

Unlike `batman_status`, this does **not** try to connect to a live daemon — it invokes
`batcave doctor --json` directly against the repository's state, so it works even when nothing is
serving the repo yet. Use it when `batman_status` fails or the runtime won't start: it checks the
database, state directory permissions, platform support, schema compatibility, adapter
availability, disk space, and unresolved rollout gates. See
[`cli-reference.md`](cli-reference.md#batcave-doctor) for the full check catalog.

## The embedded monitor — `/batman`

`/batman` opens (or refreshes) a live widget above the editor showing every active run: state icon,
adapter/model, workspace mode, pending approvals, and latest activity — up to 7 rows. It subscribes
to the daemon's live event stream, so it updates itself with zero further input as runs progress,
and it replays from the daemon's journal across OMP restarts, so nothing is lost or duplicated.

`/batman status <runId>` prints the full detail block for one run: task, worker, state, harness/
model, flags, pending approvals, workspace mode, latest activity, first-seen and last-event
timestamps.

If the daemon is unreachable, the monitor degrades to inactive rather than blocking session
startup — running `/batman` again retries the connection.

## Orchestration tools

All 11 tools take an `op` parameter that selects the action; most support several. Approval tiers
(`read` / `write` / `exec`) gate whether OMP prompts before running the operation.

| Tool | Ops | Tier | Purpose |
|---|---|---|---|
| `batman_profile` | register | `exec` | Register a reusable (adapter, model, startup options) profile |
| `batman_worker` | create, list, get | `exec`/`read` | Provision or look up a worker identity for a harness/model |
| `batman_task` | upsert, get | `write`/`read` | Create or read the durable, cross-session unit of work |
| `batman_run` | submit, list, get, retry, cancel | `exec`/`read` | Execute, monitor, retry, or cancel a task on a worker |
| `batman_workspace` | acquire, get, inspect, apply, release | `exec`/`read` | Manage the git worktree/copy a run executes in |
| `batman_artifact` | list, fetch | `read` | Read patches, commit lists, conflict reports a run published |
| `batman_child` | list, decide | `exec`/`read` | Approve or deny a worker's request to spawn a nested child |
| `batman_violation` | decide | `exec` | Resolve a policy violation that quarantined a run |
| `batman_message` | send, list | `write` | Send/read coordination messages between workers in a run |
| `batman_approval` | list, decide | `exec` (always) | List and decide a worker's escalated approval request |
| `batman_reconcile` | (single-purpose) | `write` | Rebind task ownership after a dropped/reconnected session |

### Registering a profile — `batman_profile`

Register once per (adapter, model) combination before provisioning workers against it:
`adapter`, `model`, `startupOptions` (adapter-tagged, e.g. `{claude: {...}}`),
`environmentAllowlist?`, `permissionEnvelope?`. Registration is permanent for the runtime
database's lifetime — there's no update or delete; register a new profile instead of mutating one.
Then pass the returned `profileId` to `batman_worker { op: "create", profileId }` instead of
repeating the same fingerprint/adapter/model/permissionEnvelope on every worker.

### Finding or creating a worker — `batman_worker`

`op: "list"` before submitting a run, to see what's already provisioned for the repository.
`op: "create"` provisions a new worker identity (`fingerprint`, `adapter`, `model`, optionally
`profileId`, `permissionEnvelope`, `parentWorkerId`). `op: "get"` fetches one worker's details by
`workerId`. You need a `workerId` from `list` (or the one you just created) to submit a run.

### Creating a task — `batman_task`

`op: "upsert"` creates or updates the persistent, cross-session unit of work an external harness
will execute — distinct from OMP's own in-process subagent tasks. It persists to the SQLite
journal, survives session disconnects, and can be retried/cancelled/reconciled after failure.
`op: "get"` reads one back by `taskId`. After creating a task, select a worker with
`batman_worker { op: "list" }` and submit execution with `batman_run { op: "submit" }`.

### Running it — `batman_run`

`op: "submit"` requires `taskId`, `workerId`, and `prompt` (the instruction text the worker
executes — BATMAN stores no task text on its own). Optionally `workspaceMode`
(`shared`/`isolated`/`copy`) and `priority`. `op: "get"` checks a run's progress; `op: "list"` lists
runs for a task. `op: "retry"` re-executes a terminal run by starting a fresh worker process;
it always creates a **new** `runId` (never mutates the prior one), requires `priorRunId` and
`prompt` again, and accepts `workspaceMode` to match the original submission. Like `submit` and
`cancel`, `retry` is `exec` tier. `op: "cancel"` stops a running run.

Because `run/submit`'s error response carries no `runId`, if you need the id right after
submitting, follow up with `batman_run { op: "list", taskId }` rather than assuming submit
returned one.

### Managing the workspace — `batman_workspace`

`op: "acquire"` before a run that needs its own git worktree or copy — `requestedIsolation:
"gitWorktree"` lets concurrent workers on the same task run in true isolation without conflicting.
`op: "get"` fetches a lease's current path/state; `op: "inspect"` reads dirty/untracked file counts
and diverged commits; `op: "apply"` lands a patch or cherry-pick (`strategy`, `artifactId`,
`expectedTargetRevision`) into the workspace; `op: "release"` tears the lease down once the run is
done with it. A shared-mode write lease is exclusive project-wide; isolated leases (`gitWorktree` or
`copy`) never conflict with each other or with a shared lease.

### Reading the evidence — `batman_artifact`

`op: "list"` (optionally filtered by `kind`: `patch`/`commitList`/`conflictReport`/
`workspaceManifest`) shows what a run published; `op: "fetch"` with an `artifactId` reads its bytes.
Fetches are chunked — the response carries `nextOffset`; pass it back as `offset` to continue
reading a large artifact. Artifacts are scoped to the current task — a run on another task is never
visible.

### Nested-worker requests — `batman_child`

A worker that wants to spawn a child records only an *intent*; nothing happens until you decide.
`op: "list"` (optionally filtered by `runId`) shows pending requests; `op: "decide"` with
`parentRunId` and `decision: "accept"|"deny"` resolves one. Accepting requires `childTaskId`,
`childWorkerId`, `childRunId`; denying requires `reason`.

### Policy violations — `batman_violation`

When policy quarantines a run (for example, a worker spawning a nested child when policy forbids
it), pass the `violationId` from the violation event plus a `resolution` string. There is no `list`
op by design — violations surface through the event stream / `/batman` monitor, not a separate
query. The quarantined run makes no further progress until decided.

### Worker-to-worker messaging — `batman_message`

`op: "send"` (requires `runId`, `senderWorkerId`, `kind`, `payload`) sends a coordination message
during an active multi-worker run; `op: "list"` reviews a run's message history. `kind` is one of
`assign`, `steer`, `followUp`, `question`, `answer`, `peerMessage`, `approvalDecision`, `cancel`,
`shutdown`.

### Approvals — `batman_approval`

`op: "list"` (optionally filtered by `runId`) shows pending approvals, including whether each is
`humanRequired`; `op: "decide"` applies an `approve`/`deny` decision with an optional `reason`. The
runtime enforces `humanRequired` — this tool never auto-approves, not even on `list`. When a
decision targets a `humanRequired: true` approval and a UI is present, OMP shows an interactive
dialog (redacting any argument that looks like a token/secret/password/credential) instead of
trusting whatever decision the model supplied; the dialog times out after 5 minutes, leaving the
approval pending rather than deciding it either way.

### Reclaiming a session's tasks — `batman_reconcile`

Call this after a session drop or reconnect, if you had active tasks from a prior session. Rebinds
task ownership to the current session; requires the matching `taskId` and a monotonic `revision` —
the runtime rejects a rebind on revision mismatch to prevent races.

## A minimal end-to-end flow

The simplest path from nothing to a running task:

```
batman_worker { op: "create", fingerprint: "sha256:...", adapter: "claude", model: "..." }
batman_task   { op: "upsert", description: "..." }
batman_run    { op: "submit", taskId, workerId, prompt: "..." }
/batman                                        — watch it live
batman_run    { op: "get", runId }             — or poll explicitly
```

Register a `batman_profile` first if you're going to create more than one worker against the same
adapter/model — it saves repeating the startup options and permission envelope on every
`batman_worker { op: "create" }` call. For concurrent workers on the same task, acquire an isolated
workspace (`batman_workspace { op: "acquire", requestedIsolation: "gitWorktree" }`) per worker
before submitting their runs.

See [`docs/manual-testing.md`](manual-testing.md) for full worked sessions, including cross-worker
messaging while watching `/batman` update live, and a two-workspace concurrent-agent example.

## How the extension finds and starts `batcave`

You don't need to know this to use the tools above, but it explains what `batman_status` reports
and what `OMP_BATMAN_BINARY` is for:

1. On first use in a session, the extension tries to connect to the repository's existing runtime
   socket. If one answers, it's reused — no process is spawned.
2. If nothing answers, it picks a binary: `OMP_BATMAN_BINARY` (an absolute, executable path) wins
   outright if set — this is the local-development override, and it skips checksum/version
   validation entirely. Otherwise it resolves the platform-appropriate leaf package
   (`@nikolasd/batman-darwin-arm64`, `-darwin-x64`, `-linux-arm64-gnu`, or `-linux-x64-gnu`,
   selected by `process.platform`/`arch` and, on Linux, detected glibc vs. musl) and verifies the
   packaged binary's SHA-256 and version against its manifest before trusting it.
3. It spawns `batcave serve` detached, with `BATMAN_BINARY_SOURCE` set to `override` or `package`
   accordingly (this is the "Binary source" field `batman_status` reports), then retries connecting
   with bounded exponential backoff. If a different concurrent caller won the daemon's single-
   instance lock in the meantime, this session simply connects to that winner.
