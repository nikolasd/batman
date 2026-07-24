# Getting started

Everything you need to build, test, run, and iterate on BATMAN locally.

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.97.1+ | `rust-toolchain.toml` pins 1.97.1 for rustup users; a matching system toolchain (e.g. Homebrew) works too |
| Bun | 1.3.14+ | `package.json` declares `"packageManager": "bun@1.3.14"` |
| OMP | ≥ 17.0.7 < 18 | only needed for the OMP integration smoke; `@oh-my-pi/pi-coding-agent` is a dev/peer dependency |
| OS | macOS or glibc Linux, arm64/x64 | anything else is rejected by design |

## First build

```bash
git clone <repo-url> batman && cd batman
bun install                      # links the Bun workspaces, installs extension deps
cargo build -p batman-runtime    # produces target/debug/batcave
bun run check                    # the full gate — see below
```

`bun run check` is the everything-gate and what CI runs. It expands to:

1. `cargo run -p batman-xtask -- generate --check` — fails if committed schema/TS bindings drift
   from the Rust types,
2. `bun run build` — bundles the extension into `packages/extension/dist/`,
3. `bun test packages` — all TypeScript suites,
4. `cargo test --workspace` — all Rust suites.

If it's green, you're in a good state.

## Running the test suites

```bash
# Everything
bun run test                                   # bun test packages && cargo test --workspace

# Rust, per area (integration test targets live in crates/runtime/tests/)
cargo test -p batman-protocol                  # wire contract + fixtures + domain_contract + coordination_contract
cargo test -p batman-runtime --test paths      # state paths, repo identity, permissions
cargo test -p batman-runtime --test database   # SQLite actor, replay, intent ledger
cargo test -p batman-runtime --test redaction_boundary   # secret/thinking bytes never durable
cargo test -p batman-runtime --test ipc        # socket protocol, negotiation, roles, auth
cargo test -p batman-runtime --test lifecycle  # flock singleton, idle shutdown, graceful stop
cargo test -p batman-runtime --test domain_repository  # projection transactions, rollback, event rebuild
cargo test -p batman-runtime --test orchestration_rpc  # task/worker/run/message/approval/reconcile RPC
cargo test -p batman-runtime --test coordination       # worker-safe messaging, scope tokens, rate limits
cargo test -p batman-runtime --test approval            # correlated approval ownership/idempotency/recovery
cargo test -p batman-xtask                     # packaging determinism

# TypeScript, per file
bun test packages/extension/src/client.test.ts    # spawns the real batcave — build it first
bun test packages/extension/src/runtime.test.ts   # ensureRuntime, detach, binary override
bun test packages/extension/src/index.test.ts     # OMP tool/command registration + live daemon
bun test packages/extension/src/platform.test.ts  # tuple mapping, integrity, override precedence
bun test packages/extension/src/tools            # batman_task/worker/run/message/approval/reconcile
bun test packages/extension/src/omp-native       # task:subagent:* mapping, coalescing, restart/lost
bun test packages/extension/src/monitor          # event-reducer, rendering, replay-then-live
bun test packages/protocol-ts/src/schema.test.ts  # generated schema sanity

# Lint/format gates
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

Several TypeScript suites spawn the real daemon: run `cargo build -p batman-runtime` before them.
The lifecycle and client tests use real processes, temp state dirs, and 1-second idle timers, so
the whole suite stays fast (~seconds, not minutes).

## Running the daemon by hand

```bash
cargo build -p batman-runtime
BC=./target/debug/batcave

# Foreground (structured JSON logs on stderr, Ctrl-C to stop)
$BC serve --foreground --state-dir /tmp/batman-state --repo "$PWD" --idle-seconds 60

# In another terminal:
$BC status --wait-seconds 5 --state-dir /tmp/batman-state --repo "$PWD"   # pretty JSON snapshot
$BC stop --state-dir /tmp/batman-state --repo "$PWD"                      # graceful shutdown
$BC version                                                               # batcave 0.1.0
$BC schema                                                                # the embedded JSON Schema
```

Behaviour worth knowing:

- Omitting `--state-dir` resolves the real state root (see environment variables below).
- Omitting `--idle-seconds` runs until signalled; with it, the daemon exits after that many
  seconds with no clients connected.
- A second `serve` against the same repo exits with code **73** and prints one line of
  `already_running` JSON on stdout — that's the single-instance lock working, not a bug.
- Detached daemons (what `ensureRuntime` spawns) log to `runtime.log` in the state directory
  instead of stderr.

## Running inside OMP

```bash
cargo build -p batman-runtime
OMP_BATMAN_BINARY="$PWD/target/debug/batcave" \
  omp --extension ./packages/extension/src/index.ts --print "/batman-status"
