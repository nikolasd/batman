# The BATMAN journal — a narrative of how this got built

**Audience & purpose:** anyone curious how the codebase got this way — optional reading for either
manual's audience, not required for either. This is the companion to
[the Rust primer](rust-primer.md). The primer teaches you Rust using this codebase as the
textbook; this document tells you the *story* of the codebase itself — every commit, in order,
with the problem it solved, the decision made, the alternatives that lost, and the test that
proved it. Read [architecture.md](architecture.md) for the finished design and
[code-walkthrough.md](code-walkthrough.md) for how to navigate it; read this when you want to
know *why* it looks the way it does, and what it looked like before it did.

Two hundred and sixteen commits, nine milestones, one running theme: **OMP is the brain, BATMAN is
the hands.** Every decision below either draws that boundary more precisely or discovers where it
had blurred. Where a decision is significant enough to outlive its commit, it has a matching entry
in [`docs/adr/`](adr/) — this journal narrates the *how*; the ADRs record the *what was decided* in
a form meant to survive being read out of context, years from now, by someone who wasn't here.
Parts I–IV (the first 99 commits) close with the very first version of this document; no new ADR
was written for anything in Parts V–IX below — none of those decisions were judged significant
enough to outlive their commit, and this journal says so rather than inventing one to look complete.

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
three-tier precedence (`BATMAN_STATE_DIR` > `XDG_STATE_HOME/omp/batman` > `$HOME/.omp/batman`),
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

## Part V — Distribution honesty: finding the one true install method

Part IV closed with `037bda2`, and the very next commit (`1ee09b9`) wrote the first version of
this journal — the one that narrated Parts I–IV as "ninety-nine commits, four milestones." That
commit's own message is worth recording here because it is this document talking about itself:
"every commit hash from git log is referenced; every ADR link resolves; stubs are documented as
stubs, not working code." Part V picks up immediately after, and its very first commits are a
documentation-accuracy sweep discovering exactly the kind of drift that discipline exists to catch
— module counts wrong, dangling section references, a corrupted file. The rest of the Part is a
longer, more interesting version of the same instinct: repeatedly trying an install method, finding
it doesn't actually work end-to-end, and removing it rather than leaving it half-documented.

### 64. A documentation-accuracy sweep, immediately (7898f25, 042b8ab, 333163a, 14f0e2a, 374447c, 984b221)

Six commits, all docs, none of them adding a feature — the project checking its own homework right
after writing it down. `7898f25` records four adapter conformance gaps straight from test output.
`042b8ab` merges `known-gaps.md` into `known-limitations.md` and deletes `m4-hardening-release.md`
outright as redundant with doc comments already in the source. `333163a` fixes two module-count
typos this journal itself would have inherited (`crates/protocol`: 13 → 14; `crates/runtime`: 16 →
18, both undercounts that had missed newly-added modules) and updates the runtime file map to
match the actual directory. `14f0e2a` fixes a real omission in `architecture.md`'s Level 3
diagram: the `config`/`policy` modules existed in the code but not in the picture, and the fix is
careful to record that `PolicyEvaluator` implements `AdapterAuthorization` but is **not yet wired**
into production (`ServerConfig::default()` still used `DenyByDefaultAuthorization`) — accurate for
this exact commit, a claim Part VIII later needs to update again as the wiring changes.

`374447c` and `984b221` are the two commits worth reading in full if you want to see what
"describe the system as it stands, not as it was written" actually costs when it's skipped even
once. `374447c` finds that an earlier documentation edit had **literally injected the elision
markers a file-reading tool leaves behind** (`…`, `331:`, `355:`) as if they were real file content
— corrupting `getting-started.md`'s Testing/Troubleshooting/Contributing/License sections outright
— alongside a false claim that BATMAN was MIT-licensed when `Cargo.toml` actually said
`license.workspace = true → "UNLICENSED"` at the time, and three fabricated support channels
(`docs.batman.dev`, a Discord, a support email) with no grounding anywhere in the repository.
`984b221` finds that `architecture.md` had been rewritten onto the C4 model (this journal's own
Part IV, commit 62) with zero numbered `§N` sections left in it, while five other documents —
`engineering-lessons.md`, `code-walkthrough.md`, `rust-primer.md`, `known-limitations.md`, even
`README.md` — still cited `§4` through `§18` as if that structure still existed. Every dangling
reference is redirected to what actually exists (mostly `engineering-lessons.md`'s own anchors and
the matching ADRs), and the same pass catches a real diagram bug — two nodes referenced in mermaid
edges but never declared in the subgraph, rendering unlabeled — plus five extension files missing
from both the diagram and the key-components list. Two commits, zero new features, and together
they are the best argument in this journal for why "the docs describe what's actually there" has
to be re-checked, not assumed to hold once it's been true.

### 65. Legal and cosmetic housekeeping (1eb4b33, 7d7cc1f, 3fd3cb2, d5b9af4)

Four small commits: an acronym-expansion formatting fix, a `.gitignore` entry for macOS's
`.DS_Store`, logo/favicon assets in two color variants, and — the one with actual consequence —
`d5b9af4` adding a real `LICENSE` file (MIT, Oh My Pi copyright) and flipping the workspace
`Cargo.toml` from `license = "UNLICENSED"` to `"MIT"`, which every sub-crate inherits via
`license.workspace = true`. This is the fix that makes `374447c`'s corrected claim (three commits
earlier, "BATMAN is *not* MIT-licensed yet") retroactively become true — the kind of ordering this
journal's honesty rule cares about: the correction came first, the fact catching up to it came
second, and neither commit pretends otherwise.

### 66. Six attempts at "how does a user actually install this" (7db3edd, 8495889, 9124642, f34605d, c21f32a, fea66b6)

