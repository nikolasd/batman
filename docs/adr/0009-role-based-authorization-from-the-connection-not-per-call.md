# Role-based authorization from the connection, not per call

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

Three distinct kinds of caller connect to the daemon over the same socket: the OMP extension
itself (`ompExtension`), a supervised worker process (`workerMcp`), and a read-only display client
(`display`). Each needs a different, precisely bounded set of callable methods — and every
milestone after Foundation adds more methods, most of which only one or two of these roles should
ever see. How should method-level authorization be structured so that adding a new method never
risks accidentally widening who can call it?

## Decision Drivers

* Unauthorized access must be indistinguishable from a nonexistent method (`METHOD_NOT_FOUND` for
  both) — no information leak about what a caller *could* call if it were a different role.
* Adding a new method to one role's surface should be a small, obviously-correct code change, not
  a scattered set of individual permission checks that could each be gotten wrong independently.
* The authorization decision must be based on who the connection *authenticated as*, never on
  anything the client claims per-call.

## Considered Options

* Authenticate once (via `ClientAuth` at `initialize`) into a `ClientPrincipal`; dispatch consults
  `ClientPrincipal::allowed_methods()`, one function returning the exact method list for that
  role, before ever routing to a handler.
* Per-method inline permission checks, written at each handler as it's added.
* Capability tokens issued per method call, checked independently of any connection-level identity.

## Decision Outcome

Chosen option: authenticate once, authorize from a per-role table. `ClientAuth` is a role-tagged
enum with disjoint required fields per role; `authenticate()` turns it into a `ClientPrincipal`;
`allowed_methods()` returns that role's exact method list, and dispatch checks membership in that
list before routing to any handler. A method outside the caller's table returns
`METHOD_NOT_FOUND` — the same code a genuinely nonexistent method would return.

### Positive Consequences

* Adding a method to a role's surface is one line in one function (`allowed_methods`), reviewable
  at a glance against the other roles' lists in the same place.
* No information leak: a `workerMcp` connection probing for `task/upsert` gets exactly the same
  response as probing for a method that doesn't exist at all.
* The read/write asymmetry layered on top of this table — project-scoped reads open to any
  same-user client, ownership gating only mutation — is decided separately in
  [ADR-0024](0024-project-scoped-reads-are-open-ownership-gates-writes.md).
* Every later milestone (orchestration, coordination) extended this same table rather than
  inventing a new authorization mechanism — the pattern held for eighteen new methods across three
  role tables without modification.

### Negative Consequences

* The table itself is the single point of truth and must be kept accurate — a role gaining access
  it shouldn't have is now a one-line diff to notice in review, but it is still possible to write
  that line. Mitigated by tests that assert the *exact* expected method list per role (not just
  "some subset").

## Pros and Cons of the Options

### Authenticate once, authorize from a table (chosen)

* Good, because it centralizes the decision and makes it exhaustively testable per role.
* Bad, because the table is hand-maintained and must be reviewed carefully on every method
  addition.

### Per-method inline checks

* Good, because each check lives right next to the logic it protects.
* Bad, because "right next to the logic" also means "as easy to forget as the logic itself is easy
  to add," and there is no single place to audit "what can a `workerMcp` connection do."

### Capability tokens per call

* Good, because it decouples authorization from connection identity entirely, which could be
  useful for finer-grained, revocable per-call grants.
* Bad, because it is significantly more machinery than three coarse-grained roles need, and this
  project already has a narrower, purpose-built token mechanism (see ADR-0016) for the one case —
  worker coordination — where per-call scoping actually matters.

## Links

* Narrated in `../journal.md`, commit `4ed1b14`
* Extended by every orchestration/coordination method added in commits 13, 20, and 21
