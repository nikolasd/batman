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
