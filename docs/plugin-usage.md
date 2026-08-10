# Using the BATMAN OMP Extension

**This is the BATMAN user manual.** Audience: anyone using BATMAN through OMP — you drive it with
plain language, and the model calls the tools on your behalf. You never need to touch the source,
build anything, or write raw tool-call JSON yourself — that detail lives in Appendix A below, for
advanced users and for the model's own reference.

For installing the extension, see the [README](../README.md#installation). For running/
troubleshooting the daemon directly, see [`operations.md`](operations.md) and
[`cli-reference.md`](cli-reference.md). For whether your platform/adapter is supported, see
[`compatibility.md`](compatibility.md). For the wire protocol and internal architecture (a
contributor concern, not a usage one), see [`architecture.md`](architecture.md).

## 1. Install

```
/marketplace add nikolasd/batman
/marketplace install batman@batman
/batman-runtime-install
/batman-status
```

**This repository is private** — the marketplace step git-clones it, so it needs your own GitHub
read access to `nikolasd/batman` (an SSH key registered with GitHub, or a `gh auth login` session
backed by a git credential helper). `/batman-runtime-install` additionally needs a `GITHUB_TOKEN` or
`GH_TOKEN` environment variable, or that same `gh auth login` session, to download and verify the
release asset.

After installing, **restart your OMP session** — `/reload-plugins` only refreshes skills and slash
commands, not extension modules or tools, so the `batman_*` tools won't appear until a fresh
session picks up the newly installed extension module.

## 2. Confirm it works

Run `/batman-status`. A healthy runtime answers with exactly this shape (`formatStatus` in
`status.ts`):

```
BATMAN runtime: running
Protocol: 1.0 (healthy: true)
Project: 0f4c1d9a8b7e6f50
Active runs: 0
Schema version: 7
Uptime: 3s
Binary source: package
```

`Binary source: package` means the verified, downloaded-and-cached binary is running. `override`
means `OMP_BATMAN_BINARY` was set and is running instead — the local-development path, described in
Appendix B.

If this fails instead, skip to [When something breaks](#6-when-something-breaks).

## 3. Just ask

Once installed, you drive BATMAN with plain language — the model calls the tools. The three
installed skills (`batman-orchestration`, `batman-approvals`, `batman-troubleshooting`, under
`packages/extension/skills/`) already carry these workflows, so the model doesn't need a tool-call
hint from you. Some examples of what to say, and what happens:

| You say | What BATMAN does |
|---|---|
| "run the auth refactor on Claude" | Looks up (or creates) a worker for that adapter, upserts a task, submits a run |
| "...in its own worktree" / "don't touch my files" | Same, plus `workspaceMode: "isolated"` — the run gets its own git worktree |
| "...on a copy instead" | Same, plus `workspaceMode: "copy"` — a per-run copy of the repository |
| "run these three on separate workers" | Creates (or reuses) three workers, then submits three runs — each with its own `workspaceMode: "isolated"` so they don't collide on the same files |
| "how's that run doing?" | Polls `run/get`, or points you at `/batman` to watch it live |
| "that failed, try again" | Retries with the original prompt restated (BATMAN doesn't remember it for you) |
| "stop it" | Cancels the run |

## 4. Watching runs

`/batman` opens (or refreshes) a live widget above the editor showing every active run: state icon,
adapter/model, workspace mode, pending approvals, and latest activity — up to 7 rows. It subscribes
to the daemon's live event stream, so it updates itself with zero further input as runs progress,
and it replays from the daemon's journal across OMP restarts, so nothing is lost or duplicated. When
there's nothing running yet, it prints:

```
No BATMAN runs yet.
```

`/batman status <runId>` prints the full detail block for one run: task, worker, state, harness/
model, flags, pending approvals, workspace mode, latest activity, first-seen and last-event
timestamps.

If the daemon is unreachable, the monitor degrades to inactive rather than blocking session
startup — running `/batman` again retries the connection.

## 5. When BATMAN needs you

Some actions require an explicit human decision, and BATMAN never fabricates one on your behalf:

- **Approvals.** A worker's escalated action may require `humanRequired: true`. The runtime
  enforces this server-side and rejects a model-supplied decision for it. With an interactive UI
  present, you'll see a dialog (secrets redacted); without one, the approval simply stays pending
  until you decide — this is a fail-closed rule, not a bug.
- **Policy violations.** If policy quarantines a run (for example, a worker tries to spawn a nested
  child when policy forbids it), the run makes no further progress until the violation is resolved.
  These surface through the event stream and the `/batman` monitor, not a query you poll.
- **Nested-child requests.** A worker that wants to spawn a child records only an intent — nothing
  happens until it's accepted or denied.

## 6. When something breaks

Work through this ladder:

1. **`/batman-status`** — connects to (or spawns) the daemon and reports whether it's healthy.
2. **`/batman-runtime-install`** — downloads and verifies the `batcave` binary, if it's missing.
3. **`/batman-doctor`** — works even with no live daemon; runs the full check catalog (database,
   state directory permissions, platform support, schema compatibility, adapter availability, disk
   space, unresolved rollout gates, and more — see
   [`cli-reference.md`](cli-reference.md#batcave-doctor) for the complete list).

Every BATMAN tool failure has the same shape: text `"<method> failed: <message>"`,
`details: { code, message, data }`, `isError: true`. The `details.code` field maps to a fix:

| `details.code` | Fix |
|---|---|
| `runtime-not-installed` | Run `/batman-runtime-install` to download the binary. |
| `checksum-mismatch` | Re-run `/batman-runtime-install`. The cached binary doesn't match its manifest. |
| `version-mismatch` | Re-run `/batman-runtime-install`. The cached binary is for a different extension version. |
| `manifest-invalid` | Re-run `/batman-runtime-install`. The cached manifest is corrupt or for another platform. |
| `unsupported-platform` | BATMAN only supports macOS and glibc Linux, arm64/x64. |
| `connection-failed` | Run `/batman-doctor` for a detailed check without needing a live daemon. |
| `http-error` (from `/batman-runtime-install`) | **This repository is private.** Set `GITHUB_TOKEN`/`GH_TOKEN`, or run `gh auth login`, then retry the install. |

## Appendix A — tool reference

For advanced users, and for the model's own use: the extension registers **11 orchestration
tools**, plus `batman_status`/`/batman-status`, `batman_doctor`/`/batman-doctor`, and
`batman_runtime_install`/`/batman-runtime-install`. Every tool shares one runtime connection per OMP
session — the first call connects to (or spawns) the repository's `batcave` daemon; every later call
in the same session reuses that connection.

**Shared contract** (`packages/extension/src/tools/shared.ts`, `callOrchestration`): a successful
call's text content is `"<method>: <JSON.stringify(result)>"`, and `details` is the daemon's JSON
result **verbatim** — no wrapping, no renaming. A failed call's text is `"<method> failed: <message>"`,
with `details: { code, message, data }` and `isError: true`.

Approval tiers (`read` / `write` / `exec`) gate whether OMP prompts before running the operation.

| Tool | Ops | Tier | Purpose |
|---|---|---|---|
| `batman_profile` | register | `exec` | Register a reusable (adapter, model, startup options) profile |
| `batman_worker` | create, list, get | `exec`/`read` | Provision or look up a worker identity for a harness/model |
| `batman_task` | upsert, get | `write`/`read` | Create or read the durable, cross-session unit of work |
| `batman_run` | submit, list, get, retry, cancel | `exec`/`read` | Execute, monitor, retry, or cancel a task on a worker |
| `batman_workspace` | acquire, get, inspect, apply, release | `exec`/`read` | Manage the git worktree/copy a run executes in |
| `batman_artifact` | list, fetch | `read` | Read patches, commit lists, conflict reports a run published |
| `batman_child` | list, decide | `read`/`exec` | Approve or deny a worker's request to spawn a nested child |
| `batman_violation` | decide | `exec` | Resolve a policy violation that quarantined a run |
| `batman_message` | send, list | `write` | Send/read coordination messages between workers in a run |
| `batman_approval` | list, decide | `exec` (always) | List and decide a worker's escalated approval request |
| `batman_reconcile` | (single-purpose) | `write` | Rebind task ownership after a dropped/reconnected session |

### `task/upsert`

```json
{ "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f", "sequence": 42 }
```

### `task/get`

```json
{
  "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f",
  "projectId": "0f4c1d9a8b7e6f50",
  "ownerClientInstanceId": "client-a1b2c3d4",
  "revision": 3,
  "createdAt": "2026-08-10T14:02:11Z",
  "updatedAt": "2026-08-10T14:05:47Z"
}
```

### `profile/register`

```json
{ "profileId": "3c9e2f1a-7d4b-4e8a-9c1d-2b3a4c5d6e7f", "fingerprint": "sha256:a1b2c3d4e5f6" }
```

### `worker/create`

```json
{ "workerId": "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d", "sequence": 43 }
```

### `worker/list`

```json
{
  "workers": [
    {
      "workerId": "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
      "parentWorkerId": null,
      "createdAt": "2026-08-10T14:00:00Z",
      "profileRef": { "id": "3c9e2f1a-7d4b-4e8a-9c1d-2b3a4c5d6e7f", "fingerprint": "sha256:a1b2c3d4e5f6", "adapter": "claude", "model": "claude-opus-4" }
    }
  ]
}
```

### `worker/get`

Same as one `worker/list` entry, plus `projectId` and `profileRef.permissionEnvelope`:

```json
{
  "workerId": "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
  "projectId": "0f4c1d9a8b7e6f50",
  "parentWorkerId": null,
  "createdAt": "2026-08-10T14:00:00Z",
  "profileRef": {
    "id": "3c9e2f1a-7d4b-4e8a-9c1d-2b3a4c5d6e7f",
    "fingerprint": "sha256:a1b2c3d4e5f6",
    "adapter": "claude",
    "model": "claude-opus-4",
    "permissionEnvelope": { "allow": ["read", "write"] }
  }
}
```

### `run/submit`

```json
{ "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e", "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f", "sequence": 44 }
```

Plus `workspacePath` and `workspaceMode: "isolated"` when a workspace was materialized, plus
`display` when a monitor pane was selected:

```json
{
  "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
  "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f",
  "sequence": 44,
  "workspacePath": "/Users/you/.omp/orchestrator/repos/<repository-id>/worktrees/b2c3d4e5",
  "workspaceMode": "isolated"
}
```

### `run/retry`

Same as `run/submit`, plus `priorRunId`:

```json
{ "runId": "c3d4e5f6-a7b8-4c9d-0e1f-2a3b4c5d6e7f", "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f", "sequence": 45, "priorRunId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e" }
```

### `run/get` (and each entry of `run/list`)

```json
{
  "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
  "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f",
  "workerId": "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
  "state": "working",
  "flags": {
    "degradedControl": false,
    "needsReconciliation": false,
    "protocolUnhealthy": false,
    "policyQuarantined": false,
    "workspaceDirty": true,
    "childrenActive": false
  },
  "vendorSessionId": "vendor-session-9f8e7d6c",
  "createdAt": "2026-08-10T14:02:20Z",
  "startedAt": "2026-08-10T14:02:21Z",
  "completedAt": null,
  "policyFingerprint": "sha256:f1e2d3c4b5a6"
}
```

### `run/list`

```json
{ "runs": [ /* … one object shaped like run/get above, per run … */ ] }
```

### `run/cancel`

```json
{ "sequence": 46 }
```

### `workspace/acquire`

```json
{
  "leaseId": "d4e5f6a7-b8c9-4d0e-1f2a-3b4c5d6e7f8a",
  "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
  "mode": "write",
  "isolationKind": "gitWorktree",
  "path": "/Users/you/.omp/orchestrator/repos/<repository-id>/worktrees/b2c3d4e5",
  "state": "active",
  "baseRevision": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
  "acquisitionSequence": 47
}
```

### `workspace/get`

```json
{
  "leaseId": "d4e5f6a7-b8c9-4d0e-1f2a-3b4c5d6e7f8a",
  "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
  "mode": "write",
  "isolationKind": "gitWorktree",
  "path": "/Users/you/.omp/orchestrator/repos/<repository-id>/worktrees/b2c3d4e5",
  "state": "active",
  "baseRevision": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"
}
```

### `workspace/inspect`

```json
{
  "leaseId": "d4e5f6a7-b8c9-4d0e-1f2a-3b4c5d6e7f8a",
  "patchArtifactId": "e5f6a7b8-c9d0-4e1f-2a3b-4c5d6e7f8a9b",
  "commitCount": 3,
  "commitIds": ["a1b2c3d", "b2c3d4e", "c3d4e5f"],
  "dirtyFileCount": 2,
  "untrackedFileCount": 1,
  "baseRevision": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
  "currentRevision": "b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1"
}
```

### `workspace/apply`

```json
{ "leaseId": "d4e5f6a7-b8c9-4d0e-1f2a-3b4c5d6e7f8a", "success": true, "conflictArtifactId": null, "targetRevisionAfter": "c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2", "errorCode": null }
```

### `workspace/release`

```json
{ "released": true, "cleanupFailed": false }
```

### `artifact/list`

```json
{ "artifacts": [ /* … artifact entries, filterable by kind on the request … */ ] }
```

### `artifact/fetch`

```json
{ "artifact": { "artifactId": "e5f6a7b8-c9d0-4e1f-2a3b-4c5d6e7f8a9b", "kind": "patch" }, "contentBase64": "ZGlmZiAtLWdpdCBhL2ZvbyBiL2Zvbwo=", "nextOffset": 4096, "complete": false }
```

### `message/send`

```json
{ "messageId": "f6a7b8c9-d0e1-4f2a-3b4c-5d6e7f8a9b0c", "sequence": 48 }
```

### `message/list`

```json
{
  "messages": [
    {
      "messageId": "f6a7b8c9-d0e1-4f2a-3b4c-5d6e7f8a9b0c",
      "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
      "senderWorkerId": "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
      "recipientWorkerId": null,
      "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f",
      "kind": "steer",
      "payload": "focus on the error path first",
      "deliveryState": "acknowledged",
      "createdAt": "2026-08-10T14:03:00Z",
      "sentAt": "2026-08-10T14:03:00Z",
      "acknowledgedAt": "2026-08-10T14:03:02Z",
      "replyTo": null
    }
  ]
}
```

### `approval/list`

```json
{
  "approvals": [
    {
      "approvalId": "a7b8c9d0-e1f2-4a3b-4c5d-6e7f8a9b0c1d",
      "runId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
      "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f",
      "action": "runShellCommand",
      "arguments": { "command": "rm -rf /tmp/scratch" },
      "humanRequired": true,
      "policyReason": "destructive command outside allowlist",
      "createdAt": "2026-08-10T14:04:00Z",
      "decidedAt": null,
      "decision": null
    }
  ]
}
```

### `approval/decide`

Request takes `decision: "approve" | "deny"`; the response's `outcome` reports what happened to
that decision, not the decision itself:

```json
{ "approvalId": "a7b8c9d0-e1f2-4a3b-4c5d-6e7f8a9b0c1d", "outcome": "decided" }
```

`outcome` is `"decided"`, `"decidedCallbackFailed"` (decided, but notifying the waiting worker
failed — the decision still stands), or `"alreadyDecided"` (a no-op repeat of an identical prior
decision).

### `violation/decide`

Request takes `resolution: "release" | "cancel"` (releases the run from quarantine, or cancels it
outright); the response's `outcome` is `"decided"` or `"alreadyDecided"`:

```json
{ "violationId": "b8c9d0e1-f2a3-4b4c-5d6e-7f8a9b0c1d2e", "outcome": "decided" }
```

### `child/list`

```json
{ "requests": [ /* … pending child-spawn requests, one JSON object per request … */ ] }
```

### `child/decide`

```json
{ "sequence": 49 }
```

### `reconcile/omp`

```json
{ "taskId": "5f0b6b3e-6b1a-4b8e-9c2d-1a2b3c4d5e6f", "newOwnerClientInstanceId": "client-b2c3d4e5", "sequence": 50 }
```

### `batman_runtime_install`

Success (text, then `details`):

```
BATMAN runtime installed: batcave 0.1.0 (darwin-arm64)
Path: /Users/you/.omp/orchestrator/bin/0.1.0/batcave
```

```json
{ "version": "0.1.0", "target": "darwin-arm64", "path": "/Users/you/.omp/orchestrator/bin/0.1.0/batcave", "sizeBytes": 41211752 }
```

Failure (private-repo case):

```
Runtime install failed: failed to fetch release https://api.github.com/repos/nikolasd/batman/releases/tags/v0.1.0: HTTP 404
```

```json
{ "code": "http-error", "message": "failed to fetch release https://api.github.com/repos/nikolasd/batman/releases/tags/v0.1.0: HTTP 404" }
```

That `404` on a private repo almost always means no `GITHUB_TOKEN`/`GH_TOKEN` was set and no
`gh auth login` session exists — see the [code table above](#6-when-something-breaks).

## Appendix B — how the runtime binary is resolved

You don't need to know this to use the tools above, but it explains what `batman_status` reports and
what `OMP_BATMAN_BINARY` is for:

1. On first use in a session, the extension tries to connect to the repository's existing runtime
   socket. If one answers, it's reused — no process is spawned.
2. If nothing answers, it picks a binary in two tiers: `OMP_BATMAN_BINARY` (an absolute, executable
   path) wins outright if set — this is the local-development override, and it skips checksum/
   version validation entirely. Otherwise it looks for `<state root>/bin/<version>/batcave`, verifies
   its SHA-256 and version against a sibling `manifest.json` (and rejects a manifest whose `target`
   doesn't match this platform), and only trusts it once that check passes. That cache is populated
   by `/batman-runtime-install`, which downloads both files from this extension version's GitHub
   Release. The state root itself resolves as `BATMAN_STATE_DIR` (env var) →
   `$XDG_STATE_HOME/omp/batman` → `$HOME/${PI_CONFIG_DIR:-.omp}/orchestrator`.
3. It spawns `batcave serve` detached, with `BATMAN_BINARY_SOURCE` set to `override` or `package`
   accordingly (this is the "Binary source" field `batman_status` reports), then retries connecting
   with bounded exponential backoff. If a different concurrent caller won the daemon's single-
   instance lock in the meantime, this session simply connects to that winner.
