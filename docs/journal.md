# The BATMAN journal — a narrative of how this got built

This is the companion to [the Rust primer](rust-primer.md). The primer teaches you Rust using
this codebase as the textbook; this document tells you the *story* of the codebase itself — every
commit, in order, with the problem it solved, the decision made, the alternatives that lost, and
the test that proved it. Read [architecture.md](architecture.md) for the finished design and
[code-walkthrough.md](code-walkthrough.md) for how to navigate it; read this when you want to
know *why* it looks the way it does, and what it looked like before it did.

Ninety-nine commits, four milestones, one running theme: **OMP is the brain, BATMAN is the hands.**
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
npm `optionalDependencies` leaf packages (`@nikolasd/batman-darwin-arm64`, `-darwin-x64`,
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
without moving one gram of scheduling authority out of OMP and into Rust.
[ADR-0011](adr/0011-omp-retains-task-graph-authority.md) is the decision this entire Part either
draws more precisely or discovers where it had blurred: Rust persists OMP-supplied intent
verbatim and enforces only the transitions its own runtime evidence gives it standing to make —
never a scheduling, retry, merge, or worker-selection decision, no matter how convenient one
would be to make locally. Twelve commits, and the last two are where the plan met reality.

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

## Part III — Worker Adapters: giving BATMAN something real to work with

The Orchestration Extension proved the shape could hold state durably. Part III's job was to
give that state *teeth*: a real contract every vendor harness implements, a process supervisor
every adapter shares, four real adapters (Claude, Codex, Copilot, OMP-RPC), a worker-coordination
MCP surface those adapters' own vendor CLIs can call into, and a conformance runner that decides —
per adapter, per scenario — which of a worker's *declared* capabilities OMP is actually allowed to
schedule against. Thirty-four commits, and the running discipline is the same one Part II
established for the domain layer: declare the contract in a trait before any adapter implements
it, and never let a capability reach OMP that a real scenario hasn't proven.

### 25. Journal, ADRs, and two documentation fixes (52587d4, 14c2f20, 0bab8e9, 20bc763)

Before touching a single adapter, four small commits settle the paper trail. `52587d4` is, quite
literally, the commit that wrote the first 24 sections of *this document* and the MADR-format
records under `docs/adr/` — the journal narrating its own origin is as good a proof as any that
"write the docs the moment the code stops moving" (Part I, commits 11–12) held past Foundation.
`14c2f20` fixes a runId gap and a session-topology error the smoke-testing walkthrough had been
carrying since commit 23. `0bab8e9` adds `git-town.toml`, the branch-workflow config this
project's own contribution flow runs on. `20bc763` splits a dedicated `manual-testing.md` out of
`getting-started.md`, the document every later hardening commit's "verified live" claim in this
journal ultimately points back to.

### 26. `feat(runtime): define worker adapter contract` (6e57787)

The seam every adapter implements: `probe/start/resume/send/respondToApproval/cancel/snapshot/dispose`,
object-safe, returning `AdapterFuture<T>` (a boxed future resolving to `Result<T, AdapterError>`),
parameterized by an `AdapterEventSink` passed into `start`/`resume` rather than being a trait
method itself. No method has a default body — every adapter must decide explicitly what each
operation means for it, even if that decision is "return `capability_unsupported`" — nothing here
silently no-ops.

`ProbeResult` is what an adapter *claims*: protocol kind, resume/steering/approvals/usage/nesting/
native-view/workspace-control/durability capability, each a closed strict enum where an unknown
wire value is a hard deserialize error, never silently coerced to a default. Declaring a
capability here is not the same as it being production-approved — the conformance runner built
later in this Part strips any capability whose fixture scenario failed before OMP ever sees it.
This is the same "declare, then prove" discipline commit 17's `RunDriver` seam established for
scheduling; this trait is where it repeats for adapters specifically.

### 27. `feat(runtime): supervise worker process groups` (8c3e1c8)

Every adapter launches its supervised vendor process through `supervisor/process.rs` rather than
calling `tokio::process::Command` directly, so every worker gets the same process-group,
output-bounding, and cancellation-escalation guarantees regardless of which adapter owns it.
`Supervisor::spawn` takes a `SpawnSpec` and returns a `ManagedProcess`; escalation is
SIGINT → SIGTERM → SIGKILL on configurable timings, and `RotatingCapture` bounds captured
stdout/stderr so a runaway or crashed worker's output is truncated, never unbounded.

`EnvironmentPolicy` repeats commit 4's discipline at the process-spawn boundary:
`allowed_env_names: Vec<String>` carries variable *names* only — there is no field anywhere in
this module that could hold an inherited variable's *value*, so a value can never reach the
worker profile snapshot, the durable journal, or a log line through this type, structurally, not
by convention.

### 28–31. The four adapters (f61908a, f6db711, 1e81a57, 1ad4e44)

Four commits, one shape repeated four times, each against a genuinely different vendor wire
protocol: `CodexAdapter` over `codex --app-server`'s JSON-over-stdio; `OmpRpcAdapter`, which
doesn't spawn a process at all but reuses the extension's own Unix-socket connection
(authenticated as `workerMcp`) to let a nested worker call back into OMP; `ClaudeAdapter` over the
installed `claude` CLI's `stream-json` mode; `CopilotAdapter` over `gh copilot`'s ACP
(Agent-Communication-Protocol) mode.

Every one of the three process-spawning adapters shares the same concurrency model: a single
background task owns the `ManagedProcess` exclusively once `start`/`resume` spawns it (its
`write_stdin`/`next_stdout_frame` both require `&mut self`, so no other caller may touch it
directly); `send`/`cancel`/`dispose` talk to that task through an internal `SessionCommand`
channel instead; `snapshot` reads a small `Arc<Mutex<..>>` of session facts the background task
updates as it normalizes frames. And every one draws the same line around what the default test
run may do: `probe()` is exercised for real against the installed CLI (version/auth-readiness
checks only) — never a model call. `start()`/`resume()`/`send()` are real, complete
implementations, but actually calling them would write a real prompt to a real vendor process's
stdin, which *would* invoke the model the instant the CLI reads it — so the default adapter test
suites never call them past their own pre-start guard clauses. A `#[ignore]`d `<adapter>_live.rs`
end-to-end test, gated on `BATMAN_LIVE_<ADAPTER>=1`, is what actually exercises the
spawn+stdin+reader-task path for each.

### 32. `fix(adapter): add omp-rpc host tools / host URI scheme support` (2d34035)

OMP-RPC's host tools need a way to say "this call is coming from a nested worker, not from OMP
itself." The fix: an `omp-rpc://<runId>/<workerId>/<method>` host URI scheme the adapter parses to
recover the run/worker/method it's being asked to act on behalf of — the piece that makes
`OmpRpcAdapter` usable as the vehicle for a *child* worker's coordination calls, not just the
top-level one.

### 33. `feat(runtime): wire all four worker adapters into adapter::mod` (31d849a)

The wiring commit: `adapter::mod.rs` now exports `claude`/`codex`/`copilot`/`omp_rpc`, and
`AdapterKind` is a real four-variant enum a later registry (commit 42) matches on to construct
whichever adapter a worker profile names. Nothing schedules against these yet — that's still
`FakeRunDriver` at this point — but the four adapters now exist as a coherent module, not four
independent commits nobody has connected together.

### 34–36. Coordination MCP: identity, schema, and a real subprocess (82e807c, 7c26e6e, b4c1e0a)

Before an adapter can inject worker-safe tools into its vendor process, the tools themselves have
to exist as an MCP server something can actually spawn. `82e807c` fixes a scope bug in
`coordination/send`: the sender identity was being read from the request payload instead of the
authenticated connection's own scope binding — the kind of bug that lets a caller claim to be
someone else simply by naming them in a field nobody cross-checks, closed by deriving identity
from the socket's own credentials every time, never from anything the caller wrote. `7c26e6e`
defines the MCP tool schemas (`batman_task`, `batman_send`, and their siblings) and an in-process
dispatch table mapping each to the matching `coordination/*` JSON-RPC method. `b4c1e0a` is the
part a vendor CLI can actually exec: `batcave coordination-mcp` — a stdio Model Context Protocol
server that reads `BATMAN_WORKER_SCOPE_TOKEN` from its inherited environment and *removes it
immediately* (never forwarded to anything this subprocess might itself spawn, because it spawns
nothing), connects back to the owner-only repository socket authenticated as `workerMcp`, and
proxies MCP `initialize`/`tools/list`/`tools/call` on stdio to the corresponding `coordination/*`
call over that connection. It never reads the SQLite database directly — every operation goes
through the same authenticated socket any other `workerMcp` client would use.

The bind-race this subprocess has to survive is documented, not papered over: a scope token is
reserved by `ScopeTokenStore::reserve_token` before the vendor process (and therefore, possibly,
this MCP subprocess) has started, and only *bound* to a real pid afterward — so
`connect_and_authenticate` retries only an `InvalidToken`-shaped rejection, for up to two seconds,
and lets every other rejection reason (`NoCredentialStore`, `OutsideAncestry`, `RunNotLive`) fail
immediately, because none of those are transient and masking them behind a multi-second retry
would only delay a real failure, never fix one.

### 37. `feat(runtime): add per-adapter coordination MCP launch helpers` (633c94e)

`mcp_config.rs`: the argv/env/config each adapter's command builder needs to inject
`coordination-mcp` into its supervised vendor process — `coordination_mcp_argv` (separate
arguments, never shell-joined, so no path can be split or injected by embedded whitespace),
`coordination_mcp_env` (only `BATMAN_WORKER_SCOPE_TOKEN`, nothing else added to the vendor's
environment), and `coordination_mcp_config_document` (the `{"mcpServers":{"batman":{...}}}` shape
both Claude's `--mcp-config` file and Copilot's `--additional-mcp-config` inline argument carry —
identical shape, different delivery). `codex_mcp_overrides` is the odd one out: Codex's
`-c key=value` overrides parse as TOML, not JSON, so this module also carries a from-scratch TOML
basic-string escaper — every value it embeds is escaped completely against the full control-
character table the spec requires, not just the two characters a filesystem path happens to use
today.

Every adapter's own native MCP/plugin/skill/hook discovery stays on throughout: nothing here ever
adds a flag that suppresses or replaces it, only one additional named server (`"batman"`)
alongside whatever the vendor CLI already loads from the user or project's own configuration.

### 38. `fix(runtime): reject worker-safe coordination calls once a run has settled` (e1f0898)

A run that has reached a terminal state must not accept any further coordination call — not
`send`, not `requestChild`, not `askPolicy`. Every coordination method now checks the run's state
before attempting anything, and a call against a terminal run returns a rejection
indistinguishable from a method that doesn't exist, never a "permission denied" that would leak
which runs exist to a caller who shouldn't be able to tell. This is the safety net that makes
commit 20's record-before-delivery semantics safe under the scope-token model: if a crash leaves a
message `sent`/`recorded`, the startup sweep settles it to `unknown`, and no further call can
target that run until it is explicitly restarted.

### 39. `feat(runtime): add AdapterMcpConfig reserve/activate helper` (fe6d4e3)

The lifecycle glue between the scope-token mechanism (commit 20) and the per-adapter launch
helpers (commit 37): `AdapterMcpConfig::reserve` mints a scope token bound to
`{projectId, taskId, workerId, runId, vendorProcessIdentity, expiresAt}` the instant a vendor
process is (would be) launched; the corresponding `activate`/`bind` step is what
`ScopeTokenStore::bind` does once the real pid is known. An adapter holds an
`Option<AdapterMcpConfig>` — `None` for a caller (chiefly existing tests) that never asked for
worker-MCP tools at all, so every existing constructor keeps compiling and behaving unchanged.

### 40. Injecting the MCP config into Claude, Codex, and Copilot (bfa8dc8, 1f46410, f6b624a)

Three commits, one already-designed shape (commit 37) landing in three adapters: Claude's
`build_mcp_injection` reserves a token and writes a `--mcp-config` file at owner-only `0600`
permissions, naming only the `coordination-mcp` command/args — never the token itself; Codex's
equivalent writes the same information as `-c` TOML overrides on the `codex app-server` command
line instead of a file; Copilot's writes it as an inline `--additional-mcp-config` argument. All
three delete their config artifact (file or none) once the session ends, and all three treat the
scope token's bytes as the only thing that must never be journaled, logged, or appear in a
`Debug` output — the vendor process's own environment is the token's one legitimate home.

### 41. Answering the host tool calls in OMP-RPC, and a formatting pass (167fddc, f55b36c)

OMP-RPC has no separate MCP subprocess to inject anything into at all — `omp --mode rpc`'s "host
tools" are invoked over the *same* RPC channel the adapter already owns (a `host_tool_call` frame
on its stdout, answered with a `host_tool_result` on its stdin), so `167fddc` wires that in-process
bridge to `CoordinationBroker::execute_tool_call` — the same dispatch table `coordination-mcp`'s
stdio proxy resolves to, just reached without a socket, because the runtime process making this
call is the vendor's own parent, never a descendant of it, and so could never authenticate over
the scope-token socket even if it tried (ancestry is checked in the wrong direction for that path).
`f55b36c` is a pure `rustfmt` pass over pre-existing coordination/approval files — no behavior
change, included here only because it's a real commit in `git log` and this journal's rule from
commit 11 is to never let its own list drift from the one `git` actually recorded.

