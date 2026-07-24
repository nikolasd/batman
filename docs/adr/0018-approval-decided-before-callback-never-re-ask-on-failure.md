# Approval decided before callback; never re-ask on failure

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

When a human (or OMP, on a human's behalf) decides an approval, the runtime records the decision
and then invokes an adapter callback to acknowledge it to the waiting vendor process. What should
happen if that callback itself fails — is the decision retried, is the human asked again, or does
something else happen?

## Decision Drivers

* Asking a human to approve or deny the same action twice, under two different approval IDs, is a
  real audit and trust problem ("did I actually agree to this, or did the system just ask me
  twice because of a plumbing failure?").
* A decision, once made, is a fact — it shouldn't be erased or superseded just because the
  *notification* of it had trouble.
* A broken adapter callback is a signal worth surfacing durably, not silently swallowing or
  silently retrying forever.

## Considered Options

* Record the decision first, then invoke the callback. On success, transition the run back to
  `working`. On failure, keep the decision exactly as recorded and mark the run
  `protocolUnhealthy` instead of asking again.
* Don't record the decision until the callback succeeds — retry the whole request/decide flow on
  failure.
* Record the decision, then retry the callback automatically with backoff, without exposing
  failure to anything outside the retry loop.

## Decision Outcome

Chosen option: record first, callback second, and on failure keep the decision while flagging
`protocolUnhealthy` — never re-ask. `ApprovalService::decide`'s ownership check
(only the connected principal whose `instanceId` currently owns the task may decide) and
idempotency check (an identical repeat decision is a silent no-op, never re-invoking the callback)
both feed into this same discipline: a decision is made exactly once, and everything after that is
about *communicating* it, not remaking it.

### Positive Consequences

* A human is never asked to approve or deny the same action twice.
* `protocolUnhealthy` is a durable, queryable signal that the *plumbing* (not the policy decision)
  is broken — exactly the information whoever operates the adapter registry needs to go fix the
  right thing.
* Idempotency and ownership checks compose cleanly with this rule: repeating an already-recorded
  decision is always safe, because the decision itself never changes after it's first recorded.

### Negative Consequences

* If a callback failure was transient (a momentary network blip to the vendor process, say), there
  is no automatic self-healing built into this milestone — a human or operator must notice
  `protocolUnhealthy` and intervene, or a later adapter-registry milestone must design its own
  retry story explicitly on top of this base guarantee.

## Pros and Cons of the Options

### Record first, no re-ask on callback failure (chosen)

* Good, because the human's decision is durable and asked for exactly once.
* Bad, because recovering from a stuck `protocolUnhealthy` run currently requires manual
  intervention.

### Don't record until callback succeeds

* Good, because it would avoid ever exposing a "kept decision, broken callback" intermediate
  state.
* Bad, because it means a transient callback failure could cause the *decision itself* to be
  retried — silently reopening a question a human believed they'd already answered.

### Automatic retry with backoff, hidden from callers

* Good, because it could resolve transient failures without any visible intermediate state.
* Bad, because a retry loop hidden from the caller is a retry loop nobody can observe or reason
  about — exactly the kind of silent recovery-that-might-fail-anyway this project's "no automatic
  resend" instinct (see ADR-0017) argues against.

## Links

* Narrated in `../journal.md`, commit `534d3db`
* Shares its no-automatic-resend philosophy with [ADR-0017](0017-record-before-delivery-message-semantics.md)
