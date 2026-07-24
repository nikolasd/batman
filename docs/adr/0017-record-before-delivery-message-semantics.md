# Record-before-delivery message semantics

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

Worker-safe messages (`assign`, `steer`, `question`, `cancel`, and others) can be interrupted by a
crash between "the runtime decided to deliver this" and "delivery actually happened." What should
happen to a message caught in that window, and should the runtime ever resend automatically to
resolve the uncertainty?

## Decision Drivers

* An unwitnessed, automatic duplicate delivery of a message like `cancel` or `assign` is a worse
  failure mode than an honestly reported "delivery outcome unknown."
* Every attempted message needs a full audit trail, independent of whether delivery ultimately
  succeeded.
* Recovery after a crash must be deterministic and not depend on remembering in-flight state that
  didn't survive the crash.

## Considered Options

* Record-before-delivery: commit the message as `recorded` (one durable event, one projection row)
  *before* attempting delivery, then commit the outcome (`sent`, later `acknowledged`/`failed` once
  a real adapter exists) as a second event. A crash between the two commits leaves the message
  `recorded`/`sent`; a startup sweep (`sweep_unacknowledged_as_unknown`) settles anything left in a
  non-terminal delivery state to `unknown`, and never resends automatically.
* At-least-once delivery: automatically retry until acknowledged, accepting the risk of duplicate
  delivery.
* Fire-and-forget: no delivery state tracked at all.

## Decision Outcome

Chosen option: record-before-delivery, with an explicit `unknown` terminal state for the
crash-window case and no automatic resend, ever.

### Positive Consequences

* No silent duplicate side effects — a message the runtime isn't certain was delivered is reported
  as such, not retried into possibly happening twice.
* A complete audit trail exists for every message the runtime ever attempted, regardless of
  outcome.
* Recovery is deterministic: the startup sweep is the *only* place a stuck delivery state changes,
  and it always moves toward `unknown`, never toward a guessed success or a silent retry.

### Negative Consequences

* A message stuck at `unknown` requires the caller (ultimately OMP) to notice and decide whether
  to resend — as a genuinely *new* message, with its own new identity — rather than the runtime
  resolving the ambiguity on its own. This pushes a real decision back to the layer that has the
  context (and authority, per ADR-0011) to make it correctly.

## Pros and Cons of the Options

### Record-before-delivery, no auto-resend (chosen)

* Good, because it never manufactures a duplicate side effect the caller didn't ask for.
* Bad, because it leaves recovery from `unknown` as someone else's explicit job.

### At-least-once with automatic retry

* Good, because it would resolve transient failures without any caller involvement.
* Bad, because for messages like `cancel` or `approvalDecision`, an automatic duplicate is not a
  harmless retry — it's a second, unwitnessed instance of an action that may have real
  consequences the first time it lands.

### Fire-and-forget

* Good, because it would be the simplest possible implementation.
* Bad, because it provides no audit trail and no way to ever detect, let alone recover from, a
  message lost to a crash.

## Links

* Narrated in `../journal.md`, commit `3172d99`
