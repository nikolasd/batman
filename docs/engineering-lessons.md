# Engineering Lessons

**Audience & purpose:** contributors debugging something that feels like it might have happened
before — a companion to [code-walkthrough.md](code-walkthrough.md)'s debugging playbook and the
developer manual ([getting-started.md](getting-started.md)). This document catalogs hard-won
lessons from smoke testing, production incidents, and debugging. These are the kind of things that
should be discovered by reading documentation, not by trial and error.

**Reference:** These lessons are cross-referenced by file/ADR, not by `architecture.md` section number — that document was later rewritten onto the C4 model and no longer has numbered `§N` sections.

---

## IPC and Client Management

### Cached client must authenticate with the union of all roles

**Location:** `packages/extension/src/runtime.ts::ensureRuntime` (see [ADR-0021](adr/0021-shared-client-authenticates-with-the-union-of-required-roles.md))

A cached client shared across callers with different role needs must authenticate with the *union* of every role its callers need, not whatever the first caller happened to need.

**The bug:** `ensureRuntime` originally hardcoded `ClientAuth::Display` (read-only) for what its own doc comment called "a launcher connection" — correct for `batman_status`, the only caller when it was written. `index.ts::getClient()` later cached and reused that exact client for every orchestration tool too, so every mutation (`task/upsert`, `run/submit`, ...) failed `-32601 method ... is not available to this client` — silently, since `display`'s method table is a strict *subset* of `ompExtension`'s and nothing checks that relationship at compile time.

**The fix:** `ensureRuntime` now authenticates as `ompExtension` unconditionally, safe for every existing caller because `ompExtension`'s allowed methods are always a superset.

**The lesson:** When one cached connection is shared across callers with different needs, its role must be the *union*, not whatever the first caller happened to need — and a role table's superset/subset relationships between roles are exactly the kind of fact that belongs in a comment next to the role definition, not just in reviewers' heads.

---

## Extension Loading and Module Resolution

### Never use `with { type: "json" }` imports at extension-load time

**Location:** `packages/extension/src/monitor/compat.ts`

A static `import ... with { type: "json" }` at module scope can hang the extension or corrupt `bun test`.

**The bug:** `compat.ts` originally did `import pkg from "@oh-my-pi/pi-coding-agent/package.json" with { type: "json" }` at module scope, called from `registerMonitor` at extension-load time. That import resolves fine under `bun run`/`bun test` in this repo's own `node_modules`, but **hangs forever** the instant the real `omp` binary (itself a compiled, bundled Bun executable) loads the extension file and tries to resolve that exact subpath from *its own* bundled module graph — confirmed by bisecting a minimal repro down to the bare import statement with no call.

**The fix:** The version check now lives only in a test (`render.test.ts`), matching the plan's own framing of it as a "no-model fixture", never called from production code; and the check itself now reads the peer's `package.json` via a plain filesystem walk from `import.meta.dir`, never through Bun's module resolver.

**The lesson:** Never run a `with { type: "json" }` import — or any dynamic resolution of a peer package's own metadata — at extension-load time or module scope in code the real `omp` binary will load; if you need a peer's installed version, read the file directly.

---

## Persistence and Event Broadcasting

### Durable mutations must broadcast the same event they just committed

**Location:** `crates/runtime/src/domain/repository.rs::append_and_apply`, `crates/runtime/src/ipc/connection.rs::replay` (see [ADR-0020](adr/0020-per-mutation-event-broadcast-is-not-optional.md))

A durable mutation must broadcast the same event it just committed, in the same call.

**The bugs (two separate issues):**

1. **Full vs. bare envelope:** `DomainRepository::append_and_apply` stored the *full* `EventEnvelope` (with `sequence`, `timestamp`, ...) into `event_json`, but `ipc/connection.rs::replay()` expects that column to hold only the bare `RuntimeEvent` — it reconstructs the envelope from the `events` table's own `sequence`/`timestamp`/`project_id`/`run_id` columns. Every `events/replay` call therefore failed to deserialize once any mutation had committed.