### 42. `feat(runtime): add adapter registry and conformance runner scaffolding` (90aa259)

The `AdapterRegistry`: implements `RunDriver` (commit 17's seam) by resolving a run's immutable
worker profile, gating start on conformance-derived effective capabilities through an injected
`AdapterAuthorization`, constructing the matching `Adapter`, and owning it for the run's lifetime
in a run-indexed table. `AdapterAuthorization` ships two implementations: `FixtureAuthorization`
(a deterministic allow/deny toggle, tests only) and `DenyByDefaultAuthorization` (the production
default — denies every worker unless a development override is explicitly set, replaced later by
a real `PolicyEvaluator`). The commit's own doc comment is explicit that production callers "must
not ship a permissive production authorization implementation" — a rule the later M4 policy work
(commit 56) exists specifically to satisfy for real.

Alongside it, `crate::conformance`: `run_fixture_conformance(kind)` runs an adapter's fixture
scenarios (zero model calls, always safe) and returns a `ConformanceReport`;
`run_live_conformance(kind)` runs its live scenarios (real model calls, gated per adapter) and
returns the same report shape. This is the scaffolding every later "expand conformance" commit in
this Part fills in.

### 43. `feat(runtime): add adapters and conformance CLI subcommands` (d26e253)

`batcave adapters --json` (every registered adapter kind and its current conformance status) and
`batcave conformance --adapter <kind> [--live] --output <path>` (runs one adapter's suite and
writes the report). These are the only CLI surfaces that expose adapter or conformance data —
the extension's own monitor reads the daemon's state directly, never CLI output, so these two
subcommands exist purely for a human (or CI) to inspect the same facts from outside a running
session.

### 44. Expanding every adapter's fixture suite to fourteen scenarios (c79e0c3, 5f30a9c, 0e3fb11, a983081)

Claude, Codex, Copilot, and OMP-RPC each get a full conformance suite covering the Worker Adapters
plan's Task 8 scenario list verbatim: probe, read-only start/progress, isolated write, follow-up,
approval, every cancellation scope, session resume, vendor reconnect, runtime restart,
result/usage/artifacts, native discovery, redaction, managed-nesting rejection, and unexpected-
child observation — fourteen names, at the time these four commits landed. Not every scenario
applies to every adapter (`VENDOR_RECONNECT` is OMP-RPC-specific; a foreign adapter reports it
`passed: true` with a detail explaining it is not applicable, never silently omits it — omission
and "not applicable" have to stay distinguishable from a scenario nobody ran). This fourteen-name
list is not where it ends: a later commit (55, `aa25584`) removes two of them from the *required*
set once the underlying capability gaps turn out to be either permanent (a protocol wall) or
genuinely optional — `crate::conformance::scenario::ALL` is twelve names by the time this Part is
over, and this journal's honesty rule (commit 23) means saying so here rather than leaving the
"fourteen" claim uncorrected.

### 45. Proving it against reality: a bug fix, integration tests, and an honest catalog (7b5e065, 44fd31f, 7920453, bb60ccd)

Four commits closing out the conformance work the way commit 23 closed out Part II: by actually
running it. `7b5e065` adds CLI integration tests that spawn the real compiled `batcave` binary
against `adapters --json` and `conformance ... --output`, so the CLI's output is checked against
the daemon's real state, not just against itself. `44fd31f` documents, in `manual-testing.md`,
exactly which CLI version and which `BATMAN_LIVE_<ADAPTER>` gate each adapter's live suite needs,
so a human can run one without reading the source.

`7920453` is a real bug the live suite itself caught: Copilot's `resume` scenario was calling
`CopilotAdapter::resume` with a hardcoded, stale session id instead of the id `start` had actually
returned — a mistake that would always fail resume, silently proving nothing about session
persistence at all. Fixed by threading the real returned id through. Exactly the class of bug
this journal has flagged before (commit 17): both sides type-check fine, and only an assertion on
the *actual value* — not "it compiled" — catches it.

`bb60ccd` is the closing move: every fixture-mode scenario that still fails gets sorted into one of
four honest categories — a fixture-mode proof limit (a real live run resolves it today), a
protocol wall (only a future vendor protocol version could resolve it — Copilot's ACP v1 has no
child-observation event at all), a genuine implementation gap (a concrete fix shape exists), or an
environment dependency (needs a reachable vendor CLI or model selector this environment doesn't
have). Separating "worth fixing" from "genuinely can't be resolved short of a vendor change" in
writing is what lets the *next* milestone (Part IV) decide which of these to actually close instead
of re-discovering the same list from scratch.

### 46. `chore: add Serena project config and initialization scripts` (beb67d7)

Serena project configuration and initialization scripts, so the project-management tooling this
team uses can track BATMAN's own tasks and milestones the same way it tracks any other project —
no runtime behavior change, included for the same reason commit 41's `rustfmt` pass was: it is a
real commit, and this journal's list is the commit list, not a curated subset of it.

---

## Part IV — Hardening, Display, & Release: making it production-ready

Part III proved BATMAN could run real workers. Part IV's job was to make that trustworthy enough
to actually ship: real workspace isolation instead of a bare path, real terminal-multiplexer
display instead of a `String`, the structural gaps the plan itself had left open (deny-by-default
authorization *in production*, task content actually reaching an adapter, worker-coordination MCP
actually wired at construction time), layered configuration with org-level locks, a policy
evaluator that replaces the fixture authorization stub for real, and the release scaffolding to
build and publish a `batcave` binary at all. Forty-one commits, and — matching this journal's own
rule about not letting a claim outlive its evidence — this Part also names, plainly, the two
pieces that are still stubs at the end of it.

### 47. `feat(protocol): define workspaces and artifacts` (1272177, f58cd2d)

The wire types for workspace operations, defined the same way every other milestone in this
journal starts: vocabulary before logic. `InspectRequest`/`InspectResult`, `ApplyRequest`/
`ApplyResult`/`ApplyStrategy`, `LeaseMode` (`Exclusive` or `Shared`), `IsolationKind` (`Shared`,
`GitWorktree`, or `Copy`), `WorkspaceInfo`/`WorkspaceState`, and `Artifact`/`ArtifactId`/
`ArtifactKind`. `InspectResult` is designed to carry *evidence*, not an opinion: dirty file count,
untracked file count, recent commit ids, and a patch artifact — the same "durable proof, not a
narrated summary" instinct behind commit 5's redaction boundary and commit 20's audited messaging.

### 48. Workspace scaffolding: materialization, inspection, and a missing dispatch arm (a140979, 987f0e5, 2dac4c9, f607ec6, f08c07c)

Five commits standing up the workspace subsystem's skeleton before it does anything real:
`WorkspaceMaterializer`/`LeaseService` modules and their first tests; a batch of display-backend
test files staged ahead of the display work itself (commit 51); `WorkspaceInspector`/
`WorkspaceApplier` types wired into dispatch; a fixed match arm for the `workspace/*` and
`artifact/*` methods that the previous commit had left unreachable (a compile-time-safe version
of the same "route to `METHOD_NOT_FOUND`, never a panic" discipline commit 14 established for
orchestration methods); and the TypeScript protocol bindings regenerated to match. Nothing here
touches a real filesystem yet in a load-bearing way — that's the next commit.

### 49. Real isolation: git worktrees, file copies, and two symlink bugs (feb1648, 3f18e22, 54985d5)

`feb1648` makes `IsolationKind::GitWorktree` and `IsolationKind::Copy` actually materialize a
working directory: a real `git worktree add` for the former, a real recursive copy (excluding
`.git`) for the latter. Two bugs surfaced immediately, and both are the kind that only exist
because symlinks are a filesystem feature most copy logic gets wrong on the first pass: `3f18e22`
fixes symlink *escape* detection in `WorkspaceMaterializer::validate_path` — a path containing `..`
or resolving through a symlink to somewhere outside the lease root has to be rejected before any
copy or worktree operation touches it, not discovered after the fact. `54985d5` fixes the copy
operation itself: `CopyIsolation::copy` was following symlinks and copying their *targets*'
contents instead of recreating the symlink as a symlink — fixed by checking
`std::fs::symlink_metadata` (which does not follow the link) *before* any `is_dir`/`is_file` check
(which does), so a symlink is always recreated as a symlink, and only a resolved directory or
regular file is ever actually recursed into or copied.

### 50. `feat(workspace): implement real inspect/apply with artifact store` (211811f)

`WorkspaceInspector` now runs real `git diff`/`git status` commands and persists the resulting
patch to `ArtifactStore`, an in-memory (optionally on-disk) content store keyed by `ArtifactId`
with bounded, base64-chunked fetch (`fetch_chunked(id, offset, length)` never loads a whole large
artifact into one response). `WorkspaceApplier` fetches an artifact back out and applies it via
`ApplyPatch` (a real `git apply`) or `CherryPick` (a real `git cherry-pick`), validating the
caller's `expected_target_revision` against the workspace's actual current HEAD *before* mutating
anything and returning `STALE_REVISION` on a mismatch rather than applying against a workspace
that has moved out from under the caller since it last inspected.

### 51. `feat(display): implement display backends with Herdr/Tmux/Terminal` (199011a)

The first cut of the display subsystem: a `DisplayBackendTrait` (`activate`/`status`/
`is_available`), three implementations (`HerdrDisplay`, `TmuxDisplay`, `TerminalDisplay`, the
last one always available as a fallback), a `DisplayRegistry` holding all three, and a
`DisplaySelector` with ordered fallback. At this stage compatibility gating is a bare version
floor (Herdr ≥ 0.1.0, tmux ≥ 3.0) and `activate` does little beyond confirming the backend is
usable — real pane-level operations arrive later, inside the M2/M3 gap-closure squash (commit 55).

### 52. `feat(runtime): complete Task 9 - Terminal adapter with registry integration` (071c9bb)

The fourth "adapter," and the odd one out: `TerminalDisplay`-backed `TerminalDegraded` control for
when a structured adapter's protocol has gone unhealthy and the only remaining option is
terminal-screen automation. `CommandRunner` is injected (never a bare `std::process::Command`
call), so the adapter's own tests never spawn a real terminal multiplexer, and `AdapterRegistry`'s
`run_one` gains the match arm that resolves a `TerminalDegraded` worker profile to this adapter
instead of one of the four `AdapterKind` variants — `TerminalDegraded` was defined back in commit
27's `StartupOptions` enum specifically as the identity that wraps *any* underlying harness rather
than replacing one of the four reserved kinds, and this is the commit that gives it a real
implementation.

### 53. `feat(runtime): wire AdapterRegistry into daemon lifecycle` (f64a61d)

The moment `AdapterRegistry` stops being test-only scaffolding and becomes what `lifecycle::serve()`
actually constructs `ServerConfig::run_driver` from — with `FixtureAuthorization { allow: true }`,
not yet the deny-by-default production policy (that swap is commit 55). Alongside it:
`RunDriver::send_follow_up`, the seam a live message can be forwarded through to an already-running
adapter instance rather than requiring a second `start()` call, and a rename of every adapter's
artifact tracking from `Vec<String>` to `Vec<serde_json::Value>` so a structured artifact (not just
a bare path) can travel the same field. `TODO.md` gets a matching trim — a gap this
commit just closed no longer belongs on the open list.

### 54. Redesigning `batman_task` around a natural-language description (8cf3e72, 975b710)

Two commits undoing a design mistake the same week it shipped. `8cf3e72` is the first patch:
default `ownerClientInstanceId` to `extCtx.sessionManager.getSessionId()` instead of requiring the
model to supply an OMP-internal session id it has no reason to know. `975b710` goes further and
removes the requirement entirely: `batman_task` now accepts a single natural-language
`description`, and the tool itself generates `taskId` (a fresh UUIDv4), resolves
`ownerClientInstanceId` from the session, and defaults `revision` to `0` — the model describes
*what to do*, and the extension owns every protocol detail underneath it. This is the same lesson
commit 18's flat-discriminator ADR taught about tool ergonomics, applied one layer up: a tool
whose parameters mirror the wire protocol exactly is easy to implement and unpleasant for a model
to actually call correctly, and unpleasant-to-call is a defect independent of whether the
implementation underneath it is correct.

### 55. `refactor(conformance): drop OMP-RPC artifact and Copilot subagent gaps from required scenarios` (aa25584)

This is Part IV's version of commit 23 — a single merged pull request, eight phases, that goes
back over "the plan says it's done" with the same fresh eyes commit 10 brought to Foundation, and
finds four real structural gaps still open underneath a green test suite.

**Phase 2 (A4):** `DenyByDefaultAuthorization` replaces `FixtureAuthorization` in
`lifecycle::serve()` at last — every worker is denied unless `BATMAN_DEV_ALLOW_ALL_WORKERS=1` is
explicitly set, closing the exact gap commit 42's own doc comment had flagged as a "must not ship"
item three commits earlier in wall-clock time.

**Phase 3 (A5):** `RunSpec`/`RunDriverContext` grow an optional `prompt: Option<String>`, and it
is threaded all the way to `StartSpec::prompt` — a run's initial content actually reaches its
adapter at start time now, not just at the database-projection layer. `OrchestrationService::message_send`
is wired to `RunDriver::send_follow_up` (commit 53's seam) for live delivery to an already-running
adapter; critically, a failed follow-up delivery — the normal case for a `queued` run with no
adapter running yet — never fails the RPC call or the durably recorded message, it journals a
`Diagnostic(follow_up_delivery_failed)` event instead. The same "don't drop the run, don't lie
about the outcome" instinct from commit 17's `adapter_unavailable` design, one layer further along
the run's life.

**Phase 4 (A6):** `AdapterRegistry::new` now accepts `Option<AdapterMcpConfig>`, threaded into
every Claude/Codex/Copilot adapter it constructs from a resolved `batcave` binary path via
`current_exe()`. OMP-RPC's in-process bridge needs a `CoordinationBroker` instead, which cannot
exist yet at registry-construction time (the real broker only exists after `Server::bind` returns,
which is necessarily *after* the registry must already be handed to `ServerConfig::run_driver`) —
so `AdapterRegistry::set_broker` is a documented post-construction setter for exactly that
ordering constraint, not a design smell. `lifecycle::serve()` also stops unconditionally rejecting
every worker-MCP reconnect: a real `ScopeTokenStore` becomes the server's `worker_verifier`,
replacing the Foundation-era `RejectAllWorkerVerifier` default in production for the first time.
This phase also fixes a message-duplication bug where both `AdapterAuthorization` implementations
had double-wrapped their own rejection string through `RegistryError::AuthorizationDenied`.

**Phase 5:** OMP-RPC's `ApprovalsCapability::Observable` claim gets backed by real state.
`extension_ui_request` confirm/select frames now produce a `PendingApproval`, surfaced through
`snapshot()`'s `state_summary` — never through the event sink, since `AdapterEventPayload` has no
approval variant for this path. Every other `extension_ui_request` (`setWidget`, `notify`, ...)
still produces zero events, deliberately; `respond_to_approval` stays `capability_unsupported` by
design, because no `extension_ui_response` wire path exists to answer one — a capability the
adapter is honest about *not* having, rather than one it silently drops a call against.

**Phase 6:** `batcave monitor` — the CLI-side twin of the extension's embedded widget (commit 22).
Connects as a `display` principal, replays every event from sequence 0, renders one line per
contributing run event via a reducer (`apply_and_render`) that deliberately mirrors the embedded
TypeScript monitor's own `reduceEvent`/`eventPatch`, then follows live events until interrupted.
`--run-id` filters to one run; omitted, it renders every run in the project. No extra redaction
logic is needed here — `EventEnvelope`'s fields are already fully sanitized before reaching the
wire (commit 5's boundary), so there is no raw classified content at this layer left to filter.

**Phase 7:** Herdr and tmux get real pane-level fidelity, replacing commit 51's bare version-floor
gate. Herdr's compatibility check becomes a real `herdr status --json` probe requiring *exact*
client/server protocol equality (cached 5 seconds), grounded against the installed Herdr 0.7.5
binary's real output shape; pane operations (split → run → move/close → report-agent) are
sequenced so a partial failure cleans up only the pane just created, and ownership is tracked
in-memory so this backend never closes a pane it didn't open. tmux gains real pane creation via
`new-window`/`split-window -P -F` (tmux's own print-format convention — no output parsing needed
to recover a created pane id) and now additionally requires a real, already-running tmux session
before reporting itself available, never starting an ambient server as a side effect of a mere
check.

**Phase 8:** A guard test for Copilot's permanent gap. `unexpected_child_observation` is
unresolvable *only* while `COPILOT_MAX_ACP_PROTOCOL_VERSION == 1` — ACP v1 genuinely has no
`session/update` variant for a vendor-spawned subagent. The test inspects `normalize.rs`'s own
source text and fails, with a clear message, if that constant is ever raised without a matching
`NestedWorkerObserved`-producing branch also landing — manually verified by temporarily bumping
the constant to 2 and confirming the guard fires, then reverting. The final `refactor` step then
drops `RESULT_USAGE_ARTIFACTS` and `UNEXPECTED_CHILD_OBSERVATION` from the *required* scenario
list (`crate::conformance::scenario::ALL` goes from fourteen names to twelve) — not because either
gap got fixed, but because both are now honestly optional rather than falsely required, closing
the "genuine implementation gap" and "protocol wall" categories commit 45's honest catalog had
just finished sorting them into.

### 56. `feat(policy): merge immutable runtime configuration` (2ce75b4)

M4's Task 1: layered YAML configuration — org → repo → user → per-run params — with strict
precedence (higher layers win) and org-level field *locks* that prevent any lower layer from
overriding a specific value no matter how it's spelled. `ConfigLayer`'s ordering and
`RuntimePolicy`'s SHA-256 fingerprint are the load-bearing pieces: the fingerprint means two
runtimes that resolved configuration from the same layered inputs can prove, without comparing
the documents byte-for-byte, that they landed on the identical effective policy. Unknown top-level
keys fail closed with line/column diagnostics — the same `deny_unknown_fields` promise from
commit 2, now enforced at the YAML-parsing boundary instead of the JSON wire boundary.

`PolicyEvaluator` (in a sibling `policy` module) is the real `AdapterAuthorization` implementation
commit 42's doc comment had been waiting for: model allowlist enforcement, a concurrency ceiling
using `AtomicUsize::fetch_update`'s compare-and-swap loop (eliminating the TOCTOU race a naive
check-then-increment would have between two concurrent `authorize()` calls), nested-worker policy,
and security-pattern enforcement. `EffectivePolicy` (commit 27's environment-allowlist type) and
`RuntimePolicy` (this commit's org/repo/user-merged policy) are two distinct types on purpose — the
commit's own module doc calls out that the similar names describe unrelated concerns and are never
interchangeable, a naming collision worth documenting rather than silently avoiding by renaming
one of them into obscurity.

### 57. `feat(security): add org-configurable redaction rules and audit module` (f503b9a)

`OrgRedactionRule`: a compiled regex plus a human-readable id (parsed from an inline `# comment`
after the pattern string, or generated from the pattern's index), loaded from an org configuration
document's `security.patterns` array and applied *alongside* — never instead of — the built-in
rules `Redactor::sanitize` already enforces from commit 5. An organization can add redaction
coverage; it can never use this mechanism to remove any of the built-in coverage, because the
built-in rules are compiled into `Redactor` itself and this module never touches that code path.

The `audit` module lands here too: `Export` (JSONL export of events, the implementation behind
`batcave audit export`) and `Retention` (event pruning by age). The commit message is direct about
scope: this is where the module is *introduced*, and `Retention::prune` is, as of this commit,
still a documented stub that returns `Ok(())` without touching the database — real pruning logic
is explicitly deferred, not silently assumed to already work.

### 58. Recovery and Doctor: two commits, two stubs, said plainly (9f51832, 31088c0)

`RecoveryCoordinator` and `Doctor` both land as real types with real public APIs and real doc
comments describing what they *will* check — a database-connectivity probe, state-directory
accessibility, rollout-gate resolution, adapter availability, configuration validity — but neither
commit message hides what's actually inside: both are explicitly titled "stub implementation
ready for full integration." `RecoveryCoordinator` carries `#[expect(dead_code)]` on its own
struct definition, and as of this journal, neither type is constructed anywhere in
`lifecycle::serve()` — a crash leaves a run in a non-terminal state exactly the way commit 7's
socket-disappearance-means-journal-closed design always intended it to, but nothing yet walks the
journal afterward to reconcile it, and `batcave status --recover`'s stated recovery path is not
yet wired to this coordinator. `Doctor::check_database`/`check_state_dir`/`check_configuration`
each carry an explicit `// This is a stub implementation` comment naming exactly what a full
version would additionally do. This journal's rule from commit 23 is to say when something is
real and proven versus merely scaffolded; for these two modules, as of the commits documented
here, the honest statement is: the shape is real, the checks are not yet.

### 59. `feat(release): add CI workflows and xtask for release artifacts` (ebddc6e)

`.github/workflows/release.yml`: a tag-triggered (`v*.*.*`) matrix build across macOS
(aarch64/x86_64), Linux (x86_64-gnu), and Windows, each producing a `batcave-<target>` binary
uploaded as a build artifact and, on an actual version tag, attached to a GitHub Release via
`softprops/action-gh-release`. The commit's own message claims SHA-256 checksums for every
artifact; the workflow committed here packages and uploads binaries but does not yet compute or
publish those checksums as a separate step — a gap this journal's own honesty rule means noting
rather than repeating the claim unchecked.

The commit also adds a *second*, standalone `xtask/` crate at the repository root — distinct from,
and duplicating part of, the `crates/xtask` package tooling commit 3 and commit 9 already built.
This duplicate scaffolding survives for the rest of Part IV and is only removed in the very last
commit of this journal (63, `037bda2`), which is the more honest place to tell that story: a
mistake made here, caught and fixed several commits later.

### 60. `feat(conformance): add TypeScript conformance gates runner` (2b6b53e)

The TypeScript-side counterpart to `batcave conformance`: `packages/extension/src/conformance/index.ts`'s
`runConformance(config)` shells out to the compiled `batcave` binary, supports both `fixture` and
`live` modes, and produces a `ConformanceReport` shaped for CI consumption; `formatConformanceSummary`
renders it as a human-readable pass/fail table. This is the piece that lets a CI pipeline invoke
the same adapter conformance suite Part III built, from a `bun` script, without a developer
manually running the Rust binary and parsing its JSON by hand.

### 61. `feat(runtime): implement all CLI commands` (ddc42e2)

The commit title is exact about what it fixes: `cli.rs`'s `serve`/`status`/`stop`/`monitor`/
`schema`/`audit export` subcommands are wired to their real `lifecycle.rs`/`audit::Export`
implementations for the first time as a complete set — `serve` acquiring the single-instance lock
and starting the IPC server, `status` querying `runtime/status` and printing JSON, `stop` signaling
a live runtime and waiting for socket removal, `monitor` replaying and following events as a
`display` principal, `schema` printing the canonical JSON Schema, and `audit export` delegating to
commit 57's `Export` module. `manual-testing.md` gains a full environment-variables section
(`BATMAN_STATE_DIR`, `BATMAN_ORG_CONFIG`, `BATMAN_DEV_ALLOW_ALL_WORKERS`, `BATMAN_LIVE_<ADAPTER>`,
`OMP_BATMAN_BINARY`) and documents exactly where the state directory and configuration files
resolve to on disk. All 144 library tests and both monitor CLI integration tests pass against the
now-complete surface.

### 62. Sixteen documentation commits, closing the same way every milestone has (cea94b7, baa214c, d8088d2, 6ee06fd, 275eb72, b92e38d, 1f960ac, 114168d, 9f1f313, 358d45f, 73fe828, e7b4f7f, ed1458e, 60eed81, 47904f4, 3e81422)

Sixteen commits, every one of them documentation, mirroring commits 11–12 and commit 24's closing
acts one more time: `docs/m4-hardening-release.md` (a 609-line API reference covering
configuration, security, recovery, doctor, release, and conformance, plus a migration guide and an
explicit "known gaps" list — the same honesty this journal has been asking for, written into the
project's own docs this time); `getting-started.md` rewritten twice over (once broad, once
specifically to cover every M4 feature) and then corrected three times for smaller factual slips
(Homebrew install steps, `bun install` over `npm`/`yarn`, and a wrong claim that Git comes
pre-installed on the target platforms); `CONTRIBUTING.md` added and then the same pre-installed-Git
claim removed from it too; `TODO.md` opened at the repository root to track a real, specific
feature request (org config as a URL, not just a file path) rather than losing it to a chat log;
`358d45f` fixing that exact ambiguity in the org-config documentation before `TODO.md` even
finishes making the case for supporting the URL form; and `architecture.md` restructured twice —
first two incremental updates to reflect the implementation status accurately, then a full
rewrite onto the C4 model (Context, Containers, Components, Code), trading roughly 580 lines of
one structure for 320 of another while explicitly preserving every technical claim, because a
document that describes the *finished* design (as `architecture.md`'s own stated purpose, set in
Part I's closing act, requires) is more useful organized by zoom level than by writing order.
`code-walkthrough.md` and `manual-testing.md` each get a final pass to match the actual M4
codebase rather than the plan that preceded it.

### 63. `docs: update rust-primer.md with verified codebase references, remove dead xtask/ directory` (037bda2)

The last commit in this journal's history, and it closes two loose threads at once. The
`rust-primer.md` update re-verifies every source reference the primer makes against the codebase
as it actually stands at the end of Part IV, rather than as it stood when each "Day" was first
written — the same "docs describe what's actually there" discipline every milestone's closing act
has repeated. And the root-level `xtask/` directory commit 59 introduced — a duplicate of
`crates/xtask`'s release-packaging role that had sat unused for the rest of Part IV — is deleted
outright. Not a deprecation, not a redirect: the dead duplicate is simply gone, and
`crates/xtask` (the one commit 3 and commit 9 actually wired into `bun run generate` and the
platform-package loader) remains the only implementation of that role. A fitting note to end on:
the same rule this journal has followed since commit 10 — finding your own mistake and removing
it outright is not a footnote, it's the work.

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
7. **manual-testing.md** — every live/manual verification step this journal references by name,
   runnable, including the environment variables each worker adapter's live suite gates on.
