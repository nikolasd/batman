# Code walkthrough

A guided tour for navigating, tracing, debugging, and testing the codebase. Read
[architecture.md](architecture.md) first for the *why*; this document is the *where and how*.

New to Rust? Read the [Rust primer](rust-primer.md) alongside this — it teaches Rust using this
repository's own code.

## 1. Map of the source

### `crates/protocol` — the wire contract (start here)

Small, dependency-light, and the vocabulary for everything else.

| File | What lives there |
|---|---|
| `src/lib.rs` | Re-exports everything; the crate's public API is this one page |
| `src/ids.rs` | `uuid_id!` macro generating the 8 id newtypes (`ProjectId`, `RunId`, …) |
| `src/version.rs` | `ProtocolVersion`, `VersionRange` |
| `src/rpc.rs` | JSON-RPC envelopes, `InitializeParams/Result`, `ClientAuth` roles, `RuntimeStatus`, `error_code` constants (`BatmanMethod` itself now lives in `method.rs`, re-exported here) |
| `src/method.rs` | `BatmanMethod` — every JSON-RPC method name, foundation and orchestration alike |
| `src/event.rs` | `EventEnvelope`, `RuntimeEvent`, `Timestamp`, `ContentClass`/`Classified<T>` |
| `src/task.rs` | `TaskRef` |
| `src/worker.rs` | `WorkerProfileRef`, `Worker` |
| `src/run.rs` | `Run`, `RunSpec`, `RunState` (+ `can_transition_to`/`is_terminal`), `RunFlags` |
| `src/message.rs` | `RunMessage`, `MessageKind`, `DeliveryState` |
| `src/approval.rs` | `ApprovalRequest` |
| `src/coordination.rs` | worker-safe request/result types, `COORDINATION_PAYLOAD_MAX_BYTES`, `COORDINATION_RATE_LIMIT_PER_MINUTE` |
| `tests/wire_contract.rs` | Proves camelCase + `deny_unknown_fields` on the wire |
| `tests/domain_contract.rs` | `RunState` lifecycle table, `RunFlags` field names, `BatmanMethod` orchestration variants |
| `tests/coordination_contract.rs` | Message kinds, delivery states, coordination request/result wire shapes |
| `tests/fixtures.rs` | Deserializes the golden fixtures through the real types |

### `crates/runtime` — the `batcave` daemon

| File | What lives there |
|---|---|
| `src/main.rs` | Thin entry point; calls `cli::run()` |
| `src/cli.rs` | clap definitions for `serve`/`status`/`stop`/`version`/`schema`; maps outcomes to exit codes (73 = lost the singleton race) |
| `src/lifecycle.rs` | `serve()`/`status()`/`stop()`: flock singleton, lock metadata, idle shutdown, graceful-stop ordering, log routing |
| `src/paths.rs` | `RuntimePaths::resolve`, VCS-root discovery, `repository_id_from_canonical_root` |
| `src/security/mod.rs` | `StateRoot::resolve` precedence, `ensure_private_dir`/`ensure_private_file` (0700/0600, atomic) |
| `src/security/redaction.rs` | `Redactor`, `RawRuntimeEvent`, `PersistableEvent`, `SanitizedJson` — the redaction boundary |
| `src/db/actor.rs` | `DatabaseHandle` + the actor thread owning the SQLite connection |
| `src/db/migrations.rs` | PRAGMAs, migration 1 (`events`, `operations`), migration 2 (`worker_profiles`, `tasks`, `workers`, `runs`, `messages`, `approvals`) |
| `src/db/models.rs` | Row types (`ReplayedEvent`, `OperationIntent`, `Diagnostics`) |
| `src/ipc/mod.rs` | `ServerConfig`, `ClientPrincipal` + role method tables, `PeerCredentials` reader trait, `WorkerCredentialVerifier` trait, `IpcError` |
| `src/ipc/server.rs` | Socket bind (owner-only, SUN_LEN guard), accept loop, UID admission, idle bookkeeping, constructs `OrchestrationService`/`CoordinationBroker` and the one `events_tx` broadcast channel they share |
| `src/ipc/connection.rs` | Per-connection reader/writer split, initialize handshake, method dispatch (routes orchestration methods to `OrchestrationService`, `coordination/*` to `CoordinationBroker`), replay/subscribe |
| `src/domain/repository.rs` | `DomainRepository` — every projection-mutating command; `append_and_apply` (event + projection, one transaction); `Committed`, `embed_envelope`/`take_envelope` |
| `src/domain/transitions.rs` | `check_transition`, `TransitionError::Illegal` — the canonical `RunState` lifecycle relation |
| `src/service/orchestration.rs` | `OrchestrationService` — routes every Task/Worker/Run/Message/Approval/Reconcile method to `DomainRepository` or `service/query.rs` |
| `src/service/query.rs` | Read-only lookup closures (`task_get_op`, etc.) run through `DatabaseHandle::run_domain_op` |
| `src/service/run_driver.rs` | `RunDriver` trait, `RunDriverContext`, `FakeRunDriver` (`queued -> starting -> working`) |
| `src/coordination/broker.rs` | `CoordinationBroker` — record-before-delivery messaging, `sweep_unacknowledged_as_unknown` |
| `src/coordination/scope_token.rs` | `ScopeTokenStore` (mint/verify), `PidAncestryChecker` |
| `src/coordination/rate_limit.rs` | `RateLimiter` — 30 messages/minute/sender sliding window |
| `src/approval/service.rs` | `ApprovalService` — `request`/`decide`, ownership/idempotency/settled-run enforcement, `ApprovalCallback` seam |

