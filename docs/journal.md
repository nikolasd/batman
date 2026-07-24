# The BATMAN journal — a narrative of how this got built

This is the companion to [the Rust primer](rust-primer.md). The primer teaches you Rust using
this codebase as the textbook; this document tells you the *story* of the codebase itself — every
commit, in order, with the problem it solved, the decision made, the alternatives that lost, and
the test that proved it. Read [architecture.md](architecture.md) for the finished design and
[code-walkthrough.md](code-walkthrough.md) for how to navigate it; read this when you want to
know *why* it looks the way it does, and what it looked like before it did.

Twenty-four commits, two milestones, one running theme: **OMP is the brain, BATMAN is the hands.**
Every decision below either draws that boundary more precisely or discovers where it had blurred.
Where a decision is significant enough to outlive its commit, it has a matching entry in
[`docs/adr/`](adr/) — this journal narrates the *how*; the ADRs record the *what was decided* in
a form meant to survive being read out of context, years from now, by someone who wasn't here.

---

## Part I — Foundation: proving the shape works before it does anything useful

The Foundation milestone's only goal was a vertical slice: OMP loads an extension, the extension
talks to a daemon it can start and reconnect to, one event survives a restart, and one tool
returns real status — no model call anywhere. Twelve commits, one working day and a half.

### 1. `build: scaffold batman workspaces` (e62e5ec)

Every project's first commit is a lie of omission — it looks like nothing, but it's the only
commit that fixes the shape of everything after it. This one decided: two package managers, one
repo. `Cargo.toml` at the root declares a workspace of three crates (`protocol`, `runtime`,
`xtask`) before any of them have real content; `package.json` + `bunfig.toml` declare the mirror
image on the TypeScript side (`packages/extension`, `packages/protocol-ts`). `rust-toolchain.toml`
pins the Rust version so "works on my machine" isn't a debugging step later.

The decision that actually matters here — external extension plus a *separate* Rust binary,
rather than one process — doesn't show up as code in this commit at all. It shows up as the
*absence* of code: no attempt to embed Rust in the OMP process, no attempt to write orchestration
logic in TypeScript. That absence is [ADR-0001](adr/0001-omp-extension-with-separate-rust-daemon.md).
Everything from here on is either building the two sides of that boundary or building the thing
that lets them talk.

### 2. `feat(protocol): define initialization and event envelopes` (480d428)