```

Expected output ends with something like:

```
BATMAN runtime: running
Protocol: 1.0 (healthy: true)
Project: 18f82a46-....
Active runs: 0
Schema version: 1
Uptime: 0s
Binary source: override
```

No model call happens — the slash command completes locally. Run it twice: the second run reports
the **same project id** with a higher uptime, proving it reconnected to the existing daemon.
Afterwards, `./target/debug/batcave stop --repo "$PWD"` shuts the detached daemon down (or just
wait out the idle interval).

## Smoke-testing the orchestration extension

Unlike `/batman-status`, the six orchestration tools (`batman_task`, `batman_worker`,
`batman_run`, `batman_message`, `batman_approval`, `batman_reconcile`) are regular OMP tools the
model chooses to call — this needs a real model call and takes a minute or two per step, so it's
a manual smoke test, not something CI runs. This is the same scenario the orchestration
extension's completion check documents.

```bash
cargo build -p batman-runtime
bun run --cwd packages/extension build            # produces packages/extension/dist/index.js
mkdir -p /tmp/batman-smoke && cd /tmp/batman-smoke && git init -q && git commit -q --allow-empty -m init

export OMP_BATMAN_BINARY="$OLDPWD/target/debug/batcave"
EXT="$OLDPWD/packages/extension/dist/index.js"

omp --extension "$EXT" --print \
  'Use batman_task to upsert a task with ownerClientInstanceId "smoke" and revision 1. Then use
   batman_worker to create a worker with fingerprint "sha256:smoke" and adapter "fake". Then use
   batman_run to submit a run for that task against that worker. Report the taskId, workerId, and
   runId plainly.'
```

Expected: `run/submit` reports `adapter_unavailable` (no adapter is wired in this milestone) but
the run is preserved `queued` — it never pretends a run started that it can't back. Open
`/batman` in an interactive session (`omp --extension "$EXT"`, no `--print`) to watch it live:

```
<runId-prefix> · queued · run queued
```

Send a message and confirm the widget updates without reconnecting:

```bash
omp --extension "$EXT" --print \
  'Use batman_message to send a "question" from your worker on that run with payload
   "should I proceed?".'