This is where the saga starts, and it's worth reading as a sequence rather than six independent
commits, because the sequence *is* the point. `7db3edd` rewrites `README.md` for first-time
visitors — a "Why BATMAN?" section, a 5-minute get-started path, and (for the first time) a
"Known Limitations" section stating plainly what doesn't work yet. `8495889` splits Installation
into "For users" (Homebrew or pre-built binaries) and "For developers" (build from source) — a
reasonable-sounding split that `9124642`, one commit later, discovers describes infrastructure that
doesn't exist: no Homebrew formula, no GitHub Releases, no pre-built binaries, full stop. `9124642`
fixes the claim to be honest that users currently must build from source too. `f34605d` then
*builds* the Homebrew formula the doc had promised (`Formula/batman.rb`, platform detection for
four targets) — but notes explicitly that GitHub Releases must exist before the formula's tap can
actually resolve anything, so it's still not usable yet. `c21f32a` is a one-line fix inside that
formula: the GitHub owner was hardcoded to `can1357` (the upstream OMP author) instead of
`nikolasd` (this repository's actual owner) in both the formula and the README — the kind of typo
that would have made the formula 100% non-functional for anyone who tried it, caught before anyone
did. `fea66b6` adds a `curl | bash` install script as a second parallel path, explicit that this
one is meant to actually work once releases exist, not a placeholder.

### 67. Building the release pipeline the install methods above were waiting on (c759a19, e298fb1)

`c759a19` adds a `Publish` subcommand to `xtask` that reads the version from
`packages/extension/package.json`, creates a `v<version>` git tag, and pushes it — the one command
meant to trigger `release.yml`'s existing binary-build-and-publish pipeline. `e298fb1` documents
that command in the README. Both commits are straightforward; the interesting part is what happens
to this exact command four commits later.

### 68. Realizing "runtime-only" was never the actual requirement (4eb3db1, b6e1e1d, b773cb6)

`4eb3db1` notices that everything built so far (`install.sh`, the Homebrew formula) installs only
the `batcave` **runtime** binary — a user still has to separately install the OMP **extension**
themselves, which was never actually documented as a step. Rewritten to install both: the runtime
to `~/.batman/bin/batcave`, the extension to `~/.batman/lib/node_modules/@satori/batman`, no root
privileges required, uninstall reduced to `rm -rf ~/.batman`. `b6e1e1d` goes further and makes a
**local** variant (`install-local.sh`) that works *right now*, on this machine, without any
published release at all — copying the locally-built binary out of
`packages/batman-darwin-arm64/bin/batcave` and `bun add`-ing the extension from the local
checkout — because every method built in commits 66–67 still depended on a release that had never
actually been published. `b773cb6` is the honesty pass on top: the README is rewritten to state,
without hedging, what works **right now** (the local installer, on macOS ARM) versus what's
described for a future that doesn't exist yet (GitHub Releases, Homebrew) — closing a gap where an
earlier version of the doc implied things worked on a fresh clone that, in fact, immediately failed
without a prior build step.

### 69. The one true method, arrived at by elimination (4606fde)

The commit that ends the saga, and it's worth reading in full for how thoroughly it closes the
book on everything commits 66–68 built. Verified against a live `omp` binary rather than inferred
from its `--help` text: `omp install <npm-spec>` (an alias of `omp plugin install`) installs to
`~/.omp/plugins` — a user-owned directory, no root required — and it resolves the extension
package **and** its matching `@satori/batman-<platform>` leaf package (containing `batcave`)
*together*, via the existing npm `optionalDependencies` mechanism this project had already built
for exactly this purpose back in Foundation (commit 9, [ADR-0010](adr/0010-platform-binaries-as-npm-optional-leaf-packages.md)).
One command, both halves, registered for automatic discovery on every future `omp` launch — no
`--extension` flag needed, unlike every prior approach in this saga. `omp plugin uninstall` /
`omp plugin upgrade` give a real, symmetric lifecycle for free.

This supersedes and deletes every previously-proposed method in one commit: `Formula/batman.rb`,
`scripts/install.sh`, `scripts/install-local.sh`, and `xtask`'s `Publish` subcommand (commit 67 —
tagging is two plain git commands, documented in the README, not worth a bespoke wrapper). What's
added instead is the plumbing `omp install` actually needs to work: `publishConfig.registry` on the
extension and all four platform leaf packages (a placeholder URL, documented as a placeholder, not
claimed functional), `.npmrc` scoping the `@satori` npm scope to that registry, and
`release.yml` repurposed from uploading GitHub Release binary assets (which fed the now-deleted
paths) to building all four platform binaries, assembling leaf packages via `xtask package`, and
`bun publish`-ing all five packages to the private registry on tag push — catching two pre-existing
bugs in that workflow file along the way (a renamed GitHub Action, and a `--access public` flag
that had been silently overriding the extension's own restricted `publishConfig`). The README is
rewritten a final time around three sections that actually match reality: Installation (one
method), Development (contributor build-from-source), Publishing (tag + CI, for maintainers) — no
more "For users"/"future" sections narrating infrastructure that doesn't exist.

### 70. Closing the loop: a real contributor setup and doc reconciliation (278af15, b9c16fc, e5b431a, c450639, 4d50bb8)

`278af15` removes leftover Serena MCP configuration and scripts — unrelated tooling that had
drifted into the repo. `b9c16fc` answers a question implicit since Foundation: is there a real,
single-command setup for a *contributor* (as opposed to an end user)? No — `bun install` only
bootstraps the JS workspace; nothing guaranteed the pinned Rust toolchain was present. Fixed with
`scripts/setup.sh` (verifies `cargo`/`bun` are on `PATH`, *warns* rather than silently building
against a version mismatch when `rustup` isn't managing the toolchain — deliberately not assuming
`rustup`, since this was verified on a machine with Rust installed directly via Homebrew, no
`rustup` at all) and `bun run setup` wiring it into `package.json`. The same commit fixes a stale
`github.com/your-org/batman` placeholder in `CONTRIBUTING.md` that had never been updated to the
real repository, and reconciles its Setup section to the same command so the two docs stop
drifting from each other. `e5b431a` propagates the same "two install methods, name them explicitly"
discipline into `getting-started.md` and `manual-testing.md`, replacing three loose manual build
commands with the verified `bun run setup` / `bun run build` pair everywhere. `c450639` adds a
design spec for the monitor-widget work Part VI covers next. `4d50bb8` removes `scripts/install.sh`
outright — it duplicated `omp install`'s own resolution logic and, on inspection, had three real
bugs (a checksum check broken by whitespace-sensitive grep against pretty-printed JSON, a version
precheck that queried the wrong package unauthenticated and picked the *oldest* published version
instead of the latest, and a hardcoded `/usr/local/bin` target with no writability fallback) — this
journal's running theme of "if a mistake is yours, remove it outright, don't deprecate it" holding
one more time. The same commit fixes a dead anchor link, an unclosed code fence that had been
silently swallowing half of `README.md` into a malformed code block, adds the `bun install`/
`bun run build` steps `release.yml`'s publish job had been missing (verified via
`bun publish --dry-run` before and after — without this, a real release would have published the
extension package missing the exact `dist/index.js` file its own `exports` field points at), and
moves the Publishing section out of the user-facing README into `CONTRIBUTING.md`'s new Releasing
section, where a maintainer-only, `SATORI_NPM_TOKEN`-gated procedure actually belongs.

## Part VI — The widget gets a border

A short, self-contained Part: six feature commits and two real bugs, entirely inside the embedded
monitor's rendering layer. Part II (commit 22) shipped the monitor as plain text lines; this Part
gives it a bordered box, per-state icons and colors, and finds two genuinely subtle rendering bugs
in the process of doing it.

### 71. Design-first, again (bf7ec0f, 3b1a8f1)

Two documentation commits before any rendering code: `bf7ec0f` simplifies the widget's border
design down to hand-assembled strings (rejecting a fancier approach in favor of one that's easy to
reason about character-by-character — exactly the kind of decision that matters once the bugs in
commit 73 show up), and `3b1a8f1` writes the implementation plan the next four commits execute.

### 72. Icons, header, and box (a963fb8, c80f4e4, f24584b)

`a963fb8` adds per-`RunState` icon and color lookups. `c80f4e4` adds `renderWidgetHeader`, which
splices a bat-icon header directly into the box's top border line. `f24584b` adds
`renderWidgetBox`, assembling the bordered box around the existing row content with per-state
color applied.

### 73. A UTF-16 surrogate-pair bug, caught by its own test's tautology (6c17348)

Every Nerd Font glyph this widget uses (`BAT_ICON`, every `STATE_ICONS` entry) lives on an astral
Unicode plane, meaning it's stored in JavaScript as a UTF-16 **surrogate pair** — two 16-bit code
units for one visual character. `assembleBox`'s width/padding/fill arithmetic measured every line
with plain `.length`, which counts code *units*, not code *points* — so every icon-bearing line
(every content row, and the header-carrying top border) was over-measured by exactly one unit
relative to the plain bottom border and the icon-free empty-state line, misaligning the box border
by one column. The fix is a `codePointLength` helper (`Array.from(text).length`, which iterates by
code point) swapped in at the four `.length` measurement sites — no arithmetic constant changed,
only what the arithmetic measures.

