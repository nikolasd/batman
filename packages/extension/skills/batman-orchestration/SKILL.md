---
name: batman-orchestration
description: >-
  Use when the user wants to run code with an external AI worker on its own adapter/model
  (Claude, Codex, Copilot, or an OMP-RPC worker) — this is the mechanism for a specific
  vendor or model, not the in-process `task` subagent tool. Also covers hand work off to
  an agent, run tasks in parallel, check on a run, retry or cancel a task.
  Fires on "run this with Claude", "spawn a Claude", "spawn a Codex", "spawn a Copilot",
  "spawn a worker", "run with Sonnet/Opus/Haiku/GPT", "use model X", "delegate this to Claude/Codex/Copilot",
  "hand this off", "run these in parallel", "retry it", "check on that run".
---

## How BATMAN tools work

Two facts are invisible from the tool schemas but essential for correct use:

- **BATMAN stores no task text of its own.** The `prompt` argument must be supplied on every `run/submit` **and every `run/retry`**. Retry does not remember the prior prompt — you must pass it again.
- **Every BATMAN tool returns the daemon's JSON result verbatim under `details`.** Read ids (`taskId`, `workerId`, `runId`, `leaseId`, etc.) from there. Never invent or guess them.

## The canonical call sequence

To run a task on an AI worker, follow this chain — each step reads an id from the previous response's `details`:

1. **Find or create a worker.** Call `batman_worker { op: "list" }` and reuse a `workers[].workerId` whose `profileRef.adapter` and `profileRef.model` match what the user asked for. If none matches, create one with `batman_worker { op: "create", fingerprint, adapter, model }` and read the new `workerId` from the response.

2. **Create a task.** Call `batman_task { op: "upsert", description }` and read the `taskId` from the response. This is a persistent unit of work stored in the SQLite journal.

3. **Submit the run.** Call `batman_run { op: "submit", taskId, workerId, prompt }` and read the `runId` from the response. The `prompt` is the full instruction text the worker will execute — pass it exactly as the user stated (or as you refined it).

## Workspace modes

When the user specifies where the work should happen, translate their words into `workspaceMode` on `run/submit`:

- **"in its own worktree"** or **"don't touch my files"** → `workspaceMode: "isolated"` (a per-run git worktree)
- **"on a copy"** → `workspaceMode: "copy"` (a per-run copy of the repository)
- **default or unstated** → `workspaceMode: "shared"` (the repository itself)

Any other value is rejected by the runtime.

## Monitoring runs

- **Preferred approach:** Tell the user to open `/batman` — the live monitor shows all runs, their state, flags, and output in real time.
- **Programmatic polling:** Call `batman_run { op: "get", runId }` and report `state` plus any `true` entries in `flags` (like `degradedControl`, `needsReconciliation`, `policyQuarantined`, `workspaceDirty`, `childrenActive`).

## Parallel work

To run multiple tasks concurrently on separate workers:

- Acquire one `batman_workspace { op: "acquire", runId, mode: "write", requestedIsolation: "gitWorktree" }` lease **per worker** before submitting its run.
- Isolated leases (gitWorktree or copy) never conflict with each other — each gets its own directory. A shared-mode write lease is exclusive project-wide, so only one can exist at a time.
- Release each lease with `batman_workspace { op: "release", leaseId }` once its run finishes.

## Recovery

- **Retry:** `batman_run { op: "retry", priorRunId, workerId, prompt }` — requires both the `priorRunId` (from the failed run) **and the `prompt` again** (prompts are never remembered). Always returns a **new** `runId`.
- **Cancel:** `batman_run { op: "cancel", runId }` stops a stuck or running run.

## Trap: failed submits

`run/submit`'s error response carries no `runId`. After a failed submit, find the run with `batman_run { op: "list", taskId }` rather than assuming an id came back.

## Boundary

This skill covers task execution, monitoring, and recovery. It never decides approvals or resolves policy violations — that is `batman-approvals`.