Integration tests in `crates/runtime/tests/` are the daemon's behavioural spec — one file per
subsystem (`paths`, `database`, `redaction_boundary`, `ipc`, `lifecycle`, `domain_repository`,
`orchestration_rpc`, `coordination`, `approval`). The lifecycle tests run the real compiled binary
(`env!("CARGO_BIN_EXE_batcave")`) as real processes.

### `crates/xtask` — build tooling

One file (`src/main.rs`): `generate [--check]` (schema + ts-rs bindings, deterministic,
temp-dir byte-compare in check mode) and `package --target <triple> --binary <path>` (installs a
binary into a leaf package with a deterministic manifest).

### `packages/extension` — the OMP extension (`@satori/batman`)

| File | What lives there |
|---|---|
| `src/index.ts` | Default-export extension factory; registers `batman_status`, `/batman-status`, the six orchestration tools (via `tools/index.ts`), OMP-native lifecycle listeners (`omp-native/`), and the embedded monitor (`monitor/controller.ts`) |
| `src/context.ts` | `buildStatusContext` — wires state root, repository, binary resolver, client cache; `DEFAULT_IDLE_SECONDS` |
| `src/status.ts` | `getRuntimeStatus(ctx)` — the one shared status path; sanitized failure results |
| `src/client.ts` | `BatmanClient` — NDJSON framing, byte-exact caps, request correlation, Ajv validation of every inbound frame |
| `src/runtime.ts` | `ensureRuntime` (connect-or-spawn, authenticates as `ompExtension`), `buildServeArgs`, `resolveOverride` (`OMP_BATMAN_BINARY` validation), `repositoryIdFromRoot` |
| `src/state.ts` | `resolveStateRoot(env, home)` — must stay semantically identical to Rust's `StateRoot::resolve` |
| `src/platform.ts` | `resolveBatcave` tuple mapping, integrity/version checks, typed errors, `detectLibc` |
| `src/integrity.ts` | `sha256File` |
| `src/tools/shared.ts` | `callOrchestration` — the one execute body every orchestration tool uses; maps `JsonRpcRemoteError` to a stable tool error |
| `src/tools/{tasks,workers,runs,messages,approvals,reconcile}.ts` | `batman_task`, `batman_worker`, `batman_run`, `batman_message`, `batman_approval`, `batman_reconcile` |
| `src/omp-native/events.ts` | Normalizes `task:subagent:lifecycle\|progress\|event` bus payloads into `OmpNativeAgentFact` |
| `src/omp-native/reconcile.ts` | `OmpNativeReconciler` (150 ms progress coalescing, terminal-immediate), `reconcileAcrossRestart` (undetected parent-scoped runs become `lost`), `createOmpProcessEpoch`, `reconcileWithRuntime` |
| `src/monitor/model.ts` | `reduceEvent` — the pure event-reducer building `MonitorState` |
| `src/monitor/render.ts` | Turns `MonitorState` into the widget's concise lines + per-run status detail |
| `src/monitor/controller.ts` | `registerMonitor` — replay-first `session_start` wiring, `/batman [status <runId>]`, retry-on-reconnect |
| `src/monitor/compat.ts` | Test-only `assertCompatiblePiCodingAgentVersion` (never called at runtime — see §6) |

Each module has a sibling `*.test.ts`. `client.test.ts` and `index.test.ts` spawn the real daemon.

### `packages/protocol-ts` — generated contract (`@satori/batman-protocol`)

`src/generated/*.ts` and `schema/batman.schema.json` are build outputs — regenerate, never edit.
`src/validate.ts` is hand-written: it compiles Ajv validators once (`validateInitializeResult`,
`validateRuntimeStatus`, `validateEventEnvelope`, the JSON-RPC envelope validators) and exports
`assertValid` + `ValidationError`.