The more interesting part of this commit is what it says about the *existing* "equal total width"
test: it had been comparing `.length` to `.length`, which is tautologically true no matter how
padding is computed, so it could never have caught this bug even in principle. Rewritten to compare
`codePointLength` instead, plus a second test isolating the exact case that exposed the original
bug (a header carrying an icon, a content line that doesn't) — both verified to fail against a
reverted, `.length`-based `assembleBox` before confirming they pass against the fix. This is the
same lesson this journal has repeated since commit 17: an assertion that can't fail regardless of
the bug is worse than no assertion, because it looks like coverage.

### 74. Wiring it into the live extension, then a second real bug (045f60a, 6a56b15)

`045f60a` renders the bordered widget for real inside the extension. `6a56b15` finds that the OMP
host's `ctx.ui.setWidget` truncates array-content widgets at **10 total lines** — not 10 rows, as
the pre-existing `MAX_WIDGET_ROWS = 10` constant had assumed. Once `renderWidgetBox` wraps content
in a 2-line border, the worst case (10 rows + 1 overflow line + 2 border lines = 13 lines) blew
past that cap, and the host's own truncation silently ate the bottom border — rendering a box that
never visually closes. Fixed by lowering `MAX_WIDGET_ROWS` to 7, so the worst case (2 border + 7
rows + 1 overflow = 10) fits exactly, with the header comment rewritten to state the real
10-total-lines constraint instead of the wrong 10-rows one. The same commit deletes a now-dead
`renderWidgetLines` function (no production callers once `controller.ts` called `renderWidgetBox`
directly) along with its three tests — confirmed, before deleting, that the behaviors those tests
covered (empty state, overflow) remain covered by `renderWidgetBox`'s own tests — and documents a
residual limitation left deliberately unfixed: some terminals render Nerd Font glyphs as visually
double-width despite being a single code point, so the border can still be off by one cell per icon
in those terminals; a full fix needs `wcwidth`/east-asian-width logic, explicitly out of scope here.
`docs/manual-testing.md`'s "Reading the widget line" section (which [`code-walkthrough.md`](code-walkthrough.md)
and this journal both point readers at) is updated in the same commit to describe the real
bordered/iconed/colored format and the corrected 7-row cap, not the pre-Part-VI plain-text shape.

## Part VII — M2/M3 gap closure: doctor, CI, and an honest stub

The "M2/M3 gap-closure" plan named a batch of things the project had claimed were done but weren't
fully wired: a real `doctor` command, a CI workflow that runs on every push (not just release
tags), a conformance gate on release, and operator-facing docs that hadn't been split out yet. This
Part closes most of that list — and is unusually candid about the one piece it closes with a stub
instead of a real implementation, which is exactly the point.

### 75. Naming the gaps before closing them (10a95bb)

Seven new TODO items (10–16), found by re-reading the M2/M3 plan against the running code: no
`coordination-mcp` CLI entry point despite the plan marking it "Closed" (Part VIII closes this for
real), no `batcave display probe` subcommand despite the same claim, crash recovery as a single
untested file instead of the planned kill-point-tested coordinator, no CI workflow on ordinary
pushes/PRs, no conformance gate on releases, no `doctor` command or `/batman-doctor` OMP command at
all, and operator docs not yet split out per the plan. Every item below traces back to one of these
seven.

### 76. Compile errors and a corrupted CLI function, fixed before anything else could proceed (d61050b, 0aac0cd, 339cd39)

`d61050b` fixes a batch of compile errors blocking the doctor/config work: duplicate `#[error]`
attributes on `DbError`, an unnamed-lifetime issue in `config/merge.rs`, a `ToSql` trait issue in
`retention.rs`, a missing `is_blocked()` method on `RolloutGates`, and a `Serve` command pattern
match that hadn't been updated for new config fields. `0aac0cd` finds `run_doctor` itself was
corrupted — three nested, duplicate `match` blocks where one clean block belonged — and adds the
missing `Serialize` derive to `DoctorResult`/`FailedCheck` so `--json` output can actually be
produced. `339cd39` adds the first integration tests for `batcave doctor`: missing database,
JSON-output mode against a missing database, a nonexistent state directory, a nonexistent
repository — verifying both exit codes and output shape.

### 77. A real doctor, reachable from a chat session (0231a8f, b78d38b)

`0231a8f` adds `packages/extension/src/doctor.ts` (`runDoctorCommand`, `buildDoctorContext`) and
registers `batman_doctor`/`/batman-doctor` — the tool shells out to the `batcave` binary directly
rather than going through a live runtime connection, which is the entire point: it's the diagnostic
that works precisely when `batman_status` can't. `b78d38b` marks the TODO item closed, citing the
4/4 passing integration tests and a manual smoke test.

### 78. A CI workflow, immediately trimmed to what actually exists (f20abd3, 8531a05)

`f20abd3` adds the first CI workflow to run on every push/PR (not just release tags): format,
clippy, test (Rust + TypeScript, on Ubuntu and macOS), `generate --check`, and a security job
(`cargo audit` + a secret scanner). `8531a05`, one commit later, removes the JS/TS half of the
format job — no formatter was actually configured yet, so that check could only ever pass
vacuously. (Part VIII's commit 94 fixes this properly by adding Biome.)

### 79. A conformance gate that starts as a no-op, and is caught being one (cbdef62, 41f6bca, a165436, a17a500, c813368, 9516797, e469725, c950d8f, 647ab1a, 94659ab, 2289f0d, 3da37ff, c9ab423, 366b6f4)

This is the longest single arc in Part VII, and it's the clearest example in this journal of a
team catching its own premature "done" claim in writing, in real time, across a dozen small
commits. `cbdef62` adds `tests/conformance/run.ts` and `assert-report.ts` as explicit **stubs** —
`run.ts` writes empty reports, `assert-report.ts` only checks that expected fields are present, and
the commit message says so plainly. `41f6bca` wires a conformance job into the release workflow
ahead of publish, with the same honesty: "conformance job is a stub that always passes." `a165436`
writes the *first* versions of `docs/compatibility.md` and `docs/operations.md` (Task 7 of the same
plan) — a detail worth pausing on, since both documents exist, in evolved form, at the center of
the documentation review this very journal entry belongs to; their earliest ancestor's commit
message is explicit that "only verified claims from actual codebase" made it in. `a17a500` is an
unrelated `clippy`-driven cleanup landing in the same window (deriving `Default` for
`NestedViolationAction` instead of hand-writing it). `c813368` records five pre-existing, unrelated
`adapter_registry.rs` failures in the release checklist rather than hiding them. `9516797` marks
Tasks 14–16 (conformance gates, doctor, operator docs) "completed" in TODO.md — and `e469725`, one
commit later, walks that back with more precision: Task 14 specifically is only "partially
implemented — structural gate wired, but the conformance runner is a stub," because `run.ts`/
`assert-report.ts` write empty reports that a real check would need to reject. `c950d8f` folds that
same honesty into `README.md`'s Known Limitations. `647ab1a` fixes a release checklist file that
had accidentally accreted invalid Markdown after its JSON content (caught because it stopped
parsing under `python3 -m json.tool`).

Then the gate is actually hardened, in three steps: `94659ab` makes `assert-report.ts` throw if any
adapter reports zero scenarios, or if none of its scenarios passed — turning the gate from a
guaranteed-pass no-op into something that can genuinely fail CI, while noting plainly that the
*real* fix (spawning `batcave conformance` for real reports) still doesn't exist yet. `2289f0d`
closes the loop: `release.yml`'s stub report generator now produces an *empty* report on purpose,
which the hardened validator correctly rejects — the gate is now "intentionally blocking release,"
its own commit message's words, until the real runner exists. `3da37ff`, `c9ab423`, and `366b6f4`
are three small follow-up fixes to `assert-report.ts` itself (a duplicated header/import block, a
genuinely missing `readFileSync` import, a stray blank line) — the kind of typo that a stub
implementation, precisely because nothing depended on it working yet, could carry for a commit or
two before being noticed.

### 80. Recovery gets tests before it gets wired (85ea9b9, 1dfa6c9)

`85ea9b9` adds integration tests for crash recovery — explicitly framed as "stub verification,"
since `RecoveryCoordinator` (Part IV, commit 58) still isn't constructed anywhere in
`lifecycle::serve()` at this point; the tests prove the coordinator's own logic works in isolation,
not that it's reachable in production. `1dfa6c9` records the test status in the release checklist.
Part VIII's commit 84 is where the coordinator finally gets wired in for real — and, in a detail
worth flagging now so it doesn't read as a contradiction later, wired in with `#[expect(dead_code)]`
still attached, because the wiring and the *removal from the live daemon lifecycle* turn out to be
two different, sequential decisions this journal narrates in order as they actually happened.

### 81. Two real runtime bugs, found while hardening retention and redaction (7c05d19, 5afa064, d1ac7bb)

`7c05d19` fixes `retention::prune()`: the cutoff timestamp was bound as an `i64` against a column
the schema stores as RFC3339 **text**, a type mismatch that would have made every prune query
compare the wrong representation; and the terminal-state list it filtered against used states that
don't exist (`"completed"` instead of the real `RunState` names `succeeded`/`failed`/`cancelled`/
`lost`) — meaning, before this fix, retention could never have correctly identified which runs were
safe to prune. `5afa064` wires org-configured redaction patterns (Part IV, commit 57) all the way
through `AdapterRegistry::new()` and `DomainAdapterEventSink::new()`, adding a fail-open fallback
in the event-sink construction path that mirrors the one already in `lifecycle.rs` — a decision
this journal flags now because Part IX's review cycle (R14) later finds this exact fallback and
asks whether it's reachable with different behavior than the startup path; the answer at review
time is no, because both paths reuse the same already-validated pattern list, but the trap remains
structurally present for a future change to fall into. `d1ac7bb` retires `known-limitations.md`
outright, folding its two still-uncaptured sections into `TODO.md` and repointing every other
document at `TODO.md` instead — the same "one source of truth for open gaps" discipline `TODO.md`'s
own header still states today — and corrects a stale claim caught in the process:
`PolicyEvaluator` *was* actually wired into `lifecycle.rs` by this point, contradicting a doc that
still described `DenyByDefaultAuthorization` as the only implementation in use.