2. **No publisher for broadcast channel:** `Shared.events_tx` (the `tokio::sync::broadcast` channel `ipc/connection.rs::spawn_subscription` reads from) had a subscriber but **no publisher anywhere** — none of the 15+ mutation call sites across `OrchestrationService`, `ApprovalService`, `CoordinationBroker`, and `RunDriverContext` ever called `.send()` on it. A monitor connected before a mutation committed would never observe it without reconnecting (which re-triggers `events/replay` — itself broken by the first bug).

**The fix:** Storage now writes the bare event; `Committed` now carries the full `EventEnvelope`; and `domain::{embed_envelope, take_envelope}` smuggle it across the `run_domain_op` closure boundary (whose closures are constrained to return a plain `serde_json::Value`) so every service broadcasts after every commit.

**The lesson:** This is now invariant #7 in the README, and it is not enforced by the type system — the compiler cannot catch "this new mutation appended an event but forgot to broadcast it". Any new `DomainRepository` mutation method **must** be wired through `embed_envelope`/`take_envelope`/`self.broadcast(&mut result)` at its call site, matching every existing sibling in `service/orchestration.rs`, or the monitor will silently show stale state for that mutation alone.

**Regression tests:** `crates/runtime/tests/orchestration_rpc.rs`'s `events_replay_round_trips_committed_mutation_events` and `events_subscribe_delivers_live_notifications_for_orchestration_mutations` are the regression tests for both halves; the latter reproduced the bug as an infinite hang, not a clean failure — run it with a test-runner timeout if you ever suspect a new mutation has regressed this.

---

## Run Lifecycle

### A documented state machine with no production writer is inert

**Location:** `crates/runtime/src/adapter/registry.rs`, `crates/runtime/src/adapter/run_lifecycle.rs` (see [ADR-0023](adr/0023-run-state-edges-from-adapter-evidence.md))

ADR-0012 defined the run-lifecycle relation and ADR-0013 shipped `FakeRunDriver` as its only
implementer. The real `AdapterRegistry` — the production `RunDriver` — never called
`transition_run` anywhere in `run_one` or `watch_settlement`; grepping `crates/runtime/src/adapter/`
for `transition_run` returned zero hits. Every real run's row stayed `queued` however successfully
its vendor process ran and exited; `run/get`, `run/list`, the `/batman` monitor, and the approval
flow all read a value that was wrong for every real run, and only a daemon restart
(`RecoveryCoordinator`) ever terminalized anything.

**The lesson:** A state machine whose only exerciser is a test fake reads as implemented in
review — the `FakeRunDriver`-only `queued -> starting -> working` sequence looked like coverage of
a real path, because the relation itself (`RunState::can_transition_to`) was thoroughly tested and
the fake drove it end to end. It wasn't: the fake is never wired into a live `omp` session. Grep
for production call sites of the transition function itself, not for the transition table or its
unit tests — a well-tested relation with zero production callers is exactly as broken as an
untested one.

**The fix:** `RunLifecycleSink` wraps each run's `AdapterEventSink` and applies the evidence table
(`ProcessStarted` -> `starting`, first non-exit payload -> `working`, `ProcessExited` ->
`succeeded`/`failed`/`lost`) after the inner sink journals each event, walking every intermediate
hop the legal-edge table forces and never overwriting a terminal state.

**Regression tests:** `crates/runtime/src/adapter/run_lifecycle.rs`'s 9 unit tests
(`process_started_moves_a_queued_run_to_starting` through
`vendor_output_never_reopens_working_on_a_run_that_started_waiting`), plus the end-to-end proofs
against real processes: `crates/runtime/tests/run_lifecycle.rs`'s
`a_real_worker_process_walks_its_run_from_queued_into_working` and
`a_real_worker_process_exit_settles_its_run`, `crates/runtime/src/adapter/claude/mod.rs`'s
`run_state_tests` module, and `crates/runtime/tests/copilot_adapter.rs`'s
`a_supervised_process_exit_is_reported_with_its_real_status`.

---

## Workspace Leases and Resource Cleanup