### `fixtures/` — cross-language golden files

If Rust and TypeScript must agree on something, a fixture pins it: protocol frames
(`fixtures/protocol/`), state-root precedence (`fixtures/state/`), repository-id hashing
(`fixtures/repo-id/`), and the status result shape (`fixtures/omp/`). Both language test suites
consume them, so unilateral drift fails tests.

## 2. Trace: what happens when OMP runs `/batman-status`

Follow this once with the files open and you will have seen every layer.

1. **Registration** — OMP loads the extension and calls the default export
   (`index.ts:batmanExtension`), which registers the tool and command; both handlers call
   `getRuntimeStatus(ctx)` with the context from `context.ts:buildStatusContext`.
2. **Client acquisition** — `status.ts:getRuntimeStatus` reuses a cached `BatmanClient` or calls
   `runtime.ts:ensureRuntime`.
3. **Connect-or-spawn** — `ensureRuntime` computes the socket path
   (`resolveStateRoot` + `repositoryId`) and tries to connect. If nothing answers: `selectBinary`
   validates `OMP_BATMAN_BINARY` (or asks `platform.ts:resolveBatcave` for a packaged binary),
   spawns `batcave serve --state-dir … --repo … --idle-seconds …` detached, and retries with
   backoff (≤5 s).
4. **Daemon startup** — `cli.rs` parses args → `lifecycle.rs:serve` resolves `RuntimePaths`, takes
   the flock (loser exits 73), opens `DatabaseHandle` (migrations + PRAGMAs), appends a redacted
   `runtimeStarted` event through the `Redactor`, binds the owner-only socket
   (`ipc/server.rs:bind`), starts logging (`runtime.log` when detached).
5. **Handshake** — the client sends `initialize` (first frame, 4 MiB bootstrap cap). The server
   already checked the peer UID at accept time. `connection.rs` validates the version range,
   canonicalizes the ompExtension agent directory, negotiates `maxFrameBytes`, computes
   `nextSequence` via `max_sequence()`, and returns `InitializeResult` with the role's allowed
   methods. The client Ajv-validates the result.
6. **The call** — `client.request("runtime/status", …)` → dispatch checks the role table →
   `RuntimeStatus` comes back → Ajv validates → `status.ts` formats `content` text and returns the
   validated object as `details`. On any failure, `failureResult` returns
   `{ isError, code, message (generic), doctorCommand }` — no paths, no stack traces.

The event path is the same shape on the write side:
`RawRuntimeEvent → Redactor::sanitize → PersistableEvent → DatabaseHandle::append_event`
(commit, then reply) → `events/event` notification to subscribers / `events/replay` for
reconnecting clients.

## 3. Trace: submitting a run through `batman_run`, and the monitor observing it live

Same idea as §2, but through the orchestration surface — follow it once and you've seen how a
mutation, the durable journal, and the embedded monitor connect.

1. **The tool call** — the model calls `batman_run` with `{ op: "submit", taskId, workerId }`
   (`tools/runs.ts`); `execute` calls `ctx.getClient(cwd)` (the *same* cached `ompExtension`
   client every orchestration tool and the monitor share — see §18 in `architecture.md` for why
   its role matters) and `callOrchestration(client, "run/submit", params)`
   (`tools/shared.ts`) — nothing more; no worker selection, no retry, no lifecycle inference here.
2. **Dispatch** — `connection.rs::dispatch` sees `BatmanMethod::RunSubmit` is one of the
   orchestration methods, forwards the raw params to `OrchestrationService::dispatch`
   (`service/orchestration.rs`), which the role table (§6 in `architecture.md`) already confirmed
   this connection's `ompExtension` principal may call.
3. **The mutation** — `run_submit` builds a `Run { state: queued, ... }` and calls
   `DomainRepository::submit_run` inside a `run_domain_op` closure. `append_and_apply`
   (`domain/repository.rs`) inserts the event row, learns its `sequence` from the rowid, rewrites
   `event_json` with the bare `RuntimeEvent::RunEvent { kind: RunQueued, ... }`, inserts the `runs`
   projection row, and commits — one transaction, both writes or neither.
4. **The broadcast** — back in `run_submit`, `embed_envelope`/`take_envelope` carry the returned
   `Committed.envelope` across the `run_domain_op` boundary, and `self.broadcast(&mut result)`
   sends it on `Shared.events_tx` *before* the JSON-RPC response is built. Any connection currently
   in `spawn_subscription` (§6 in `architecture.md`) receives it as an `events/event` notification
   in the same tick — this is the fix for the bug in §18.
