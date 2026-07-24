# Manual testing

Every automated suite (`bun run check`) runs without a model call and without a human watching a
screen. Some things can only be verified by actually running `omp`, calling a tool with a real
model, and looking at what comes back — this document is the complete, current list of those
checks: what to run, in what order, exactly what you should see, and what it means if you don't.

Run these after any change that touches the daemon lifecycle, the IPC layer, an orchestration RPC
method, an OMP tool, or the monitor — the automated suites can tell you a function returns the
right value; only these checks can tell you the *whole system*, wired together, still behaves the
way `architecture.md` says it does.

## Prerequisites

Same as [getting-started.md](getting-started.md#prerequisites): Rust 1.97.1+, Bun 1.3.14+,
`omp` ≥ 17.0.7 on your `PATH`. Build both sides first:

```bash
cargo build -p batman-runtime
bun run --cwd packages/extension build
```

## 1. The daemon by hand (no extension, no OMP)

The lowest layer, useful when you suspect the problem is in the daemon itself rather than
anything above it:

```bash
BC=./target/debug/batcave

# Foreground (structured JSON logs on stderr, Ctrl-C to stop)
$BC serve --foreground --state-dir /tmp/batman-state --repo "$PWD" --idle-seconds 60

# In another terminal:
$BC status --wait-seconds 5 --state-dir /tmp/batman-state --repo "$PWD"   # pretty JSON snapshot
$BC stop --state-dir /tmp/batman-state --repo "$PWD"                      # graceful shutdown
$BC version                                                               # batcave 0.1.0
$BC schema                                                                # the embedded JSON Schema
```

Behavior worth knowing:

- Omitting `--state-dir` resolves the real state root (`getting-started.md`'s environment
  variables table).
- Omitting `--idle-seconds` runs until signalled; with it, the daemon exits after that many
  seconds with no clients connected.
- A second `serve` against the same repo exits with code **73** and prints one line of
  `already_running` JSON on stdout — that's the single-instance lock working, not a bug.
- Detached daemons (what `ensureRuntime` spawns) log to `runtime.log` in the state directory
  instead of stderr.

## 2. `batman_status` through OMP (no model call)

The first layer that involves the extension. `/batman-status` is a slash command, not a tool, so
this completes with no model call at all — if this doesn't work, nothing above it will either.

```bash
export OMP_BATMAN_BINARY="$PWD/target/debug/batcave"
EXT="$PWD/packages/extension/dist/index.js"

omp --extension "$EXT" --print "/batman-status"
```

Expect:

```
BATMAN runtime: running
Protocol: 1.0 (healthy: true)
Project: 18f82a46-....
Active runs: 0
Schema version: 1
Uptime: 0s
Binary source: override
```

Run it again — same command, same repo:

```bash
omp --extension "$EXT" --print "/batman-status"
```

Expect the **same** `Project` id, with a **higher** `Uptime`. That's the connect-or-spawn design
(ADR-0008) reconnecting to the daemon it just started, not spawning a second one.

## 3. The orchestration tools (needs a real model call)

Unlike step 2, the six orchestration tools (`batman_task`, `batman_worker`, `batman_run`,
`batman_message`, `batman_approval`, `batman_reconcile`) are regular OMP tools the model *chooses*
to call — this genuinely needs a model, and each step below takes something like ten seconds to a
couple of minutes. Work in a scratch repository, never this one:

```bash
mkdir -p /tmp/batman-smoke && cd /tmp/batman-smoke && git init -q && git commit -q --allow-empty -m init
```

### 3a. Create a task, a worker, and submit a run

```bash
omp --extension "$EXT" --print \
  'Use batman_task to upsert a task with ownerClientInstanceId "smoke" and revision 1. Then use
   batman_worker to create a worker with fingerprint "sha256:smoke" and adapter "fake". Then use
   batman_run to submit a run for that task against that worker. Report the taskId and workerId
   plainly.'
```

Expect the model to report a `taskId` and `workerId`, and to say `run/submit` failed with
`adapter_unavailable` — and, importantly, that it **can't** report a `runId` from that call. That
last part is correct, not a bug in the model: `run/submit`'s error response is
`ServiceError { code, message }`, with no `data` field at all, so the caller genuinely has no way
to learn the run's id from that one call alone.

The run was still committed as `queued` underneath. Look it up with a second call, using the
`taskId` from the response above:

```bash
omp --extension "$EXT" --print \
  'Use batman_run with op "list" and taskId "<taskId from above>" to find the run that was just
   submitted. Report the runId and state plainly.'
```

Expect `state: queued`. The run is preserved even though nothing could start it — `run/submit`
never pretends a run started that it can't back, and it never drops the run just because no
adapter exists to run it (ADR-0013).

### 3b. Watch it live — two processes, on purpose

Open an **interactive** session and leave it running. This is a different invocation from the
`--print` calls above, and it matters that it stays open for the rest of this step:

```bash
omp --extension "$EXT"          # no --print: stays open, interactive
```

Type `/batman`. Expect one line, replayed from the daemon's journal the instant this session
started — it never touched the task/worker/run above, this is a brand-new session:

```
<runId-prefix> · queued · run queued
```

Now, **without closing that session**, open a *second* terminal and run the message-send call
there — a separate, short-lived process that connects to the same daemon and exits on its own:

```bash
omp --extension "$EXT" --print \
  'Use batman_message to send a "question" on runId "<runId from 3a>" from workerId
   "<workerId from 3a>", taskId "<taskId from 3a>", payload "should I proceed?".'
```

Go back to the **first** terminal — the one you never touched during that second call — and look
at it again. Expect it to have updated on its own, with zero input from you:

```
<runId-prefix> · queued · messageRecorded recorded
```

That's the live-broadcast path: the first session was already subscribed to the daemon's event
stream, and the message-send (from a *different* process) got pushed to it over the socket it
already had open — no reconnect, no re-typed `/batman`, no polling.

Only the trailing "latest activity" field changes here; the run's own `state` stays `queued`
throughout, because nothing in this scenario ever starts a real adapter. A `starting`/`working`
transition needs `FakeRunDriver` or a real adapter, neither of which is reachable from a live
`omp` session — only from `cargo test -p batman-runtime --test orchestration_rpc`.

### 3c. Replay after a full restart

Close the first session entirely (`Ctrl+C` or `/exit`) and start a **third**, completely fresh
one:

```bash
omp --extension "$EXT"
```

Type `/batman` again. Expect the *same* final line, replayed cold by a session that has never
seen any of this before:

```
<runId-prefix> · queued · messageRecorded recorded
```

Nothing is lost, nothing duplicates. This is a genuinely different property from 3b's
live-broadcast test — 3b required the watching session to *stay open the whole time*; this one
requires it to be fully torn down and restarted. Both must hold; neither one proves the other.

### 3d. What this walkthrough can't cover

Approval creation (`ApprovalService::request`) is only ever invoked by an adapter reporting it
needs human sign-off, and there is no `approval/request` RPC method — adapters are out of scope
this milestone. There is no way to trigger it from a live `omp` session. Exercise that half of the
flow with:

```bash
cargo test -p batman-runtime --test approval
```

which drives `ApprovalService` directly, the same way this walkthrough can't.

### Clean up

```bash
./target/debug/batcave stop --repo /tmp/batman-smoke   # or wait out the idle interval
rm -rf /tmp/batman-smoke
```

## Reading the widget line

Every `/batman` row (`monitor/render.ts::renderRowLine`) is:

```
<first 8 chars of runId> · <state> · [harness] · [flags] · [pending approvals] · [workspace mode] · <latest activity>
```

— joined by ` · `, with any part that's undefined simply omitted. In this walkthrough there's no
real adapter, so you'll only ever see the run id, `state` (always `queued` here), and
`latestActivity`, which is set per event kind (`monitor/model.ts`):

| Event | `latestActivity` |
|---|---|
| `RunEvent` | `"run " + state` (e.g. `"run queued"`, `"run starting"`, `"run working"`) |
| `MessageEvent` | `"${kind} ${deliveryState}"` (e.g. `"messageRecorded recorded"`) |
| `ApprovalEvent` | `"approval requested: <action>"` or `"approval decided"` |
| `ChildEvent` | `"child worker requested"` or `"child worker request denied"` |

## If something doesn't match

See `getting-started.md`'s [Troubleshooting](getting-started.md#troubleshooting) table first —
most manual-test surprises (`METHOD_NOT_FOUND`, an empty `/batman`, connect timeouts) are covered
there with the exact cause. If a step in this document produces something not described here or
there, that's either a real regression or a gap in this document — both are worth fixing; open an
issue or extend this file, the same way the `run/submit` error-shape gap above was found by
running the walkthrough for real and getting confused by it.