## Part VIII — The TODO validation era: coordination MCP, policy violations, and workspace isolation

This Part has no single plan behind it the way Parts I–VII each did — it's a sustained, repeated
cycle of the same move: re-read `TODO.md` (or the Obsidian vault plans behind it) against the
running code, find what's stale, fix what's fixable, and write down what's still genuinely open.
Twenty-two commits are pure "the tracking document drifted, here's the correction" work; the rest
are the real features that cycle turned up as missing.

### 82. Coordination MCP gets a CLI entry point, and cross-checking it finds two more bugs (2537d25, 80069a3)

`2537d25` closes the single largest gap Part VII's commit 75 had named: every worker adapter's MCP
launch config (Part III, commit 37) had always pointed at a `coordination-mcp` CLI subcommand that
simply didn't exist — `clap` rejected it outright with an unrecognized-subcommand error the moment
any adapter tried to use it. The fix wires `Command::CoordinationMcp` to the already-implemented,
already-tested `coordination::mcp::run` proxy from Part III, commit 36. The pre-existing
`coordination_mcp.rs` test suite (9 tests, unmodified) goes from a mix of failures to 9/9 — 4 tests
that had been failing with "closed the connection before responding" now pass because the proxy
actually serves stdio, and the other 5 (rejecting missing/expired/mismatched/revoked-token
connections) had been *coincidentally* passing all along against clap's own unrelated
unrecognized-subcommand exit code — verified, after the fix, that they now fail for the real
documented reason instead. A full workspace regression check with `--no-fail-fast` and the change
reverted via `git stash` confirms six pre-existing, unrelated failures aren't new. `80069a3`, found
during the same review pass, fixes three unrelated drift bugs: a hardcoded test expectation for
`ompExtension`'s allowed-methods list that had never been updated after `policy/violation/decide`
was added to the real dispatch table, and two doctests (`recovery.rs`, `doctor.rs`) that used
`Arc<DatabaseHandle>` without importing `Arc` at all, failing `cargo test --doc`.

### 83. A TODO rewrite that finds real gaps, then a second one that fixes a real schema bug (360f0df, 015fafd)

`360f0df` closes the coordination-mcp item, adds three newly-discovered gaps (missing
`batcave conformance`/`adapters` CLI subcommands the Worker Adapters plan's own Task 8 required,
conformance reports omitting the canonical `result_usage_artifacts` scenario, an untracked Copilot
CLI version), and corrects a second stale claim: the `workerMcp` credential store was **not**
reject-all in production by this point — `ScopeTokenVerifier` had already been wired in via
`lifecycle::serve()` the same way `PolicyEvaluator` was — a correction that also lands in
`architecture.md`'s Role Table Summary, verified line-by-line against `ipc/mod.rs`'s real
`allowed_methods()` and `protocol/method.rs`'s real wire names rather than trusted from memory
(exactly the verification this current documentation pass repeats for the same table). `015fafd`
finds the actual root cause behind five long-failing `adapter_registry.rs` tests: its shared setup
helper inserted raw rows into columns that had never been migrated (`workers.task_id`,
`adapter_kind`, `profile_kind`, `status`; `runs.status`, `updated_at`) and omitted two `NOT NULL`
columns the real schema requires — including a foreign key TODO.md's own note had mis-described as
pointing at `adapter_profiles` when the real table is `worker_profiles`. Fixing the shared fixture
exposes a second, fully latent bug underneath it: an assertion checking for `"already started"` or
`"duplicate"` in an error string that the real `RegistryError::DuplicateStart` message never
contains at all (`"run {id} already has a running adapter instance"`) — invisible before because
every one of the five tests crashed in the broken shared setup *before* reaching that assertion.

### 84. Three more TODO rewrites, a provenance-unclear commit handled with unusual explicitness, and a full-suite validation sweep (b22f693, 9702090, 16d6972, 30ef336, cee535c)

`b22f693` closes the `adapter_registry.rs` item and, in the same rewrite, finds that
`tests/domain_repository.rs` — 723 lines, claiming in its own module doc to verify the real
`DomainRepository` — never actually imports or calls that type at all, instead maintaining a
separate, hand-copied schema that had already drifted from the real migrated one. Not a functional
bug (the real `DomainRepository` is correctly tested elsewhere), but misleading coverage that would
keep drifting further from reality the longer it went unnoticed — tracked, not fixed, in this
commit. `9702090` adds the implementation plan for that schema fix. `16d6972` is the one commit in
this entire journal whose own message states it doesn't know who wrote the change it's committing:
a dead `org_security_patterns` field and some needless test cleanup were found already staged in
the working tree, present and unchanged for roughly 38 hours of otherwise-active work, attributable
to no session's own history. Committed anyway, on the user's explicit instruction, only after
independently verifying the change compiles and all 8 redaction unit tests still pass — this
journal's honesty rule extending, for one commit, to "the provenance of this exact diff is
genuinely unknown, and that fact is worth recording rather than glossing over with a plausible
authorial attribution."

`30ef336` is the most thorough validation pass in this Part: every open TODO item checked against
`cargo test --workspace --no-fail-fast` *and*, for the first time in any validation pass, a full
`bun test` run. Result: zero regressions among previously-tracked items, one stale claim corrected
(the OMP-RPC approval-normalization gap had, in fact, already been fixed and was proven by a
passing conformance test — though the artifact-production half of that same item remained
genuinely open), and two new gaps the `bun test` run surfaced for the first time:
`runtime/status.binarySource` always reporting `"unknown"` because `cli.rs` never read the
`BATMAN_BINARY_SOURCE` environment variable the extension had been setting all along, and a stale
tool/command list in `index.test.ts` that predated `batman_doctor`. `cee535c` closes out three
fully-executed implementation plans (this one, the coordination-mcp CLI fix, and the monitor widget
work) by deleting them from the repo's scratch-plan folder and gitignoring it going forward — it's
agent working space, not permanent documentation.

### 85. Real work the TODO cycle turned up: workspace RPC wiring and a proof that cancel kills a real OS process (ae8f279, 4c639ff)

`ae8f279` routes `WorkspaceAcquire`/`WorkspaceGet`/`WorkspaceRelease`/`WorkspaceInspect`/
`WorkspaceApply`/`ArtifactList`/`ArtifactFetch` from `connection.rs` to
`OrchestrationService::dispatch` for the first time — previously every one of those methods,
despite being fully implemented in Part IV, was unreachable over the wire, rejected with
`METHOD_NOT_FOUND` before `OrchestrationService` ever saw the request. `4c639ff` adds the test this
journal's own recurring theme (commit 17, commit 45) keeps asking for: not "does `run/cancel`
return `Ok`," but does the underlying OS process actually die. It constructs a real `OmpRpcAdapter`
against the `fake-worker` fixture, submits a run through the full RPC surface, calls `run/cancel`,
and polls until the fake-worker's real OS pid is confirmed dead — closing the gap between a prior
test (proving `ManagedProcess::terminate()` kills a process in isolation) and the real adapter chain
end to end. It does not prove `SIGKILL` escalation (`fake-worker`'s mode dies on the first `SIGINT`,
so escalation is never exercised by this particular test) — noted explicitly rather than implied.