5. **The adapter seam** — `run_submit` then calls the injected `RunDriver` (none by default this
   milestone): with no driver, it returns `adapter_unavailable` *after* the queued run already
   committed in step 3 — the run is never silently dropped just because nothing can start it yet.
6. **The monitor observes it** — `monitor/controller.ts`'s `client.subscribe` callback (already
   running from `session_start`) receives the notification from step 4, `model.ts::reduceEvent`
   builds/updates the run's row from the `RunEvent` payload, and `refresh()` calls
   `ctx.ui.setWidget` — the row appears in `/batman` without the extension ever polling or
   reconnecting.

## 4. Debugging playbook

**See what the daemon thinks is happening.**

```bash
batcave status --wait-seconds 5 --state-dir <root> --repo <repo>   # JSON snapshot
tail -f <root>/repos/<repo-id>/runtime.log | jq .                  # structured log (detached mode)
```

Find `<repo-id>`: it's the only directory under `<root>/repos/` for that repo, or compute it —
first 32 hex chars of `sha256` of the canonical VCS root path.

**Run the daemon in the foreground while iterating.** `--foreground` puts the same structured
records on stderr and keeps the process attached to your terminal:

```bash
RUST_LOG=debug ./target/debug/batcave serve --foreground --state-dir /tmp/bs --repo "$PWD"
```