```

`/batman` now reads `... · queued · messageRecorded recorded`. Restarting `omp` against the same
repository (a fresh daemon, since the detached daemon exits after its idle timeout) replays the
identical state from the durable journal — nothing is lost, nothing duplicates.

Approval creation (`ApprovalService::request`) is only ever invoked by an adapter reporting it
needs human sign-off, and there is no `approval/request` RPC method — adapters are out of scope
this milestone. Exercise that half of the flow with `cargo test -p batman-runtime --test
approval` instead; it drives `ApprovalService` directly, the same way the smoke scenario can't.

Clean up: `./target/debug/batcave stop --repo /tmp/batman-smoke` (or wait out the idle interval),
then `rm -rf /tmp/batman-smoke`.

## Environment variables

| Variable | Effect |
|---|---|
| `BATMAN_STATE_DIR` | Overrides the state root. Must be absolute. Highest precedence. |
| `XDG_STATE_HOME` | If set (absolute), the state root becomes `$XDG_STATE_HOME/omp/batman`. |
| `PI_CONFIG_DIR` | Changes the default's middle segment: `$HOME/${PI_CONFIG_DIR:-.omp}/orchestrator`. |
| `OMP_BATMAN_BINARY` | Development override for the daemon binary. Must be an absolute path to an existing, regular, executable file; each violation fails before spawn with a typed error. Bypasses the packaged-binary integrity checks and reports `binarySource: "override"`. |
| `BATMAN_BINARY_SOURCE` | Set by the launcher (`override`/`package`) so the daemon can report its own origin in `runtime/status`; you rarely set this by hand. |

## Common workflows

### Changing or adding a wire type

1. Edit the Rust type in `crates/protocol/src/` (add derives + serde attributes matching the
   neighbours; every wire struct is `camelCase` + `deny_unknown_fields`).
2. If it's a new top-level request/result/event type, add it to `ProtocolDocument` and the export
   list in `crates/xtask/src/main.rs`, and re-export it from `crates/protocol/src/lib.rs`.
3. `bun run generate` — regenerates the schema and TS bindings. Commit the regenerated files.
4. `bun run check` — proves nothing drifted and all suites still pass.

Never edit anything under `packages/protocol-ts/src/generated/` or the schema JSON by hand.

### Adding a JSON-RPC method

1. Add the variant to `BatmanMethod` (`crates/protocol/src/method.rs`) with its wire name.
2. Add it to the appropriate role table(s) in `ClientPrincipal::allowed_methods`
   (`crates/runtime/src/ipc/mod.rs`) — methods not in a caller's table are invisible
   (`METHOD_NOT_FOUND`).
3. Implement dispatch in `crates/runtime/src/ipc/connection.rs`, add params/result wire types, and
   regenerate (previous workflow).
4. Add integration coverage in `crates/runtime/tests/ipc.rs` (foundation methods) or
   `crates/runtime/tests/orchestration_rpc.rs`/`coordination.rs`/`approval.rs` (orchestration
   methods), and, if the extension calls it, validation + tests on the TypeScript side.

### Anything that must be persisted

Route it through the redaction boundary. Events go `RawRuntimeEvent → Redactor::sanitize →
PersistableEvent → DatabaseHandle::append_event`; operation payloads go
`serde_json::Value → Redactor::sanitize_json → SanitizedJson`. If you find yourself wanting a
raw-string append API, stop — that's the boundary you'd be deleting.

### Adding a new domain mutation (task/worker/run/message/approval)

Every `DomainRepository` mutation (`crates/runtime/src/domain/repository.rs`) must reach live
`events/subscribe` listeners — including the embedded monitor — the moment it commits, or you
will reintroduce the exact bug documented in `docs/architecture.md` §18 (item 3). At your
service-layer call site (`service/orchestration.rs`, `approval/service.rs`, or
`coordination/broker.rs`):

1. Inside the `run_domain_op` closure, wrap the repository call's success value with
   `domain::embed_envelope(json!({ ... }), &committed.envelope)` instead of returning the JSON
   bare.
2. After `.await`, call `self.broadcast(&mut result)` (or the free-standing equivalent if you're
   not inside one of those three services) **before** using `result` to build the RPC response —
   `broadcast`/`take_envelope` mutates `result` in place, stripping the internal `__envelope` key.
3. If your service doesn't yet hold an `events_tx: broadcast::Sender<EventEnvelope>` field, add
   one and thread it through from wherever `crates/runtime/src/ipc/server.rs::bind` constructs
   your service (it already holds the one true `events_tx`).
4. Prove it: add a case to `events_subscribe_delivers_live_notifications_for_orchestration_
   mutations` in `crates/runtime/tests/orchestration_rpc.rs`, or write a sibling test following
   its exact shape (subscribe, mutate on a second connection, assert the notification arrives).
   A missed broadcast doesn't fail loudly — the test either fails a value assertion or, if you
   forget the broadcast entirely, **hangs forever** waiting on a notification that never comes;
   run new tests in this family with an explicit timeout the first few times.

## Troubleshooting

| Symptom | Likely cause / what to do |
|---|---|
| `serve` exits 73 immediately | Another daemon owns this repo's lock. `batcave status --repo …` to see it; `batcave stop --repo …` to stop it. |
| `bun run generate --check` fails | Committed generated files drifted from the Rust types. Run `bun run generate` and commit the result. |
| TS tests fail with connect timeouts | You forgot `cargo build -p batman-runtime`, or a stale daemon from an aborted run is holding a temp socket — `pgrep -fl batcave` and kill leftovers. |
| `batcave` refuses to start: socket path too long | Unix socket paths are limited (~104 bytes on macOS). Use a shorter `--state-dir`. |
| Status tool returns `isError` with a code | Run the `doctorCommand` it hands back (`batcave status --repo <repo>`); codes like `checksum-mismatch`/`unsupported-platform` point at binary resolution, `connection-failed` at the daemon. |
| Nothing in `runtime.log` | Foreground daemons log to stderr instead; the log file is only written by detached daemons. |
| An orchestration tool fails `method "..." is not available to this client` | The client authenticated with too narrow a role for what it's calling. Check `ClientPrincipal::allowed_methods` (`crates/runtime/src/ipc/mod.rs`) for the role your `ensureRuntime`/test client actually used — this is exactly the bug documented in `docs/architecture.md` §18 (item 2). |
| `/batman` shows nothing even though a mutation succeeded | Either `events/replay` is failing to deserialize (check the daemon's response to a raw `events/replay` call) or the mutation never broadcast — see the "Adding a new domain mutation" workflow above and `docs/architecture.md` §18 (item 3). |

For deeper debugging techniques (inspecting the SQLite journal, tracing a request), see the
[code walkthrough](code-walkthrough.md).