### 86. Closing the `policy/violation/decide` stub for real (364dee4, d9bb6ff)

`364dee4` is the single largest feature commit in this Part (24 files, 2 new), and it closes a stub
this journal has mentioned twice already (Part IV commit 55's Phase 8 note, and every mention of
`policy/violation/decide` since). `ViolationService::record()` is idempotent — the quarantine/cancel
action applies exactly once, but a `PolicyViolationRecorded` event journals on every observation,
so a second identical observation is provably not silently dropped, just not re-actioned.
`ViolationService::decide()` enforces ownership, refuses to re-decide an already-decided violation,
and refuses `release` outright against a run that's already terminal — the same three-part
enforcement shape (ownership, idempotency, settled-run rejection) commit 21's `ApprovalService`
established for a structurally identical problem. `MIGRATION_4` adds the `policy_violations` table;
`PolicyViolationId` becomes the ninth UUIDv7 newtype in `crates/protocol/src/ids.rs` (the eight from
Foundation, commit 2, gain a peer). `DomainAdapterEventSink` calls `record()` whenever a
`NestedWorkerObserved` event arrives and the run's effective nested-capability isn't `Managed` —
covering both the `None` and `Observable` cases, since either one means the observation itself was
already outside what the run was authorized to do. Enforcement gates land in three call sites that
previously had none: `message/send`, `workspace/apply`, and `coordination/publishArtifact` all now
check `Run.flags.policyQuarantined` and return a new dedicated error code
(`POLICY_QUARANTINED`, -32101) — a run that's quarantined is *actually* blocked from further
progress, not just marked as such for a UI to display. A `nested_violation_action` config knob
(`Quarantine` / `Cancel` / `QuarantineAndCancel`) threads the policy's own choice of remedy from
`RuntimePolicy.rollout_gates` through to `ViolationService`. Four new integration tests prove the
shape holds: quarantine actually blocks `message/send` until released, `decide` is forbidden for a
non-owning client, `release` is refused against a terminal run, and a second observation on an
already-actioned run never double-cancels. `d9bb6ff` is a one-line follow-up removing a stray
leftover header line from the TODO entry this commit closed.

### 87. Naming what's still unreachable from a chat session (b00e863, 633a7d7, 1ee41db, 499e659, b590002)

`b00e863` names a specific, narrow gap: `profile/register` (and a `profileId` field on
`batman_worker`) had no OMP tool wrapping it at all, so a real Claude/Codex/Copilot worker
genuinely could not be created from a live chat session, even though every byte of the underlying
RPC and runtime machinery had worked and been tested since Part III. `633a7d7` is a far larger
sweep: all eight Obsidian vault planning documents re-read in full, one independent reviewer per
document, each verified against the *running code* rather than trusting the plan's own prose. The
Foundation (M0) plan had nothing new to report — already fully implemented, matching this
journal's own Part I. Everywhere else turned up real gaps, most significantly that
`PolicyEvaluator` enforced only two of the six authorization dimensions the Hardening plan actually
specified: cost ceilings and adapter-kind allowlisting had no implementation at all, not even a
stub. `1ee41db` is a one-line, immediately-actionable fix that same sweep produced: the default
concurrency ceiling (applied whenever a layered config omits `concurrency.ceiling` entirely) was
raised from 2 to 8 — 2 having been discovered as impractically low for real use. `499e659` documents
the same `profile/register` gap from a second angle (items 15–16: several RPC methods, not just
`profile/register`, had no OMP tool wrapper — `policy/violation`, `coordination/child`,
`workspace/*`, `artifact/*` were all fully implemented in the runtime and completely unreachable
from a chat session). `b590002` is a pure bookkeeping fix, but a thorough one: a concurrent session
had renumbered nearly every TODO item but only partially updated the internal "item N"
cross-references those items make to each other, leaving several pointing at the wrong item.
Fixed with a script mapping every old number to its new one and checking every "item N" mention in
the body text against that mapping — 27 stale references across 14 items found and fixed, including
two range mentions that no longer corresponded to contiguous ranges at all, spelled out explicitly
instead of left as a range. Re-run after the fix: zero mismatches.

### 88. Tool descriptions, a real conformance CLI, and a genuinely missing events-table column (a033371, 631cacb, 6fcc20b, 7a78a06)

`a033371` rewrites every OMP tool's description to explain when to use it, its key operations, and
typical workflows — aimed squarely at helping a model choose the right tool and invoke it
correctly, not at documenting the RPC shape underneath it (that's what `architecture.md` and
`plugin-usage.md` are for). `631cacb` closes four TODO items at once: `scenario::ALL` had only 12
entries where it needed 14 (missing `RESULT_USAGE_ARTIFACTS` and `UNEXPECTED_CHILD_OBSERVATION`),
which was silently causing three adapters' conformance tests to panic — fixed by extending the
array and adding the missing Copilot scenario function, which in turn exposed that the OMP-RPC
adapter's own `conformance.rs` had never wired either scenario into `build_scenarios()` at all (one
function didn't exist yet; the other existed behind `#[allow(dead_code)]` and was simply never
called). The same commit adds real `batcave conformance`, `batcave adapters`, and
`batcave display probe` CLI subcommands, wired to logic that had already existed and already been
tested — unblocking the conformance release gate Part VII's commit 79 had built as an honest stub.
And it finds a genuinely missing piece of the events schema: the `events` table had no
`task_id`/`worker_id` columns at all, even though `append_and_apply` had been building them into the
in-memory `EventEnvelope` for live broadcast the whole time — they simply evaporated on persist,
and `events/replay` had been hardcoding both to `None` ever since. Fixed with a new migration and
threading both columns through the insert and replay paths (two more columns,
`parent_worker_id`/`vendor_event_ref`, are added in the same migration but remain `NULL` — no write
path supplies them yet, tracked as a separate, still-open gap rather than silently populated with a
guess). `6fcc20b` and `7a78a06` are the paired doc-fix/feature-implementation halves of the same
change: `RecoveryCoordinator` is documented as wired into `lifecycle.rs`'s startup sequence but
still carrying `#[expect(dead_code)]` — the wiring this journal flagged as pending back in commit
80 landing for real, described precisely as "wired but not yet live" rather than either extreme.

### 89. Fixing a bug that would have broken the cross-agent scenario before it could ever start (07619b0, b8994b3, 383bcf1, 354371a)

Four commits, and together they're the difference between "workspace isolation exists in the type
system" and "two workers can actually run in two different git worktrees at the same time." `07619b0`
persists `isolation_kind` in the `workspace_leases` table for the first time (it had always been
hardcoded to `"shared"` before this commit, regardless of what was actually requested) and moves
lease acquisition to two phases — an `allocating` row inserted first, promoted to `active` only
after the workspace is actually materialized — so that isolated workspaces (`GitWorktree`/`Copy`)
can coexist with each other and with shared workspaces, since they occupy disjoint paths and no
longer need the old global write-exclusion to stay safe. `b8994b3` finds the bug that same
restructuring was needed to fix: `workspace_acquire`'s original implementation called
`materialize()` and then discarded its result with `let _ = materialize()` — meaning it created a
*real* git worktree on disk but returned a *fake* `/tmp/ws-…` path to the caller, and leaked the
`allocating` row forever if materialization failed. The rewrite makes the response carry the real,
persisted path from `activate()`, and releases the lease on materialization failure instead of
leaking it. `383bcf1` threads that real path all the way to where it matters: `RunDriverContext`
gains an optional `workspace_path`, `run_one` uses it as the adapter's working directory instead of
the repository root whenever one is present, and `run/submit` acquires an isolated lease whenever
`workspaceMode` is `"isolated"` or `"copy"` — the commit message states plainly what this makes
possible for the first time: two runs with `workspaceMode: "isolated"` now execute in two genuinely
separate git worktrees. `354371a` fixes the two bugs that would have made all of this untestable
from an actual chat session: `batman_run`'s submit path was silently dropping the `prompt`
parameter (every worker would have started with an empty instruction), and `batman_worker`'s create
path was silently dropping `profileId` (Claude and Codex workers would have failed
`PROFILE_REQUIRED` immediately) — both parameters had existed in the schema and simply never made
it into the RPC call.

