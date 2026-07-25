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

## 4. Worker adapters

Everything above this line predates the Worker Adapters milestone and never spawns a real Claude/
Codex/Copilot/OMP-RPC process. This section covers the four supervised adapters, their conformance
runner, and the worker coordination MCP surface.

### 4a. Prerequisites

Four vendor CLIs, plus everything from the top-level [Prerequisites](#prerequisites) above:

```bash
claude --version   # this milestone's baseline: Claude Code 2.1.217 (2.1.220 verified to work)
codex --version    # this milestone's baseline: codex-cli 0.145.0 (exact match required for the
                    # schema-compatibility check — see 4b)
copilot --version  # this milestone's baseline: GitHub Copilot CLI 1.0.73 (1.0.75 verified to work)
omp --version       # this milestone's baseline: omp/17.0.7 (17.1.1 verified to work)
```

None of these baselines are a hard requirement — `batcave adapters --json`/`batcave conformance`
*measure* what the installed CLI actually supports rather than trusting the version string; a
newer patch version that still passes every fixture scenario is fine. Codex is the one exception:
its adapter checks the installed binary's own generated JSON-RPC schema against a committed
compatibility manifest, so an incompatible **schema** change (not just a version bump) fails that
one check specifically, independent of everything else.

`OMP_BATMAN_BINARY` (the same override from the top-level Prerequisites) is how you point a real
`omp` session at your dev build rather than a packaged release — set it once per shell:

```bash
export OMP_BATMAN_BINARY="$PWD/target/debug/batcave"
```

Build the daemon (the extension isn't involved in 4b/4c below, only in 4e):

```bash
cargo build -p batman-runtime
```

### 4b. Per-adapter smoke, fixture mode (no model call, no vendor CLI required to *pass* — but
### each adapter's own PROBE scenario needs its CLI installed to report a real version)

```bash
./target/debug/batcave conformance --adapter claude  --fixture --output /tmp/batman-conformance-claude.json
./target/debug/batcave conformance --adapter codex   --fixture --output /tmp/batman-conformance-codex.json
./target/debug/batcave conformance --adapter copilot --fixture --output /tmp/batman-conformance-copilot.json
./target/debug/batcave conformance --adapter ompRpc  --fixture --output /tmp/batman-conformance-omprpc.json

# Or all four in one call, output as a single JSON array:
./target/debug/batcave conformance --adapter all --fixture --output /tmp/batman-conformance.json
```

Each command also prints its report to stdout. Expected shape (one array element per adapter for
the `--adapter all` form; a single-element array otherwise):

```json
[
  {
    "adapter": "claude",
    "mode": "fixture",
    "version": "2.1.220",
    "declaredCapabilities": { "protocol": "structured", "resume": "session", ... },
    "effectiveCapabilities": { "protocol": "structured", "resume": "session", ... },
    "scenarios": [
      { "name": "probe", "passed": true, "detail": "claude --version reported ...; authReady=true" },
      { "name": "read_only_start_and_progress", "passed": true, "detail": "..." },
      ...
    ],
    "passed": true
  }
]
```

"Pass" for one adapter means top-level `"passed": true` — every entry in `scenarios` has its own
`"passed": true`. `effectiveCapabilities` only ever narrows `declaredCapabilities`, never widens
it: a scenario failure downgrades exactly the capability it disproves (e.g. a failed `approval`
scenario forces `approvals` to `"none"`) and leaves everything else untouched. If `"passed": false`
anywhere, read that scenario's own `detail` first — it names concretely what failed, not just that
something did.

Every adapter's `--fixture` report should show `"passed": true` throughout, with these documented,
intentional exceptions — genuine gaps or environment dependencies this milestone reports honestly
rather than papering over with a fabricated pass:

| Adapter | Scenario(s) | Why |
|---|---|---|
| `ompRpc` | `probe`, `cancellation_scope`, `follow_up` | Depend on `omp models --json` currently listing a local `lm-studio`/`omlx` selector — the model server itself need not be *running*, just listed. If none is listed right now, expect these three `"passed": false` with a detail saying so. |
| `ompRpc` | `approval` | This adapter's `normalize_frame` has no case for the real vendor's `extension_ui_request` frame at all; `ApprovalsCapability::Observable` is declared but not yet actually backed by any observable event. |
| `codex` | `follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart` | The installed `codex-cli` does not write a thread's rollout file to disk until a turn actually runs — resuming/following up/cancelling a turn on a never-turned thread needs a real (billed) turn, which `--fixture` mode must never make. `--live` mode (4c) proves all four for real when its gate is set. |
| `copilot` | `session_resume`, `runtime_restart` | The installed CLI (1.0.75) does not persist a never-prompted session across a process boundary — proving full persistence needs a real turn. |
| `copilot` | `unexpected_child_observation` | ACP protocol v1 has no `session/update` variant this adapter maps to a nested-worker observation — a genuine, currently-unimplemented gap. |

`batcave adapters --json` runs the same fixture suite for all four adapters and always emits a
four-element array — it takes no `--adapter`/`--fixture`/`--output`, it *is* the "all adapters,
fixture mode, to stdout" shortcut:

```bash
./target/debug/batcave adapters --json
```

### 4c. Per-adapter smoke, live mode (requires a real API key/session; makes a real, billed model
### call for the adapters that reach one)

Each adapter's live suite is gated on its own environment variable, checked internally — the CLI
command is identical to 4b with `--live` instead of `--fixture`; nothing here ever needs a secret
*in* the command itself:

```bash
mkdir -p /tmp/batman-conformance-live && cd /tmp/batman-conformance-live && git init -q && git commit -q --allow-empty -m init

# Claude — needs an authenticated `claude` CLI session (run `claude auth status` first if unsure)
BATMAN_LIVE_CLAUDE=1 /path/to/target/debug/batcave conformance --adapter claude --live \
  --output /tmp/batman-conformance-live-claude.json

# Codex — needs $OPENAI_API_KEY (or an authenticated `codex` CLI session) in the environment
BATMAN_LIVE_CODEX=1 /path/to/target/debug/batcave conformance --adapter codex --live \
  --output /tmp/batman-conformance-live-codex.json

# Copilot — needs an authenticated `copilot` CLI session (`copilot` itself manages this, not an
# env var this adapter reads directly)
BATMAN_LIVE_COPILOT=1 /path/to/target/debug/batcave conformance --adapter copilot --live \
  --output /tmp/batman-conformance-live-copilot.json

# OMP-RPC — needs a local model server (LM Studio/oMLX) actually running; no cloud API key at all
BATMAN_LIVE_OMP=1 /path/to/target/debug/batcave conformance --adapter ompRpc --live \
  --output /tmp/batman-conformance-live-omprpc.json
```

Run each from inside `/tmp/batman-conformance-live` (a disposable repo — some live scenarios spawn
a real vendor process with that directory as its `cwd`), and reference credentials only as the
environment variable name, never the value, exactly as shown above.

If you run WITHOUT the matching `BATMAN_LIVE_<ADAPTER>=1` gate set, the command still exits `0` and
still writes a report — just one reporting the gate is unset, never a hard failure:

```json
[
  {
    "adapter": "claude",
    "mode": "live",
    "passed": false,
    "error": "live Claude conformance requires BATMAN_LIVE_CLAUDE=1"
  }
]
```

**What "no paid model call" means here, precisely:** every 4b (`--fixture`) command above is
*guaranteed* zero model calls — that is this milestone's own design invariant, proven by
`cargo test -p batman-runtime --test <adapter>_adapter` never invoking a model either. A 4c
(`--live`) command, once its gate is actually set, is the opposite: it deliberately makes a real,
billed call for whichever scenarios that adapter's own live suite defines as needing one (this
milestone's default posture is to prove as much as possible in fixture mode and reserve live mode
for the few properties — a real vendor process schema/handshake, mostly — that only a live process
can prove at all). Never set a `BATMAN_LIVE_<ADAPTER>` variable in a CI job or an unattended run.

### 4d. End-to-end orchestration with an adapter: still `adapter_unavailable`, and that's expected

`AdapterRegistry` (the `RunDriver` implementation this section's conformance suites feed into) is
built and independently tested (`cargo test -p batman-runtime --test adapter_registry`), but it is
**not yet wired into the running daemon** — `lifecycle::serve()`'s `ServerConfig::default()` still
leaves `run_driver: None`. This is a deliberate, documented scope boundary (see
`crates/runtime/src/adapter/registry.rs`'s own module doc for the two reasons: `run/submit` carries
no prompt/message content for a started adapter to act on yet, and adapters constructed by the
registry today receive no worker-coordination MCP config), not an oversight.

Practically: **section 3 above, unchanged, is still the correct end-to-end walkthrough.** Submitting
a run through a live `omp` session still reports `adapter_unavailable`, exactly as documented there
— that has not changed and will not change until a future milestone wires `AdapterRegistry` into
`ServerConfig.run_driver` inside `lifecycle::serve()`. To exercise the registry's own start/reject/
authorize/construct logic directly (the actual new behavior this milestone adds), use its test
suite rather than a live `omp` session:

```bash
cargo test -p batman-runtime --test adapter_registry
```

### 4e. Worker MCP coordination tools

Like 4d, the *supervised* path (a real adapter's vendor process calling `batman_task`/`batman_send`
through its injected MCP config) is not reachable from a live `omp` session yet, for the same
reason: no adapter is wired into the running daemon to supervise a vendor process in the first
place. The MCP server side (`batcave coordination-mcp`) and the scope-token-authenticated
in-process/subprocess plumbing behind it are fully built and independently tested against a real
compiled `batcave` binary, driven as a genuine MCP client would:

```bash
cargo test -p batman-runtime --test coordination_mcp
```

That suite spawns the real `batcave coordination-mcp --state-dir ... --repo ... --run-id ...`
subprocess, drives it over real stdio exactly as a supervised vendor CLI's own MCP client would,
and verifies `batman_task`/`batman_peers`/`batman_send`/`batman_request_child`/
`batman_publish_artifact`/`batman_report_blocked`/`batman_ask_policy` all land in a real
`CoordinationBroker` behind a real `Server` — including the scope/authorization negative cases
(missing, expired, wrong-run, post-vendor-exit, or unrelated-process credentials all fail; a
verified descendant of the same live vendor process may reconnect).

### 4f. Cleanup

```bash
pgrep -fl batcave                                            # confirm what's actually running
./target/debug/batcave stop --repo /tmp/batman-conformance-live  # or wherever you ran 4c from
rm -rf /tmp/batman-conformance-live /tmp/batman-conformance*.json
pgrep -fl batcave || echo "no batcave processes remain"
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