First real code, and it goes into `crates/protocol`, not `crates/runtime` — on purpose. Before a
single line of daemon logic exists, the wire types exist: `EventEnvelope`, `RuntimeEvent`,
`Timestamp` (a canonical UTC RFC 3339 string, never a raw `DateTime` leaking construction-timezone
ambiguity to the wire), the eight UUIDv7 id newtypes (`ids.rs`'s `uuid_id!` macro), and the
JSON-RPC envelope shapes in `rpc.rs`. Every one of these derives `Serialize`, `Deserialize`,
`JsonSchema` (schemars), and `TS` (ts-rs), and every struct carries
`#[serde(rename_all = "camelCase", deny_unknown_fields)]`.

That last attribute pair is a decision, not a style choice: `deny_unknown_fields` means an
extension talking to a *newer* daemon that added a field it doesn't understand gets a hard error
instead of silently ignoring data — protocol drift becomes loud immediately instead of quietly
corrupting behavior six months later. `tests/wire_contract.rs` (also in this commit) exists
entirely to prove that promise: it serializes real values and asserts the JSON keys are
camelCase, and that an unknown field is rejected.

This is also where "Rust is canonical" stopped being an aspiration and became a fact you could
point at: nothing in `packages/` yet, because nothing in `packages/` is allowed to define a wire
type first. [ADR-0002](adr/0002-rust-canonical-protocol-with-generated-bindings.md) is the
decision this commit enacts.

### 3. `build(protocol): generate schema and TypeScript bindings` (700380f)

Having a canonical Rust type is only half the promise — the other half is that TypeScript never
hand-writes a competing definition. This commit builds the machine that makes that automatic:
`crates/xtask`'s `generate` command walks every `#[ts(export)]` type, emits one `.ts` file per
type into `packages/protocol-ts/src/generated/`, and emits one JSON Schema 2020-12 document
(`batman.schema.json`) from a synthetic `ProtocolDocument` root struct that references every
request/result/event type so nothing gets forgotten.

The `--check` mode (generate into a temp directory, byte-compare against what's committed) is the
part that makes this durable rather than aspirational: `bun run generate --check` runs in every
`bun run check`, so a Rust type change that forgot to regenerate fails CI, not a code review three
weeks later. `fixtures/protocol/initialize.{request,response}.json` — golden files deserialized
through *both* language's types in their respective test suites — is the second half of the same
idea: it's not enough that Rust and TypeScript each think they agree, a shared fixture makes them
prove it against the same bytes.

### 4. `feat(runtime): derive secure repository state paths` (8f8e70a)

Now the daemon crate gets its first real logic, and it's not about sockets or events — it's about
*where anything lives at all*. `security/mod.rs`'s `StateRoot::resolve(env, home)` implements a
three-tier precedence (`BATMAN_STATE_DIR` > `XDG_STATE_HOME/omp/batman` > `$HOME/.omp/orchestrator`),
and `paths.rs`'s `RuntimePaths::resolve` turns a repository path into a stable, private,
per-repository directory: canonicalize it, walk parents for a `.git` marker (directory or file —
worktrees have a file), hash the canonical root to get a `repository-id`, derive a `ProjectId`
from the same hash.

Two decisions worth noticing because they're easy to get wrong quietly: the resolver takes
`env`/`home` as *explicit parameters*, never reading `std::env::var` directly — so a test can drive
every precedence tier from a fixture instead of mutating process-global state (and racing every
other test that also wants to mutate it). And directories are created *with* mode `0700` at
creation time (`ensure_private_dir`), not created-then-chmod'd — there is no window where the
directory exists world-readable. `fixtures/state/state-root-cases.json` is shared between this
Rust resolver and its TypeScript twin (`state.ts`, same commit) — two implementations, one truth
table, so they can never drift about which environment variable wins.

### 5. `feat(runtime): add SQLite journal actor` (8cd8ad8)

This is the commit the Rust primer's Day 6 was written to explain, and it's the one that decided
two things you'll meet constantly for the rest of the project.

First: **SQLite, and only SQLite** — `db/migrations.rs` opens a private (mode `0600`) file, sets
`journal_mode=WAL`, `foreign_keys=ON`, `synchronous=FULL`, `busy_timeout=5000`, and migrates with
`rusqlite_migration`. No Postgres, no Redis, no container — the Global Constraints ruled those out
before this commit was written, but this commit is where "no cloud dependency" became "one file
on disk with WAL turned on." [ADR-0003](adr/0003-sqlite-as-the-sole-persistence-engine.md).

Second, and more interesting to actually *read*: `rusqlite::Connection` isn't `Send` in the way
that makes it easy to share across async tasks, and SQLite wants one writer. So `db/actor.rs`
gives the connection to exactly one dedicated `std::thread`, and every other part of the program
talks to it by sending a `Command` enum value down a bounded `tokio::mpsc` channel, each carrying
a `oneshot::Sender` for the reply. A write command's reply is sent *only after* `tx.commit()`
succeeds — "the call returned" is defined to mean "it's durable," never "it's queued."
[ADR-0005](adr/0005-single-thread-actor-owns-the-sqlite-connection.md) is this pattern; you'll see
it reused, unmodified, for every mutation added in the next twenty commits.

The same commit lands `security/redaction.rs` — `Classified<T>` (a value tagged `Visible`,
`Thinking`, or `Secret`), `Redactor::sanitize` (drops `Thinking`/`Secret` outright, masks
regex-matched secrets in `Visible` text), and `PersistableEvent`/`SanitizedJson` — types with
private fields and *no public constructor*, so the only way to produce one is to pass through the
redactor. `crates/runtime/tests/redaction_boundary.rs` doesn't just unit-test the redactor; it
pushes a fixture with visible text, a secret, and a thinking block through the *real* append path
and then byte-scans `runtime.db`, the WAL file, and `runtime.log` for the raw secret bytes. If this
test ever fails, it fails because a secret reached disk, not because an assertion string changed.
[ADR-0006](adr/0006-type-enforced-redaction-boundary.md).

### 6. `feat(ipc): add negotiated runtime socket protocol` (4ed1b14)

Now the two sides get a way to talk. `ipc/server.rs` binds a Unix domain socket at mode `0600`;
`ipc/connection.rs` splits each connection into one reader task and one serialized writer task
(fed by a bounded channel, so a live event notification can never interleave mid-frame with an
RPC response); the wire format is newline-delimited JSON with a 4 MiB bootstrap cap
(`tokio_util::codec::LinesCodec`), negotiated down after `initialize` to
`min(client offer, runtime max)`, with a 64 KiB protocol floor.

Every one of those numbers is a decision with a "why," and [ADR-0004](adr/0004-json-rpc-2-over-bounded-ndjson-on-a-unix-socket.md)
is where they're written down together rather than scattered across code comments: JSON-RPC 2.0
because both sides already speak JSON-serializable Rust/TS types; NDJSON because it's trivially
line-buffered on both ends without a length-prefix framing layer; a Unix socket, never a TCP
listener, because the security boundary is "same machine, same user" and a TCP port would have to
re-derive that from scratch.

The other half of this commit is authorization: `ClientAuth` (a role-tagged enum —
`ompExtension`/`workerMcp`/`display`, each with different required fields) authenticates a
connection into a `ClientPrincipal`, and `ClientPrincipal::allowed_methods()` returns the *exact*
list of methods that role may call. Dispatch consults this table, never a client-supplied method
name against a blanket allow-list — a method outside the caller's table returns
`METHOD_NOT_FOUND`, indistinguishable from a method that doesn't exist at all. This is
[ADR-0009](adr/0009-role-based-authorization-from-the-connection-not-per-call.md), and it's the
single decision that made every later milestone's new methods safe to add by just extending a
match arm's list, never by writing a new permission check from scratch.

`packages/extension/src/client.ts` is the TypeScript mirror: incremental UTF-8 buffering, a
`Map<string, PendingRequest>` correlation table, and — this is worth calling out on its own —
**Ajv validation of every single inbound frame** before it ever reaches extension logic. A
response with an extra field the schema doesn't know about is rejected client-side, the same
`deny_unknown_fields` promise from commit 2, enforced a second time on the receiving end.

### 7. `feat(runtime): manage detached repository daemons` (18a76fd)

A socket is useless if nothing's listening, and nothing should have to be listening *manually*.
This commit is the daemon lifecycle: `lifecycle.rs`'s `serve()` takes an exclusive `flock` on a
persistent `runtime.lock` file before doing anything else. Exactly one process wins the race; the
loser reads the winner's metadata (written under the held lock) and exits with code **73**,
printing machine-readable `already_running` JSON. There's no lock *file deletion* anywhere in this
design — staleness is implicit, because the kernel releases the flock the instant the owning
process dies, crash or clean exit alike. [ADR-0007](adr/0007-repository-scoped-singleton-via-kernel-flock.md)
picked this over the more obvious "write a PID file, check if that PID is alive" because PID
liveness checks race PID reuse and need a second mechanism (a lock, usually) to be correct anyway
— so skip straight to the lock and let it double as the liveness check.

Idle shutdown (`--idle-seconds N`: exit after N seconds with zero connections and zero active
runs) and graceful stop (SIGTERM or the in-band `runtime/shutdown` RPC: append a redacted
`runtimeStopping` event, close the database actor, remove the socket, release the lock, exit 0 —
in that order, so "the socket is gone" *means* "the journal is closed") round out the daemon side.

The TypeScript side, `runtime.ts::ensureRuntime`, is the connect-or-spawn half:
try to connect first; if nothing answers, validate/select a binary, spawn it *detached*
(`stdio: "ignore"`, `.unref()`, deliberately omitting `--foreground` so the daemon owns its own log
file), and retry connecting with bounded exponential backoff for up to five seconds. If a
concurrent caller won the startup race, this caller just connects to the winner instead of
failing — the flock from the Rust side is what makes that safe. [ADR-0008](adr/0008-connect-or-spawn-with-idle-self-shutdown.md)
is the pair of these: no system service to install, no daemon to remember to start — the first
tool call that needs it starts it, and it shuts itself down when nobody needs it anymore.

### 8. `feat(extension): expose batman runtime status` (7ef7e49)

First payoff: `batman_status` (a tool) and `/batman-status` (a command), both calling the same
`getRuntimeStatus(ctx)` — reuse a cached client or `ensureRuntime`, request `runtime/status`,
Ajv-validate the result, return concise text plus the validated object as `details`. On failure:
`isError: true`, a machine-readable `code`, a **generic** message, and a runnable `doctorCommand`
— never a stack trace, a filesystem path, or an environment value, because those failure paths are
exactly the ones a user might paste into a bug report or a screen-share.

This is the first commit where the vertical slice is actually *whole*: OMP loads the extension,
the extension starts or reconnects to the daemon, negotiates the protocol, and returns real status
without a model call anywhere in the path. Everything built before this commit was necessary but
invisible; this is the first commit you could demo.

### 9. `build: add batcave platform package loader` (39596bc)

Foundation's last piece of real engineering: how does a *packaged* extension (not a dev checkout)
find its daemon binary? `platform.ts::resolveBatcave` maps `(platform, arch, libc)` to one of four
npm `optionalDependencies` leaf packages (`@satori/batman-darwin-arm64`, `-darwin-x64`,
`-linux-arm64-gnu`, `-linux-x64-gnu`) and rejects everything else — musl, Windows, anything — with
a typed `UnsupportedPlatformError`, never a silent fallback to source-building or a generic binary.
For a packaged binary it verifies a SHA-256 against the leaf's `manifest.json` and requires the
leaf's version to equal the extension's own version, so a half-updated `node_modules` fails loud
instead of running a stale binary silently. [ADR-0010](adr/0010-platform-binaries-as-npm-optional-leaf-packages.md).

`OMP_BATMAN_BINARY` — a validated absolute-path override, checked *before* any spawn attempt for
existence/regularity/executability — bypasses all of that for development, which is exactly the
escape hatch every commit and every smoke test from here on actually uses; there's no committed
binary in this repository at all, by design (`crates/xtask package` installs one into a leaf
locally, for a *release* to publish, not for this repo to ship).

### 10. `fix(runtime): close foundation review gaps` (f6237dd)

Every real project has this commit: the one where you stop building forward and go back over what
you just built with fresh eyes. Twelve files touched, none of them new features — hardening the
`repository_id_from_canonical_root` hashing (a new fixture, `repo-id-cases.json`, pins the exact
algorithm against both languages), tightening a couple of `security/mod.rs` permission checks,
closing a database-actor edge case, extending `runtime.test.ts`'s override validation. Nothing
here is a story on its own; together, it's the difference between "the happy path works" and "the
happy path works and the unhappy paths fail the way they're supposed to."

### 11–12. `docs: add README, architecture, onboarding, and Rust primer docs` / `docs: update project title formatting` (5a6c746, 92b6e21)

Foundation's closing act: four documents (`README.md`, `architecture.md`, `code-walkthrough.md`,
`rust-primer.md`) written the moment the code they describe stopped changing under them, not
after. This is the convention this journal and the ADRs in `docs/adr/` are extending, not
inventing — Foundation set the precedent that a milestone isn't done until someone who wasn't in
the room can read four documents and rebuild the mental model from scratch. The title-formatting
fix is exactly as small as it sounds: a one-line polish pass, included here only because skipping
it would make this journal's commit list not match `git log`.

---

## Part II — Orchestration Extension: giving OMP something durable to point at

Foundation proved the shape works. The Orchestration Extension's job was to make that shape *mean*
something: stable task/worker/run records, six tools a model can actually call, a lifecycle no one
can cheat, audited messaging, correlated approvals, and a monitor that shows all of it live — all
without moving one gram of scheduling authority out of OMP and into Rust. Twelve commits, and the
last two are where the plan met reality.

### 13. `feat(protocol): define orchestration records and methods` (3d604af)

Task 1 of the orchestration plan, and it repeats commit 2's move exactly: define the vocabulary in
Rust before writing a single line of runtime logic. `task.rs` (`TaskRef`), `worker.rs`
(`WorkerProfileRef`, `Worker`), `run.rs` (`Run`, `RunSpec`, `RunFlags`, and the ten-variant
`RunState`), `message.rs` (`RunMessage`, `MessageKind`, `DeliveryState`), `approval.rs`
(`ApprovalRequest`), and a new `method.rs` extending `BatmanMethod` with eighteen orchestration
methods.

`RunState::can_transition_to` is the decision this commit is really about: the ten states and
their legal edges are written once, as an explicit relation, and *nothing* transitions a run
except by passing through it. `crates/protocol/tests/domain_contract.rs` — 474 lines, the single
largest test file added this milestone — asserts every one of the 28 legal edges is accepted and
every illegal edge (26 of them, including every self-transition and every edge out of a terminal
state) is rejected. That table-driven exhaustiveness is deliberate: a state machine you can get
*mostly* right is worse than one you get demonstrably completely right, because "mostly" fails
exactly when nobody's watching. [ADR-0012](adr/0012-explicit-run-lifecycle-relation-runtime-evidence-only.md).

`RunFlags`'s six booleans (`degradedControl`, `needsReconciliation`, `protocolUnhealthy`,
`policyQuarantined`, `workspaceDirty`, `childrenActive`) are independent on purpose — none of them
is *derivable* from `RunState`, because a run can be `working` and `protocolUnhealthy`
simultaneously (an approval callback failed, but the run itself is fine) and collapsing that into
the state enum would either lose information or explode the state count combinatorially.