### 90. The eighth tool, and letting a worker see its peer's workspace (a47c191, 4f0d154, 114291d, 11477e6)

`a47c191` adds `batman_profile` (wrapping `profile/register`) and `batman_workspace` (wrapping
`workspace/acquire|get|release|inspect`) — the two tool gaps commits 87 and 89 had already named as
blocking the cross-agent scenario — bringing the OMP tool count to eight. `4f0d154` adds
`CoordinationPeerWorkspace`, a new RPC method letting a worker resolve a same-task peer's workspace
path/mode/isolation-kind/state for direct cross-workspace code review, exposed as an eighth
worker-safe MCP tool (`batman_peer_workspace`) alongside a fix that `batman_peers` had been omitting
each peer's `runId` from its response the whole time. `114291d` updates every document this journal
has been checking for staleness throughout this Part — `architecture.md`, `code-walkthrough.md`,
`manual-testing.md` (which gains a new §5 for the cross-agent workspace-isolation scenario),
`getting-started.md` — to say "eight tools" and "RecoveryCoordinator is wired," and is explicit that
this journal's own earlier "six tools" references (Foundation-era, Part II) are deliberately left
unchanged as historical record rather than silently updated to match the current count. `11477e6`
closes four TODO items this Part's work resolved (workspace-mode threading, the two new tools, peer
workspace resolution) while leaving one open on purpose — worker-MCP artifact list/fetch was
deliberately excluded from the plan's scope, not forgotten — and removes crash recovery from
README's Known Limitations now that it's genuinely wired.

## Part IX — A hardening squash, then a review that finds what it missed

Part IX closes this journal, and it does so with two very different textures back to back. The
first eleven commits are a wide, parallel-authored hardening pass across almost every subsystem at
once — each commit's own message is a single terse line with no body, which this journal notes
plainly rather than inventing detail the commits themselves don't provide. The second half is the
opposite: a formal, four-reviewer codebase review (`REVIEW.md`) that re-reads the entire hardened
system with fresh eyes and finds four critical, production-blocking bugs the tests had missed —
followed by the same-day discipline this journal has praised since commit 10: finding them, fixing
them, and writing down exactly what was fixed and what's still open.

### 91. A parallel hardening squash across nine subsystems (38c8c3f, 6a08785, 4fad81c, fc5f9db, cb6842f, 274b0d5, 4621add, 7a7a4c0, 56507fa, 9f85dc3, 02f3426)

`38c8c3f` adds the Biome formatter and a CI format gate for TypeScript — the gap Part VII's commit
78 had explicitly deferred, closed here for real; its own commit message notes that TypeScript
formatting changes from this point on travel with the commits that own them, rather than arriving
as a single repo-wide reformatting diff. `6a08785` regenerates the shared schema/TS-bindings
codegen in one reproducible commit, keeping Rust protocol definitions and their generated output
never more than one commit apart — the same discipline `bun run generate --check` has enforced
since Foundation, commit 3. The next seven commits are titled by subsystem rather than by story —
`feat(runtime/db,domain): harden event persistence and recovery`,
`feat(runtime/policy,security): enforce run policy and fail closed`,
`feat(runtime/workspace): harden leases, conflicts, and artifact limits`,
`feat(runtime/adapter): harden live conformance and event normalization`,
`feat(runtime/ipc): expose workspace, artifact, child, and display workflows`,
`feat(runtime/cli): add audit export, doctor checks, and startup sweeps`,
`feat(extension): add OMP tools and restart reconciliation` — and none of the seven carries a
commit body beyond that one line. This journal records that plainly rather than reconstructing a
narrative these commits didn't write down themselves: each is a substantial, subsystem-scoped
hardening pass, landed together, and the accurate account of *what* changed in each is the source
tree at that commit and the tests that shipped with it, not a retrospective story. `9f85dc3` adds a
release provenance matrix and makes the conformance gate real (superseding Part VII's honest stub).
`02f3426` is a documentation commit refreshing closed gaps and pruning completed items out of
`TODO.md` — the routine maintenance this journal has shown recurring throughout Part VIII, once
more, after a large batch of work lands.

### 92. One eager-cleanup fix and one flaky-test hardening (3907e8f, a79d4ee)

`3907e8f` makes subscription-forwarder tasks exit as soon as the writer half of a connection
closes, instead of waiting for another broadcast to notice — closing out TODO item 49 and a small
resource-cleanup gap in `ipc/connection.rs`. `a79d4ee` fixes a genuine race in the lifecycle lock
tests: the *losing* process in a singleton-flock race can exit the instant it observes the winner's
lock, before the winner has finished opening its database and binding its socket — a test asserting
the winner's socket exists *immediately* after the loser exits was racing the winner's own startup.
Fixed by using the test suite's existing bounded wait instead of an instantaneous assertion.

### 93. A four-reviewer codebase review, and four critical fixes the same day (889cbd8, b004857, 6a4c506, 86244da, 3678b99, cafa0e0, 26dcf07)

`889cbd8` is `REVIEW.md`'s first commit — the document this documentation pass has been
cross-referencing throughout Parts VII and VIII. Its own method section is worth restating here
because it's a real methodology, not filler: the tree was split across four parallel reviews
(runtime core; adapters/policy/security; TypeScript/OMP integration; build/docs/release), every
Critical and High finding was re-read against its cited source before inclusion, and leads that
turned out to be strengths rather than bugs were removed rather than kept for volume. Four Critical
findings came out of it, and three are fixed in this journal's very next three commits — the same
same-day-fix discipline this journal has praised in every prior review-shaped commit (23, 45, 55)
holding one more time.

`6a4c506` fixes **R1**: the extension authenticated every runtime connection with the constant
`instanceId: "batman-extension"`, while `batman_task upsert` recorded the real OMP session ID as
`ownerClientInstanceId` — meaning approval and policy-violation decisions, which require exact
identity equality, could *never* be decided by the session that created them. Fixed by threading an
optional `sessionId` through `EnsureRuntimeOptions`, `initParams`, `tryConnect`,
`connectWithBackoff`, `ensureRuntime`, `getClient`, `statusContextFor`, and all eleven tool call
sites — closing the status-path gap in the same commit rather than leaving it as a known follow-up.
`86244da` fixes **R2**, the single highest-impact bug this review found: each successful worker
authorization incremented `PolicyEvaluator`'s `active_runs` counter, but `PolicyEvaluator` was
immediately erased behind the `AdapterAuthorization` trait object, whose interface had no release
method at all — meaning after `concurrency_ceiling` **cumulative** runs (not concurrent — every run
ever authorized, forever), the daemon would permanently refuse every new run until restart. Ordinary
sustained use would eventually and silently disable the runtime's core function. Fixed by adding a
`release()` method to the trait (a no-op for `FixtureAuthorization`, a real `decrement_runs()` call
for `PolicyEvaluator`), called by the adapter completion watcher after evicting a settled adapter,
and by `run_one` on every post-authorize error path — defended by an integration test that books a
`concurrency_ceiling: 1` slot through the real `PolicyEvaluator`, proves a second run is denied,
releases the slot through the trait object, and proves the ceiling denial clears. `b004857` fixes
**R3** and **R4** together, both in the release pipeline: R3 was that the Linux ARM64 release
target built on an x86_64 GitHub runner with no AArch64 cross-linker installed at all (fixed by
installing `gcc-aarch64-linux-gnu` and setting the matching `CARGO_TARGET_*`/`CC_*`/`AR_*`
environment variables, plus a new dry-run CI workflow exercising every release target on every
push); R4 was that GitHub's artifact-upload/download cycle silently strips the executable bit
`xtask package` had set, which the package-set assembly step correctly rejected — meaning even
after R3's fix, no release could complete without a person noticing the rejection and manually
`chmod +x`-ing something. Fixed by removing the release workflow's destructive flatten loops and
having both the package-set and publish jobs run `find ... -name batcave -exec chmod +x {} +` after
every artifact download, restoring the bit the platform itself removes.

`3678b99` records the resolutions for all four in both `TODO.md` and `REVIEW.md` — R2–R4 fully
closed, R1 (the identity fix) marked partially closed pending a dedicated end-to-end test rather
than claimed complete on the strength of the fix alone. `cafa0e0` adds that test: two live-daemon
integration tests proving the full `sessionId → instanceId → ownerClientInstanceId` chain — the
positive case seeds a task/approval/violation owned by session A, connects as A, and confirms both
decide calls succeed; the negative case seeds the same data but connects as session B, confirming
both decisions are rejected with "does not own." `26dcf07` marks R1 and TODO item 68 fully resolved
on the strength of that test — the same pattern this journal has called out since commit 21: no
fix is recorded as closed until the test proving it exists, not just the diff implementing it.

### 94. Repo guidance for future sessions, and the exact commits this documentation pass grew out of (eba1556, 60e8fa3, d1ef420, 0f670dc)

`eba1556` adds `AGENTS.md` (the canonical, exhaustive directory table and invariant reference) and
`CLAUDE.md` (a working summary that defers to it) — the two files whose own text this journal's
Parts VII through IX have been cross-checking claims against throughout. `60e8fa3` is the direct
ancestor of the documentation review this very journal entry is part of: it adds
`docs/cli-reference.md` and `docs/plugin-usage.md` as new documents for the first time (covering
every `batcave` subcommand/flag and all eleven orchestration tools respectively), and rewrites
`docs/operations.md` to remove content that had never been true — invented Homebrew/apt uninstall
steps, a fabricated Herdr-restart feature, a fake compatibility matrix — while fixing real,
verified inaccuracies (the lock mechanism, the state-dir default, missing subcommands) and
deferring to the new `cli-reference.md` for flags instead of duplicating them. The same commit fixes
a fabricated `--port` flag and `--recover` flag in `getting-started.md`, a fabricated config
auto-discovery path, a wrong `Redactor::new()` call, an incomplete health-check list, a stale test
count, and permission-error guidance that told readers to `chmod 755` their state directory —
directly contradicting that same document's own `0700`/`0600` security claims two sections above
it. `d1ef420` adds `batcave capture`, automated tooling to regenerate adapter conformance fixtures
from real vendor CLI output (a deterministic scrubber replacing session IDs, timestamps, costs, and
paths with stable placeholders while preserving the correlation IDs conformance suites assert on,
so re-capturing an unchanged CLI is byte-identical) — replacing what had been, until this commit,
hand-authored fixture JSON. `0f670dc`, the commit this journal's Part IX ends on, adds `release/` to
`AGENTS.md` and `CLAUDE.md`'s directory tables (a top-level, cross-language directory that both the
Rust build tooling and CI had been reading without either guidance document mentioning it) and
gitignores the release manifest CI generates fresh on every run — closing this journal at the same
kind of small, unglamorous accuracy fix it opened Part V with, which is a fitting place to stop:
the discipline this document has narrated since commit 10 is still the same discipline in commit
217, wherever the next one after this journal's own writing turns out to be.

