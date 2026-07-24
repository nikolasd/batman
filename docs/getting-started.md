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
cargo test -p batman-protocol                  # wire contract + fixtures
cargo test -p batman-runtime --test paths      # state paths, repo identity, permissions
cargo test -p batman-runtime --test database   # SQLite actor, replay, intent ledger
cargo test -p batman-runtime --test redaction_boundary   # secret/thinking bytes never durable
cargo test -p batman-runtime --test ipc        # socket protocol, negotiation, roles, auth
cargo test -p batman-runtime --test lifecycle  # flock singleton, idle shutdown, graceful stop
cargo test -p batman-xtask                     # packaging determinism

# TypeScript, per file
bun test packages/extension/src/client.test.ts    # spawns the real batcave — build it first
bun test packages/extension/src/runtime.test.ts   # ensureRuntime, detach, binary override
bun test packages/extension/src/index.test.ts     # OMP tool/command registration + live daemon
bun test packages/extension/src/platform.test.ts  # tuple mapping, integrity, override precedence
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

1. Add the variant to `BatmanMethod` (`crates/protocol/src/rpc.rs`) with its wire name.
2. Add it to the appropriate role table(s) in `ClientPrincipal::allowed_methods`
   (`crates/runtime/src/ipc/mod.rs`) — methods not in a caller's table are invisible
   (`METHOD_NOT_FOUND`).
3. Implement dispatch in `crates/runtime/src/ipc/connection.rs`, add params/result wire types, and
   regenerate (previous workflow).
4. Add integration coverage in `crates/runtime/tests/ipc.rs` and, if the extension calls it,
   validation + tests on the TypeScript side.

### Anything that must be persisted

Route it through the redaction boundary. Events go `RawRuntimeEvent → Redactor::sanitize →
PersistableEvent → DatabaseHandle::append_event`; operation payloads go
`serde_json::Value → Redactor::sanitize_json → SanitizedJson`. If you find yourself wanting a
raw-string append API, stop — that's the boundary you'd be deleting.

## Troubleshooting

| Symptom | Likely cause / what to do |
|---|---|
| `serve` exits 73 immediately | Another daemon owns this repo's lock. `batcave status --repo …` to see it; `batcave stop --repo …` to stop it. |
| `bun run generate --check` fails | Committed generated files drifted from the Rust types. Run `bun run generate` and commit the result. |
| TS tests fail with connect timeouts | You forgot `cargo build -p batman-runtime`, or a stale daemon from an aborted run is holding a temp socket — `pgrep -fl batcave` and kill leftovers. |
| `batcave` refuses to start: socket path too long | Unix socket paths are limited (~104 bytes on macOS). Use a shorter `--state-dir`. |
| Status tool returns `isError` with a code | Run the `doctorCommand` it hands back (`batcave status --repo <repo>`); codes like `checksum-mismatch`/`unsupported-platform` point at binary resolution, `connection-failed` at the daemon. |
| Nothing in `runtime.log` | Foreground daemons log to stderr instead; the log file is only written by detached daemons. |

For deeper debugging techniques (inspecting the SQLite journal, tracing a request), see the
[code walkthrough](code-walkthrough.md).