### 14. `feat(protocol): add orchestration domain types and extend dispatch` (cb172d5)

The next thirty minutes of the same task, and the commit message is honest about where it landed:
"3/6 [domain repository tests] pass, 3 fail pending DomainRepository" — this commit is
deliberately mid-flight, TDD in the literal sense the roadmap's skill reference asked for. The
runtime side gets just enough to route the new `BatmanMethod` variants to `METHOD_NOT_FOUND`
(rather than an unhandled-match compile error) and extend the role tables so the *next* commit has
somewhere to plug real dispatch in. `crates/runtime/tests/domain_repository.rs` — 724 lines —
already exists here, red, waiting.

### 15. `chore(protocol-ts): regenerate bindings for orchestration domain types` (84f1912)

A one-file, nineteen-line commit that exists only because `bun run generate --check` said so.
Worth including in this journal for exactly one reason: it's proof the codegen promise from
commit 3 still held eighteen commits and a full milestone later, without anyone having to remember
to run it — the check failed, someone ran `bun run generate`, and this is the diff that fixed it.

### 16. `feat(runtime): persist orchestration projections` (879b421)

Task 2, and the payoff for Task 1's contract. `db/migrations.rs`'s `MIGRATION_2` adds six
normalized tables (`worker_profiles`, `tasks`, `workers`, `runs`, `messages`, `approvals`) with
real foreign keys, alongside the append-only `events` journal from commit 5 — the durable log
stays authoritative; the tables are a queryable *projection* of it, explicitly documented as
rebuildable if they ever diverge (they can't, by construction, but the doc comment says so anyway
for the reader who's suspicious).

`domain/repository.rs`'s `DomainRepository` is where the actor pattern from Day 6/commit 5 pays
rent a second time: every mutating command — `upsert_task`, `create_worker`, `submit_run`,
`transition_run`, and eight more — runs through `append_and_apply`, one SQLite transaction that
appends the event, learns its assigned `sequence` from the rowid, updates the projection row, and
commits. A projection-update failure rolls the event insert back too. `domain/transitions.rs`'s
`check_transition` is the enforcement point for commit 13's lifecycle table — called *before* any
event is appended, so an illegal edge appends nothing at all, not even a "rejected" event.

Six of six `domain_repository.rs` tests, red since commit 14, go green in this commit. That's the
TDD cycle closing, visible in the git history rather than asserted in a commit message.

### 17. `feat(runtime): expose orchestration RPC methods` (c468073)

Task 3: `OrchestrationService` routes every Task 1 method to `DomainRepository` or a read-only
query closure (`service/query.rs`). Two decisions worth pulling out of the diff:

`db/actor.rs` grows a generic `DomainOp` command carrying a boxed closure
(`Box<dyn FnOnce(&mut Connection) -> Result<Value, DomainError>>`) — *not* a generic
"run arbitrary SQL" escape hatch on `DatabaseHandle`. The closure still only calls
`DomainRepository` methods; the genericity is in *which* typed command runs on the actor thread,
never in *what SQL* runs. This is the seam that lets `OrchestrationService`, `ApprovalService`,
and `CoordinationBroker` (three commits away) all share one actor without `DatabaseHandle`
growing a public surface wide enough to bypass the redaction/transaction discipline.

And `service/run_driver.rs`'s `RunDriver` trait, with its one production-shaped implementation
being a *fake*: `FakeRunDriver` drives `queued -> starting -> working` through the same domain
transitions a real adapter would use, and production `ServerConfig::run_driver` defaults to
`None`. `run/submit` without a driver returns `adapter_unavailable` —*after* the queued run is
durably committed, never before, and never by dropping the run. [ADR-0013](adr/0013-injectable-run-driver-seam-fake-by-default.md)
is why this milestone could ship the entire orchestration RPC surface, tools, and monitor without
a single real worker adapter existing yet: the seam is real, the implementation behind it is
deferred on purpose, and "no adapter" is a documented, tested, *durable* outcome rather than a
crash or a lie.

The commit message also owns two real bugs the new tests caught before anyone else did:
`submit_run` had bound `run.started_at` where `created_at` belonged, which would have violated a
`NOT NULL` constraint the moment it ran for real; and four call sites had typo'd `ProjectId::new()`
where `self.project_id` belonged, which would have silently corrupted event provenance — every
event from those sites would have carried a *fresh random* project id instead of the real one.
Both are the kind of bug that a type system can't catch (both sides type-check fine) and only a
test that asserts the *actual value*, not just "it compiled," will find. Worth noting for anyone
skimming this journal for "why does the test suite bother asserting exact IDs instead of just
`is_ok()`" — this is why.

### 18. `feat(extension): add orchestration tools` (16f9a23)

Task 4, and OMP finally gets something to *call*: `batman_task`, `batman_worker`, `batman_run`,
`batman_message`, `batman_approval`, `batman_reconcile`. Every tool's `execute` body is the same
four lines (`tools/shared.ts::callOrchestration`): call the RPC method, shape the result as
`{ content, details }`, or map a `JsonRpcRemoteError` to a non-throwing `{ code, message, data,
isError: true }`. No tool selects a worker, retries, mutates OMP's own todos, approves, or infers
lifecycle state — that discipline isn't a comment, it's a *consequence* of the tool body being too
thin to do any of those things even if someone tried.

The one design wrinkle worth its own paragraph, because the commit message spells it out and it's
a genuinely useful lesson: each tool's parameters use a **flat discriminated field**
(`op: "upsert" | "get"`, checked with a runtime `if`), not a Zod `discriminatedUnion`. The commit
message is blunt about why — combining `z.discriminatedUnion` with this codebase's generic
`ToolDefinition<TParams>` type hit a real TypeScript compiler limit ("excessively deep
instantiation"), and the flat-object-plus-runtime-dispatch shape sidesteps it with *zero* behavior
change, just a less fashionable type. [ADR-0014](adr/0014-flat-op-discriminator-over-zod-discriminated-unions.md)
exists specifically so the next person who reaches for `discriminatedUnion` here finds the
tombstone before they hit the same wall.

### 19. `feat(extension): reconcile OMP native agents` (bfd6620)

Task 5, and the one piece of this milestone that touches OMP's *own* state without ever writing to
it. `omp-native/events.ts` normalizes the installed `task:subagent:lifecycle|progress|event`
bus payloads into `OmpNativeAgentFact` — a status bucket (`working`/`succeeded`/`failed`/`lost`)
deliberately *distinct* from `RunState`, because these are parent-scoped facts BATMAN observes,
never a `Run` row BATMAN itself transitions.

`OmpNativeReconciler` coalesces non-terminal `progress` updates for 150ms (so a noisy stream of
"still running" doesn't spam re-renders) but lets every terminal event through *immediately* and
never lets a stale, still-in-flight coalesced update regress a fact that already went terminal —
a race between "the agent just finished" and "a progress update that started before it finished is
still in the coalescing window" resolves in favor of the truth that already landed.

`reconcileAcrossRestart(priorFacts, currentEpoch)` is the sharpest invariant in the whole
milestone: an OMP-native, parent-scoped agent that a *new* OMP process doesn't re-report becomes
`lost` — never `succeeded`, and never silently promoted into a runtime-scoped `Run`. This is the
project's answer to "what happens when the thing watching a process disappears and comes back": it
doesn't guess optimistically, it doesn't guess pessimistically either in the sense of pretending
nothing happened — it names the uncertainty and moves on. [ADR-0015](adr/0015-omp-native-facts-as-non-owning-mirror-lost-on-omission.md).

### 20. `feat(runtime): broker audited worker messages` (3172d99)

Task 6, the biggest single commit of the milestone (19 files, 2186 insertions), and it fills a
seam Foundation deliberately left open: `RejectAllWorkerVerifier` — the foundation default that
rejects every `workerMcp` connection outright, because there was nothing yet worth letting one
talk to. This commit builds what a supervised vendor process actually gets: `coordination/task`,
`coordination/peers`, `coordination/send`, `coordination/requestChild`,
`coordination/publishArtifact`, `coordination/reportBlocked`, `coordination/askPolicy` — a
worker-safe surface with **no** task-dependency, ownership, or merge mutation reachable from it,
enforced by registering these methods *only* in the `workerMcp` role table.

The trust mechanism is a scope token: `ScopeTokenStore::mint` binds a token to
`{ projectId, taskId, workerId, runId, vendorProcessIdentity, expiresAt }` the instant a vendor
process is (would be) launched; `verify(token, peer_pid)` checks the run/expiry AND that
`peer_pid` is a live descendant of the recorded vendor process, via a portable ps-based
parent-pid walk (`PidAncestryChecker` / `SystemPidAncestryChecker`) that explicitly reports
"unsupported" on platforms without trustworthy peer-process identity rather than accepting an
unverifiable reconnect silently. Token *bytes* are the `HashMap` key and nothing else — never
journaled, never logged, never in a `Debug` output. [ADR-0016](adr/0016-coordination-scope-tokens-bound-to-run-and-pid-ancestry.md)
picked this over a static shared secret (revocable but not scoped to a specific process) or mTLS
(scoped and revocable, but a lot of certificate-lifecycle machinery for a same-machine boundary
that already has Unix-socket UID admission doing the outer layer of the job).

`CoordinationBroker::send`'s delivery semantics are the other half of this commit:
**record-before-delivery** — commit `recorded` first (one durable event, one projection row), then
attempt delivery and commit the outcome. A crash between the two commits leaves a message
`sent`/`recorded`; `sweep_unacknowledged_as_unknown`, run once at startup after journal recovery,
settles anything left in a non-terminal delivery state to `unknown` — and *never resends
automatically*. [ADR-0017](adr/0017-record-before-delivery-message-semantics.md) chose "the
sender finds out delivery is uncertain" over "the runtime silently retries and maybe double-sends"
— for a message that might be `assign` or `cancel`, an unwitnessed duplicate is a much worse
failure mode than an honestly-reported "I don't know."

`domain/repository.rs::request_child`/`decide_child` complete the loop this milestone's
`coordination/requestChild` needs: the requesting run enters `waitingPeer`, records
`ChildWorkerRequested`, and *only the runtime* — never a worker, never Rust guessing — applies the
matching transition back to `working` once OMP answers through `coordination/child/decide`.

### 21. `feat: add correlated worker approvals` (534d3db)

Task 7. `ApprovalService::request` is the seam an adapter calls when a vendor process reports it
needs a human's sign-off: atomically create the request and transition the run
`working -> waitingUser` in one durable event — a decision *paused*, not a decision made. `decide`
enforces, in order: ownership (only the connected principal whose `instanceId` currently owns the
task may decide — a disconnected former owner, even if somehow still holding a socket, is
rejected); idempotency (an identical repeat decision is a silent no-op, never re-invoking the
adapter callback a second time); settled-run rejection (a decision cannot target an already
terminal run).

The sequencing inside `decide` is the part worth memorizing: **record the decision, then invoke
the callback** — never the other way around. On callback success, the run returns to `working`.
On callback failure, the decision is *kept* and the run is marked `protocolUnhealthy` instead of
asking the human again. [ADR-0018](adr/0018-approval-decided-before-callback-never-re-ask-on-failure.md)
is the reasoning: re-asking on a callback failure means the human might approve the same action
twice under two different approval IDs (confusing, and a real "did I actually agree to this"
audit gap), while "kept the decision, flagged the plumbing as broken" degrades gracefully and
leaves a clean signal for whoever's watching `protocolUnhealthy` to go fix the actual adapter.

`approval-ui.ts::showApprovalDialog` is the human-facing half — worker, requested action, *redacted*
arguments, policy reason, approval id — shown only when OMP's own policy marked the decision
`humanRequired: true`, and a dialog timeout leaves the request pending rather than picking a
default. `batman_approval`'s `decide` op checks that flag through `approval/list` before deciding
whether to show the dialog at all, so a model can't skip the human by simply not calling the tool
that would have surfaced them.

### 22. `feat(extension): render the embedded BATMAN monitor` (aabc950)

Task 8, and the milestone's UI. `monitor/model.ts::reduceEvent` is a pure function — one row per
`runId`, built from the `TaskEvent`/`WorkerEvent`/`RunEvent`/`RunFlagsEvent`/`MessageEvent`/
`ApprovalEvent`/`ChildEvent` variants, a no-op for any sequence not newer than what's already
applied (so replaying the same event twice, on reconnect, changes nothing), and structurally
incapable of letting a raw message payload or secret-classified content into the view — only
kind-based labels ever reach it. `render.ts` turns that state into the widget's concise lines (at
most ten rows, with `/batman status <runId>` as the always-available "no, really, show me
everything" escape hatch — a fuller view is a command away, never a silent truncation).

`controller.ts::registerMonitor` is where the "replay-first" idea from the roadmap becomes literal
code, and it's the single most important sentence in this milestone's design:
**there is no separate replay mode.** On `session_start`, read the last persisted sequence from the
session's own custom entry (`pi.appendEntry`), and call `client.subscribe(fromSequence, onEvent)`
— which itself drains `events/replay` first and *then* starts delivering live `events/event`
notifications, but every single event, replayed or live, flows through the exact same
`reduceEvent` call. [ADR-0019](adr/0019-monitor-is-one-reducer-over-replay-and-live-no-separate-modes.md)
is why: a second code path for "catching up" is a second place for a bug to hide, and a monitor
that behaves identically whether it's rebuilding six hours of history or reacting to one live
event is a monitor you only have to reason about once.

`compat.ts::assertCompatiblePiCodingAgentVersion` — a check that the installed
`@oh-my-pi/pi-coding-agent` falls in the pinned `[17.0.7, 18.0.0)` range this monitor's two
surfaces (`pi.appendEntry`, `ctx.ui.setWidget`) are verified against — is written here as a
*test-only* fixture, deliberately never called from `registerMonitor` itself. That restraint
turned out to be exactly half-right, and the next commit is the story of the half that wasn't.

### 23. `fix(runtime): broadcast committed events and authenticate as ompExtension` (49233a5)

This is the commit where "all eight tasks are implemented and every test passes" met "does it
actually work when a real `omp` binary loads it and a real model calls the tools" — and the answer,
on first honest attempt, was no, three times over. This journal exists partly to make sure that
sentence gets read, because a milestone's test suite passing and a milestone *working* are not the
same claim, and the gap between them is exactly what this commit closes.

**The first bug** was in the very restraint praised at the end of the last section: `compat.ts`'s
`import pkg from "@oh-my-pi/pi-coding-agent/package.json" with { type: "json" }`, called from
`registerMonitor` at extension-load time despite the doc comment's "test-only" intent — the
*intent* was right, the *code* still called it from production. That import resolves instantly
under `bun test` in this repo's own `node_modules`, and hangs forever the moment the real `omp`
binary — itself a compiled, bundled Bun executable with its own module graph — tries to resolve
that exact subpath. Bisected down to the bare import statement with no call at all; fixed by
moving the check to test-only code that actually stays test-only, and, since the check itself is
worth keeping for CI, rewriting it to read the peer's `package.json` via a plain filesystem walk
instead of Bun's module resolver.

**The second bug** was subtler and older: `ensureRuntime()` — Foundation's function, written back
in commit 7, when `batman_status` was the only caller and read-only was the correct role — still
authenticated as `display`. Six commits and six new mutation tools later, `index.ts::getClient()`
was caching and reusing that *exact same client* for every one of them. Every mutation failed
`-32601 method ... is not available to this client`, silently, because `display`'s method table is
a strict subset of `ompExtension`'s and nothing checks that relationship until a real call hits it.
Fixed by switching the shared client to `ompExtension` outright — safe for the one existing
caller, because a superset relationship, once you notice it, makes the fix obviously non-breaking.
[ADR-0021](adr/0021-shared-client-authenticates-with-the-union-of-required-roles.md).

**The third bug** was the deepest, and the one that would have kept biting quietly forever if the
smoke scenario hadn't been run for real: `domain/repository.rs::append_and_apply` stored the
*full* `EventEnvelope` into `event_json`, but `replay()` expects that column to hold only the bare
`RuntimeEvent` — so every `events/replay` call failed to deserialize the instant any mutation had
committed. And separately, worse: `Shared.events_tx` — the broadcast channel `spawn_subscription`
reads from — had a subscriber and **no publisher anywhere**. None of the fifteen-plus mutation
call sites across `OrchestrationService`, `ApprovalService`, `CoordinationBroker`, and
`RunDriverContext` had ever called `.send()` on it. Fixed both: storage writes the bare event now;
`Committed` carries the full envelope; `domain::{embed_envelope, take_envelope}` smuggle that
envelope across the `run_domain_op` closure boundary (which is constrained to return a plain
`serde_json::Value`, for reasons that go back to commit 17's generic `DomainOp`) so every service
can broadcast it after every commit. [ADR-0020](adr/0020-per-mutation-event-broadcast-is-not-optional.md).

Two regression tests were added, and their failure modes are worth remembering on their own:
`events_replay_round_trips_committed_mutation_events` failed cleanly against the pre-fix code.
`events_subscribe_delivers_live_notifications_for_orchestration_mutations` did not fail — it
**hung forever**, waiting on a notification that would never arrive, which is exactly what
happened live, in a real terminal, when this bug was first noticed as "the monitor shows nothing."
A test that hangs instead of failing is a worse developer experience but a *more honest*
reproduction of the actual bug, and that's the reason it's kept in this exact shape rather than
wrapped in a timeout that would turn a hang into a tidy red X.

Verified live, in that order — a real `omp` session against the fixed build upserts a task,
creates a worker, submits a run (`adapter_unavailable`, correctly, because no adapter is wired —
the run stays `queued`, not dropped), sends a message, and the embedded `/batman` widget reflects
every one of those mutations without a reconnect. Restarting `omp` against the same repository —
a fresh daemon, since the old one had already idle-timed-out — replayed the identical state from
the durable journal. All 222 Rust tests and 107 TypeScript tests passed, and this time "passed"
meant something, because the thing they were testing had just been driven for real.

### 24. `docs: document the orchestration extension milestone` (fd86ade)

The closing act, mirroring commits 11–12: every document written or extended to match code that
had just stopped moving under it — README's status line, eight new sections in `architecture.md`
(including §18, "Lessons from the smoke scenario," which is the same three bugs from commit 23
told as a permanent design note rather than a one-time fix), a full runnable smoke-test walkthrough
in `getting-started.md`, source-map and gotcha entries in `code-walkthrough.md`, and — because the
project's plan document had every checkbox still unchecked despite every task being done — a pass
through the Obsidian vault's plan and roadmap documents to make the paper trail match the commit
trail. This journal, and the ADRs in `docs/adr/`, are the next entry in that same trail.

---

## Reading order, if you're new here

1. **README.md** — what this is, in two paragraphs.
2. **This journal** — how it got to be that, commit by commit.
3. **`docs/adr/`** — the decisions that outlived their commit, in a form built to survive being
   read out of context.
4. **architecture.md** — the finished design, with no history in it at all.
5. **code-walkthrough.md** — how to find anything, trace a request, and debug it.
6. **rust-primer.md** — if Rust itself is still new, read this alongside the journal; every "Day"
   in the primer is the concept behind one of the commits above.