## Part X — REVIEW.md's second pass: seven more fixes, eleven doc corrections, and the residue that outlived them

Part IX closed on four Critical fixes landed the same day they were found. The seven High findings
from that same first review round (R5-R11) got the identical same-day discipline, across the fix
commits `8331a34 9720c63 8457de5 6bd6a00 f9e95c4 797d5e6 e8204da 44093d4 e4befb8 bcff4ce 143e1b3`.
Unlike R1-R4, every one of these seven left a smaller, real gap behind — not a regression, but a
residual defect the fix itself introduced or exposed. This journal records both halves, because a
"resolved" that quietly grew a new open item is not the same story as a clean close.

**R5** — a `humanRequired` approval could be decided by the model itself, with no human in the
loop. Fixed by adding a `DecidedBy` enum (`Human`/`Model`) to the protocol and rejecting a `Model`
decision against a `human_required` approval in `ApprovalService::decide`; the extension fails
closed with no UI path around it. Left behind: **R34** — the fix persists `decided_by` via
`serde_json::to_string`, storing the JSON-quoted `"human"` instead of the bare token every other
scalar-enum column in the same file uses, so `WHERE decided_by = 'human'` returns nothing, forever.

**R6** — a cached runtime client that had died silently broke every tool call until `batman_status`
happened to be invoked. Fixed by exposing `BatmanClient.isClosed` and routing every construction
site through a `resolveClient()` that reconnects on a closed cache; defended by `reconnect.test.ts`.
Left behind: **R39** — the fix's own repair path correctly pairs `controller.stop()` with clearing
`subscribedClient`, but the `session_shutdown` handler calls only `controller.stop()`, so a monitor
that lives through a session shutdown without that pairing can end up permanently unable to
reconnect.

**R7** — `run/retry` created a queued run and then never started its adapter. Fixed by routing
`run_retry` through the same `start_queued_run` helper `run_submit` already used. Verified, not just
fixed: `orchestration_rpc.rs` proves a retried run actually starts. No gap left behind — the shared
helper closes the class of bug outright.