### A resource acquired before a fallible step must be released on every path out of it

**Location:** `crates/runtime/src/service/orchestration.rs::start_queued_run`, `::workspace_acquire`; `crates/runtime/src/workspace/lease.rs::stale`

Two-phase acquisition (claim first, then do the fallible work that finishes the claim) makes the
in-between state invisible to any check written only against the *finished* state's shape.

**The bugs (two distinct triggers of one defect, closed together):** `start_queued_run` and
`workspace_acquire` each acquire a workspace lease (an `allocating`-state row), then materialize a
worktree or copy, then activate the lease with the real path, and — for `start_queued_run` only —
start the adapter. Nothing released the lease on a failure in any of the fallible steps after
`acquire`. `materialize()` failing left an `allocating` row nothing would ever touch again; a
`driver.start` failure left an `active` lease and a real worktree with no owner. `run/retry`
re-runs the whole sequence for a new `RunId`, so a driver that reliably failed to start leaked one
row per retry. The `materialize()`-failure case was worse than the `driver.start` case: the only
check meant to catch it, `LeaseService::stale()`, filtered on "a non-empty path that no longer
exists" — a signal an `allocating` row can never produce, since its path is empty by construction
until `activate()` runs. The doctor check written for exactly this residue was structurally blind to
it, not merely untested against it.

**The fix:** `abandon_lease`/`abandon_and_announce` helpers now run on every fallible step past
`acquire()` in both functions, mirroring `workspace_release`'s existing release-then-teardown-then-
`cleanupFailed` ordering rather than inventing a second convention. Teardown is deliberately
best-effort: propagating a `git worktree remove` failure (expected when the worktree was never
created) would replace the caller's real error with an unrelated cleanup artifact. `stale()` was
widened to also flag any row still `allocating` past `ALLOCATING_LEASE_GRACE` (ten minutes)
regardless of path emptiness, so a lease abandoned before materialization even started is no longer
invisible to the doctor.

**The lesson:** A doctor-style health check keyed only on the *finished* shape of a resource (here,
"a real path that vanished") cannot see a failure that happens before that shape ever exists. When a
resource is claimed in more than one commit, write the check against the intermediate state
directly — an age threshold on the claim itself, here — not only against the terminal one.

**Regression tests:** `crates/runtime/tests/orchestration_rpc.rs`'s
`start_queued_run_releases_the_lease_when_materialize_fails`,
`workspace_acquire_releases_the_lease_when_materialize_fails`, and
`start_queued_run_releases_the_lease_and_worktree_when_driver_start_fails`;
`crates/runtime/tests/workspace_lease.rs`'s
`stale_never_flags_an_allocating_lease_within_the_grace_period` and
`stale_flags_an_allocating_lease_that_outlived_the_grace_period`;
`crates/runtime/tests/doctor.rs`'s
`stale_workspaces_fails_when_an_allocating_lease_outlives_the_grace_period`.

---

## Security and Redaction

### A redaction denylist is only as good as the shapes it was actually tested against

**Location:** `crates/runtime/src/security/redaction.rs::Redactor::new` (see [ADR-0006](adr/0006-type-enforced-redaction-boundary.md))

A pattern that looks like it covers a vendor's API keys is worthless if it was written against a
remembered key format rather than the one that vendor issues.

**The bug:** the built-in `api_key` rule was `sk-[A-Za-z0-9]{16,}` — a plausible-looking `sk-`
pattern that matched none of the keys the vendors this codebase drives actually issue. Anthropic's
`sk-ant-api03-…` and OpenAI's `sk-proj-…` both put hyphens (and base64url underscores) inside the
token, immediately after the three characters the pattern accepted. Every unit test asserting the
rule worked used a hand-written `sk-ABCDEFGHIJKLMNOPQRSTUVWX` literal that shared the pattern's
assumption, so the whole test suite agreed with the bug. Classification (`Secret`/`Thinking`
fragments) is the primary boundary and was unaffected — but the denylist exists precisely for the
case where a vendor narrates a key back inside `Visible` text, which is exactly what it could not
catch.