(`tracing-subscriber`'s env-filter is compiled in; `RUST_LOG` controls verbosity.)

**Inspect the journal.** It's plain SQLite:

```bash
sqlite3 <root>/repos/<repo-id>/runtime.db \
  'SELECT sequence, timestamp, event_json FROM events ORDER BY sequence;'
sqlite3 <root>/repos/<repo-id>/runtime.db \
  'SELECT operation_id, kind, acknowledged_at FROM operations;'
sqlite3 <root>/repos/<repo-id>/runtime.db \
  'SELECT run_id, state, flags_protocol_unhealthy FROM runs;'   # orchestration projections
```

If you ever see a raw secret in there, that is a P0 — the redaction boundary exists to make it
impossible.

**Talk raw JSON-RPC to the socket.** Useful for protocol debugging without the TS client:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":"1","method":"initialize","params":{...}}' \
  | nc -U <root>/repos/<repo-id>/runtime.sock
```

(Steal a valid `params` object from `fixtures/protocol/initialize.request.json` and fix
`repository`/`agentDirectory` for your machine.)

**Decode common exits and errors.**

| Signal | Meaning |
|---|---|
| exit 73 + `already_running` JSON | Lost the singleton flock race — a daemon already serves this repo |
| `NOT_INITIALIZED` (-32001) | You sent a method before `initialize` |
| `INCOMPATIBLE_VERSION` (-32002) | Version ranges don't overlap protocol 1.0 |
| `METHOD_NOT_FOUND` for a method you know exists | Your role's method table hides it — check `ClientPrincipal::allowed_methods`; if it's `ompExtension`-only, also check you didn't authenticate as `display` (§18 in `architecture.md`, item 2) |
| `ILLEGAL_TRANSITION` (-32100) | The requested `RunState` edge isn't in the canonical lifecycle relation (`domain/transitions.rs`) — check `RunState::can_transition_to` |
| `adapter_unavailable` from `run/submit` | Expected without a wired `RunDriver` (none by default this milestone) — the run is still `queued`, check `run/list`/`run/get` |
| `RATE_LIMITED` from `coordination/send` | More than 30 messages/minute from one sender (`coordination/rate_limit.rs`) |
| Connection dropped with no JSON error | Peer-UID mismatch (dropped before parsing) or an over-cap frame |
| `ValidationError` in the TS client | The daemon sent a frame the schema rejects — regenerate bindings or find the drift |

**Orphan hunting.** Tests and smoke runs are disciplined about cleanup, but if something leaks:
`pgrep -fl batcave`, then `batcave stop` (preferred) or `kill <pid>`. The kernel releases the flock
on death, so the next start recovers automatically.

## 5. Testing guide

**Philosophy:** integration tests exercise real things — real processes, real sockets, real SQLite
files, byte-scans of real WAL files. Mocks appear only at injection seams that exist for the
purpose (peer-credential reader, worker-credential verifier, packaged-binary resolver, uid
provider).

**Where to add a test:**

| You changed… | Put the test in… |
|---|---|
| A wire type / serde shape | `crates/protocol/tests/wire_contract.rs` (+ regenerate, + fixture if cross-language) |
| Domain record shape, `RunState` lifecycle edges, `RunFlags` | `crates/protocol/tests/domain_contract.rs` |
| Coordination message kinds, delivery states, request/result shapes | `crates/protocol/tests/coordination_contract.rs` |
| Path/identity/permission logic | `crates/runtime/tests/paths.rs` (+ `fixtures/repo-id` or `fixtures/state` + the mirrored TS test) |
| DB actor commands or migrations | `crates/runtime/tests/database.rs` |
| Anything touching what gets persisted | `crates/runtime/tests/redaction_boundary.rs` — extend the byte-scan |
| Foundation protocol methods, negotiation, roles | `crates/runtime/tests/ipc.rs` |
| Locking, shutdown, idle, CLI | `crates/runtime/tests/lifecycle.rs` (real-process tests; keep timers ~1 s) |
| `DomainRepository` transactions, projection rollback, event rebuild | `crates/runtime/tests/domain_repository.rs` |
| `task/worker/run/message/approval/reconcile` RPC methods | `crates/runtime/tests/orchestration_rpc.rs` — remember the broadcast half (see the "Adding a new domain mutation" workflow in `getting-started.md`) |
| Coordination broker behavior (bounds, rate limits, scope tokens) | `crates/runtime/tests/coordination.rs` |
| Approval ownership, idempotency, callback, recovery | `crates/runtime/tests/approval.rs` |
| TS client/launcher/extension logic | Sibling `*.test.ts` in `packages/extension/src/` |
| Orchestration tool registration/schema/dispatch | `packages/extension/src/tools/tools.test.ts` |
| OMP-native event mapping, coalescing, restart/`lost` | `packages/extension/src/omp-native/reconcile.test.ts` |
| Monitor event-reducer or rendering | `packages/extension/src/monitor/model.test.ts` / `render.test.ts` |

**Conventions that reviews enforce:** test the real serialized JSON shape (not just round-trips);
no sleeps papering over races (event-driven waits with a deadline); every spawned process is
reaped even on assertion failure; test output stays pristine (a stray warning is a finding);
follow TDD — the suite's failure message before implementation is part of the evidence.

**Fast loops:**

```bash
cargo test -p batman-runtime --test ipc -- --nocapture some_test_name   # one Rust test, with output
bun test packages/extension/src/client.test.ts -t "frame"              # TS tests matching a name
```

## 6. Gotchas

- `crates/protocol/bindings/` fills with `.ts` files when you run `cargo test` (ts-rs side
  effect). It is gitignored scratch; the real bindings are `packages/protocol-ts/src/generated/`.
- The installed `typescript@7` CLI rejects the root tsconfig's `"module": "Bundler"` casing, so
  plain `tsc -p tsconfig.json` fails on config, not on code. Bun handles the config fine; for
  strict type-checks of extension files, use a scoped `bunx tsc --noEmit` invocation with explicit
  flags (see `.superpowers` history) or fix the casing repo-wide when convenient.
- `batcave schema` prints the schema **embedded at compile time** (`include_str!`). After changing
  protocol types, `bun run generate` *and* rebuild the binary, or the printed schema lags the
  types. `generate --check` in CI catches the committed-file half of this.
- Unix socket paths are capped (~104 bytes on macOS). Deep `--state-dir` paths fail fast with an
  explicit error — use `/tmp/...` in tests.
- Lock files are never deleted; ownership is the flock, not file existence. Don't "clean up"
  `runtime.lock` in scripts — deleting it while a daemon runs is harmless to the daemon but makes
  the metadata unreadable to `status`/`stop`.
- **Never resolve a peer package's own metadata (`import ... "@pkg/name/package.json" with {
  type: "json" }`, or equivalent) at extension-load time or module scope.** It resolves fine under
  `bun test`/`bun run` in this repo but hangs the real `omp` binary loading the extension (its own
  bundled module graph, different resolution entirely), and can crash a multi-file `bun test` run
  with an unrelated Bun resolver defect. If you need a peer's installed version, read its
  `package.json` with a plain `fs` walk (see `monitor/compat.ts`). Full story:
  `architecture.md` §18, item 1.
- **A cached client shared by multiple callers needs the union of every role they need, not
  whichever role the first caller happened to need.** `ensureRuntime`'s client is shared by every
  orchestration tool and the monitor; it authenticates as `ompExtension` for exactly this reason.
  Full story: `architecture.md` §18, item 2.
- **A `DomainRepository` mutation that doesn't broadcast its `Committed.envelope` breaks the
  monitor silently** — no error, no test failure, just a widget that never updates for that one
  mutation. See the "Adding a new domain mutation" workflow in `getting-started.md` before adding
  one. Full story: `architecture.md` §18, item 3.