**R8** — the release conformance gate ignored aggregate failure; a stub could pass green. Fixed
(`de07022`) by gating `batcave conformance --fixture` against a committed
`fixtures/conformance/fixture-mode-baseline.json`. Left behind: **R44** — the capture tool that
produces that baseline is calibrated against exactly one of the eleven committed fixtures (its
scrubber only recognizes `claude/initialize.jsonl`'s placeholder ID family as already-canonical),
and its `unchanged` flag is computed by reading back the file it just wrote, not by comparing
against what was committed before the write — so the safety net the gate depends on is itself
unproven beyond the one fixture it was built against.

**R9** — release version checks validated the git tag but not the packages actually assembled for
distribution. Fixed (`bb209eb`) by having `package-set` verify each leaf's own version and adding a
`version-gate` CI job that checks the tag against `v<version>` before any build work starts. No gap
left behind.

**R10** — artifact APIs claimed task-level isolation but were scoped project-wide, so one task could
read another's patches. Fixed (`44093d4`) by stamping `Artifact.run_id` at the point of production
and scoping `artifact/list`/`artifact/fetch` by `owner_client_instance_id`, proven by a dedicated
cross-owner isolation test. Left behind two gaps, both still open: **R35** — `artifact/fetch` reads
and hashes the full content *before* the ownership check runs, a timing side-channel distinguishing
"exists but not yours" from "doesn't exist" by latency alone; and **R36** — the isolation tests
hand-seed `run_id` on their fixtures rather than exercising the real producers
(`WorkspaceApplier`/`WorkspaceInspector`), so reverting the producers' own stamping code back to
`run_id: None` would leave the entire test suite green.

**R11** — Copilot's vendor turn-stop reasons were discarded outright instead of being normalized
into protocol health/failure signals. Fixed (`bcff4ce`) via `copilot_normalize_stop_reason()`,
mapping every stop reason to a `ProtocolHealthChanged` event and a failure disposition, defended by
eight unit tests. Left behind: **R42** — the unknown-reason arm's detail string interpolates the
already-lowercased, `_`/`-`-stripped match binding instead of the original vendor `stop_reason`
text, so the one piece of diagnostic detail meant to help someone grep vendor docs for an
unrecognized reason has already been mangled past matching them.

**R47** — Claude and Codex adapters never emitted `ProcessExited`, so their concurrency slots leaked
on every completed run, permanently disabling the runtime after `concurrency_ceiling` cumulative
runs (the exact failure mode R2 closed for the mechanism as a whole, open again for two of the
four adapters). Fixed across five steps: added `TerminationOutcome::exit_signals()` and
`ManagedProcess::settle()` to the supervisor (`supervisor/process.rs`); Claude's `run_session` now
yields an outcome from all three break arms and emits `ProcessExited` after cleanup
(`adapter/claude/mod.rs`); Codex's `driver_loop` carries the exit through `InboundMessage` to the
pump (`adapter/codex/client.rs`), `spawn_pump` emits `ProcessExited` and leaves the loop, and both
`cancel` and `dispose` were fixed to not abort the pump before it reports (`adapter/codex/mod.rs`);
OMP-RPC's `run_pump` now emits `ProcessExited` on its terminate arm, not just stdout-closed
(`adapter/omp_rpc/mod.rs`); and the registry's completion watcher was replaced with
`SettlementSink` — a per-run oneshot that fires on the first `ProcessExited`, immune to broadcast
lag or late subscription (`adapter/event_sink.rs`, `adapter/registry.rs`). The registry's old
`is_process_exited_for` was deleted. Defended by new tests: `settle_reports_a_self_exit_code_without_escalating`
and `settle_escalates_a_process_that_will_not_exit_on_its_own` (`tests/supervisor.rs`),
`session_exit_tests` (`adapter/claude/mod.rs`), `pump_exit_tests` (`adapter/codex/mod.rs`), and
`settlement_tests` / `settlement_sink_tests` (`adapter/registry.rs`, `adapter/event_sink.rs`) — the
former using a real `DatabaseHandle::start()` harness with `tempfile::TempDir` so the DB actor
persists through the test. `PolicyEvaluator::release()` saturates at zero, so a double release is
safe, and the oneshot's exactly-once semantics guarantee the slot releases precisely once per run.
A full end-to-end integration test driving a Claude or Codex run through the real registry's
completion watcher and asserting the concurrency slot is returned does not yet exist — the existing
component tests prove emission and the mocked release path, but the integration gap remains.

### The documentation half: eleven doc-accuracy findings, most already stale on arrival

The same first review round filed eleven Low-severity documentation findings (R19, R21-R28) —
CLI flags that didn't exist, tool counts that were wrong, deleted modules still named, an installer
Homebrew never had. By the time each was re-verified on 2026-08-08, six (R19, R23, R24, R26, R27,
R28) had already been corrected by unrelated doc work and needed nothing further — recorded as
resolved on the strength of re-reading the current text, not a fix commit filed against this
review. Two (R21, R22) had regressed independently into `AGENTS.md`/`CLAUDE.md` after those files
were generated later than the original doc fixes — corrected in place during the same
consolidation that produced this journal entry's predecessor commits. **R25** went further than
"resolved": `release/0.1.0-checklist.json`, the file the finding was filed against, was deleted
outright in `7ab1447` rather than merely relabeled — re-verification on 2026-08-10 confirmed there
was nothing left to fix.

### Where this history lives now

`REVIEW.md` itself was restructured on 2026-08-12 from a full audit trail (every finding, resolved
or not, with its evidence) into an open-items-only backlog — R1-R11, R19, and R21-R28 no longer
appear there at all. R47 joined that list the same day it was resolved, pruned from `REVIEW.md`
and recorded here. This journal entry is now the only place resolution evidence for R1-R11 and
R47 is recorded; if you're looking for *why* R34, R35, R36, R39, R42, R44, or R67 exist, the
answer in every case is "as a byproduct of a fix directly above it in this entry." That same
2026-08-12 pass also ran a full fresh re-verification of every item that *was* still open, adding
twenty new findings (R47-R66) surfaced by reading the runtime core, the adapters, the conformance
harness, the TS extension, and the release/docs surface with fresh eyes — the most severe of them
the most severe of them (R47-R49) initially sitting at Critical, though R47 was resolved the same
day it was found, and R48 itself resolved the next day (Part XI), leaving R49 alone at Critical.
R67 retains the integration-coverage residue — a reminder that a review closing its filed findings
is not the same claim as a system having no more bugs.

## Part XI — Halving the Critical pair: a ceiling that could not be enforced

Part X closed with R48 and R49 as the remaining Critical pair. R48 is now closed, and its shape is
worth recording because nothing about it was visible from the code that appeared to implement the
feature. Every piece of per-run cost enforcement existed and was wired: `config/merge.rs` read
`cost.ceiling_per_run_usd`, `policy/evaluate.rs` refused to authorize a run whose adapter could not
report usage (so the ceiling could never be silently unmeasurable), `AdapterRegistry` threaded the
ceiling into each run's `DomainAdapterEventSink`, and the sink accumulated `UsageReported.cost_usd`
and fired exactly once on the crossing event. The one thing that could not happen was the write.

`MIGRATION_4` had declared `policy_violations.vendor_child_id` and `vendor_parent_ref` `NOT NULL`,
back when a nested worker was the only kind of violation. A cost ceiling has no vendor child, and
`record_cost_ceiling` correctly journals both as `None` — which bound as SQL `NULL` and failed the
constraint on every single crossing. Because the insert is the first thing `record_cost_ceiling` does
and its error propagates with `?`, `apply_action` — the code that quarantines or cancels the run —
never ran at all, and the sole caller in `event_sink.rs` only logged a warning. A run could spend
without limit while the runtime reported nothing but one warn line.

Fixed by `MIGRATION_8`, a table rebuild (SQLite cannot drop a column constraint in place) that makes
both vendor columns nullable and preserves every existing row: an absent vendor child is now recorded
as an absence, matching what the code and the event payload already said. The sentinel-empty-string
alternative was rejected for the reason `record_policy_violation`'s own doc comment gives — an empty
id would be a lie rather than an absence.

The gap was as much a testing gap as a schema one: before this fix, nothing anywhere in the tree
touched the `policy_violations` table other than the migration that created it. Three tests now
defend it. `migration_8_makes_vendor_refs_nullable_and_preserves_existing_rows` (`db/migrations.rs`)
migrates to version 7, proves the old schema rejects the NULL insert, migrates to 8, and proves the
pre-existing row survived, that the `action`/`created_at` constraints did not get dropped along the
way, and that the resolution columns still work. `record_policy_violation_persists_absent_vendor_refs_as_null`
(`domain/repository.rs`) proves the repository writes real SQL NULLs against the production migration
list with foreign keys on. And `crossing_the_per_run_cost_ceiling_records_an_actionable_violation`
(`tests/orchestration_rpc.rs`) drives a full run whose adapter reports $2.50 against a $1.00 ceiling
and asserts the run comes back quarantined, the journaled event carries `cost_ceiling_exceeded` with
null vendor refs, and `policy/violation/decide` can release it — that last step being the one that
proves the projection row exists, since `decide` reads it rather than the journal.

R49 remains open: the built-in `api_key` redaction pattern still does not match Anthropic's own
`sk-ant-api03-…` key shape.

## Reading order, if you're new here

If you're going to *use* BATMAN, not build or maintain it, skip this journal entirely and start
with [`plugin-usage.md`](plugin-usage.md) — the user manual. Everything below is for someone
contributing to or maintaining the codebase itself.

1. **README.md** — what this is, in two paragraphs.
2. **This journal** — how it got to be that, commit by commit.
3. **`docs/adr/`** — the decisions that outlived their commit, in a form built to survive being
   read out of context.
4. **architecture.md** — the finished design, with no history in it at all.
5. **[`getting-started.md`](getting-started.md)** — the developer manual: build, configure, test.
6. **code-walkthrough.md** — how to find anything, trace a request, and debug it.
7. **rust-primer.md** — if Rust itself is still new, read this alongside the journal; every "Day"
   in the primer is the concept behind one of the commits above.
8. **manual-testing.md** — every live/manual verification step this journal references by name,
   runnable, including the environment variables each worker adapter's live suite gates on.
9. **engineering-lessons.md** — the specific bugs this journal narrates as history, indexed by
   file/ADR instead of by commit, for when you're debugging something that feels familiar.
10. **operations.md** / **cli-reference.md** / **compatibility.md** — day-to-day references once
    you're past onboarding: running `batcave` by hand, its full flag set, and what's actually
    proven to work against which platform/adapter version.
