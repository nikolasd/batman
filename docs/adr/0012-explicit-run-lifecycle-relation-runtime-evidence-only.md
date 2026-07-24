# Explicit run-lifecycle relation, applied only on runtime evidence

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

A `Run`'s state (`queued`, `starting`, `working`, ...) must be trustworthy enough that anything
reading it — OMP, a human watching `/batman`, an approval decision — can rely on it meaning exactly
what it says. If any caller could set a run to `working` by simply asking, the state would mean
"someone claimed this," not "this actually happened." How should run-state transitions be
represented and enforced so that reported state and real state can never diverge?

## Decision Drivers

* OMP may request cancellation, but must never directly flip a run's state — only the runtime,
  having observed real process or protocol evidence, should be able to.
* An ad hoc scattering of `if current == "queued" && requested == "starting"` checks is easy to
  get *mostly* right and very hard to prove *exhaustively* right.
* The plan calls out no generic pause/resume: entering `paused` and leaving it both need
  correlated adapter evidence, not a bare command.

## Considered Options

* An explicit relation — `RunState::can_transition_to(&self, target: &RunState) -> bool` — defined
  once from a full table of legal edges, with every self-transition and every edge out of a
  terminal state (`succeeded`/`failed`/`cancelled`/`lost`) illegal by omission; enforced by
  `domain/transitions.rs::check_transition` *before* any event is appended, so an illegal edge
  appends nothing at all.
* An OMP-writable state field, with the runtime only recording a history of what OMP claimed.
* Transitions implicitly derived from other data (e.g., inferring `working` because a message was
  recently sent on the run), with no explicit relation at all.

## Decision Outcome

Chosen option: the explicit relation. Ten states, an exhaustive table of legal edges
(`crates/protocol/tests/domain_contract.rs` checks all 28 legal edges and 26 illegal ones,
including every self-transition and every terminal-state exit), and one enforcement point that
runs before any event is durably appended.

### Positive Consequences

* The lifecycle is provably, not just believably, correct — a new engineer can read one table and
  one test file and know exactly what's legal, with no scattered logic to reassemble mentally.
* An illegal transition is caught *before* anything is journaled, so a bug in a caller can never
  produce a half-applied state change.
* OMP requesting cancellation and the runtime later applying `working -> cancelled` after real
  evidence are two distinct, separately auditable events — the request is never mistaken for the
  fact.

### Negative Consequences

* There is no generic pause/resume: any future feature wanting a pause-like state must extend the
  table deliberately (and, per ADR-0002, regenerate bindings and re-verify the fixture), not bolt
  one on ad hoc.
* The table is a permanent contract the moment it ships — removing a legal edge later is a
  backward-incompatible change for anything relying on it, same as any other protocol surface.

## Pros and Cons of the Options

### Explicit relation, evidence-gated (chosen)

* Good, because it's exhaustively testable and impossible to half-apply.
* Bad, because every new lifecycle need must be modeled as a table extension, which is more
  upfront design work than an inline check would be.

### OMP-writable state field

* Good, because it would be the simplest possible implementation.
* Bad, because it makes "the run is working" mean "OMP believes the run is working," which is
  exactly the ambiguity this decision exists to remove — and it directly contradicts ADR-0011's
  "only runtime evidence changes lifecycle state."

### Implicit/derived transitions

* Good, because it would need no explicit state field to maintain at all.
* Bad, because "derived from other data" is a state machine with its rules hidden across whatever
  other data happens to exist — undebuggable and untestable as a whole.

## Links

* Narrated in `../journal.md`, commit `3d604af`
* Enforces [ADR-0011](0011-omp-retains-task-graph-authority.md)