**The fix:** `(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{16,}`, with tests written from the vendors'
documented key shapes rather than from the pattern. Constraining what may *precede* the token is
load-bearing in the other direction: the widened character class accepts `-`, so an unconstrained
version swallows ordinary hyphenated prose — `disk-space-check-failed` contains a legal
`sk-space-check-failed`. The first attempt used a leading `\b`, which is **not** sufficient: `-` is
a non-word character, so `\b` still admits `pre-sk-space-check-failed`. The preceding character has
to be matched and constrained to something outside the token alphabet, then re-emitted through the
rule's `${1}` replacement so the surrounding text is not eaten with the secret.

**The lesson:** when a redaction/denylist pattern is added or widened, the test input must come from
the real producer's format, never from the pattern's own shape — and every widening needs a
paired negative test proving normal text is still untouched, because over-redaction of diagnostics
is a silent failure too. `\b` in particular is a trap when the token alphabet contains `-` or `_`:
it asserts a *word* boundary, which those characters do not create.

**Regression tests:** `anthropic_shaped_api_key_is_redacted`,
`openai_project_shaped_api_key_is_redacted`,
`hyphenated_prose_is_not_mistaken_for_an_api_key`,
`hyphen_delimited_prose_is_not_mistaken_for_an_api_key`,
`two_adjacent_api_keys_are_both_redacted`,
`sanitize_json_redacts_an_anthropic_shaped_key_at_any_depth` (all in `security/redaction.rs`), plus
`crates/runtime/tests/redaction_boundary.rs`, which carries an Anthropic-shaped key through the real
append path and byte-scans the database, WAL, log, and replay output for it.

---

## Coordination Bounds

### A bound enforced at one call site is not an enforced policy

**Location:** `crates/runtime/src/coordination/broker.rs::{send, request_child, publish_artifact}`

A doc comment that asserts a broker-wide invariant is only as load-bearing as the single inline
check it was written beside — and a second, stricter enforcement layer can hide its absence from
every surface a user actually exercises.

**The bug:** the byte bound and the per-sender rate limit lived inline in `send()` while the
struct's own doc comment described them as properties of the whole broker. The two methods added
later — `request_child()` and `publish_artifact()` — inherited the claim without the code: the
direct JSON-RPC path had no size bound at all (the server's default 4 MiB frame cap was the only
bound in sight), and `publish_artifact`'s journaled message could be looped without throttling of
any kind. The MCP tool surface had its own stricter argument bounds, so every test that drove
through that layer saw a broker that *looked* bounded; the gap was invisible from the tool
surface.

**The fix:** `reject_oversized()` and `charge_rate_limit()` as named helpers on
`CoordinationBroker` that every journaling method calls — the byte bound on each worker-supplied
string that can become durable content, and the per-sender rate-limit charge as soon as the
sender's identity is resolved. The rate-limit key is the run's own `worker_id` row read through
`run_participants()`, never a caller-supplied parameter, so a single shared window covers `send`,
`requestChild`, and `publishArtifact` alike and a worker cannot evade it by rotating between
methods. Quarantine keeps its position ahead of the rate-limit charge so a quarantined worker
still sees `POLICY_QUARANTINED`, not `RATE_LIMITED`.

**The lesson:** when a doc comment on a type asserts an invariant, the enforcement belongs in a
named helper the type's methods must call, not inline in the first method that needed it — and a
second enforcement layer (here `mcp_protocol`'s stricter argument bounds) can hide the absence of
the first from every test that only drives the outer layer. Test the innermost layer directly.

**Regression tests:** `crates/runtime/tests/coordination.rs`'s
`coordination_request_child_rejects_a_reason_over_64_kib`,
`coordination_publish_artifact_rejects_free_text_over_64_kib`,
`coordination_publish_artifact_accepts_a_description_at_the_limit`,
`coordination_publish_artifact_draws_on_the_same_per_sender_budget_as_send`, and
`coordination_request_child_draws_on_the_same_per_sender_budget_as_send`.
