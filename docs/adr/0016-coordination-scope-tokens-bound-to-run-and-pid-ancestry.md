# Coordination scope tokens bound to run identity and PID ancestry

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

A supervised vendor process needs a worker-safe messaging surface (`coordination/*`) distinct from
the extension's own `ompExtension` authority — Foundation deliberately shipped
`RejectAllWorkerVerifier` as the default specifically because nothing yet existed worth trusting a
`workerMcp` connection with. Now that a worker-safe surface exists, what should actually establish
that a connecting process is the *specific* vendor process launched for a *specific* run, scoped
tightly enough that leaking or reusing a credential can't grant broader access?

## Decision Drivers

* Trust must be scoped to one run, one worker, one launched vendor process — not a general
  "this is *a* worker" credential reusable across runs.
* Token material must never be persisted or logged, even by accident.
* Where the platform can't establish trustworthy peer-process identity, the system must say so
  explicitly rather than accept an unverifiable connection.
* A supervised process restarting (e.g., an MCP subprocess crash-and-relaunch within the same
  process tree) should be able to reconnect with the same token, not be locked out by a
  once-only-use restriction.

## Considered Options

* A minted `ScopeTokenStore` token bound to `{ projectId, taskId, workerId, runId,
  vendorProcessIdentity, expiresAt }`; verification checks the run binding, expiry, and that the
  connecting peer's PID is a live descendant of the recorded vendor process
  (`PidAncestryChecker`), with an explicit `Unsupported` result on platforms lacking trustworthy
  peer-process identity.
* A static shared secret configured per worker, checked on every connection.
* mTLS client certificates issued per worker process.

## Decision Outcome

Chosen option: minted, run-scoped tokens verified against PID ancestry. Token bytes exist only as
the key in an in-memory `HashMap` inside `ScopeTokenStore` — never journaled, never logged, never
present in a `Debug` output. `ScopeTokenVerifier` adapts the store to the same
`WorkerCredentialVerifier` seam Foundation already defined, so this milestone slots into the
existing role-based authorization design (ADR-0009) rather than inventing a parallel mechanism.

### Positive Consequences

* Trust is fine-grained (one run, one worker, one process) and time-boxed (`expiresAt`), which a
  static shared secret is neither.
* A restarted subprocess in the same process tree can reconnect with the same token while the run,
  vendor process, and expiry all remain live — covering the realistic "MCP subprocess crashed and
  relaunched" case without weakening the boundary for an unrelated process.
* Platforms without trustworthy peer-process identity report coordination as explicitly
  unsupported, rather than silently accepting a connection nobody actually verified.

### Negative Consequences

* This mechanism is tested in isolation (protocol contract tests, broker integration tests) but
  has no production wiring point yet: nothing in the current `run/submit` path (which has no real
  adapter — see ADR-0013) calls `ScopeTokenStore::mint`, and `ServerConfig::default()`'s
  `worker_verifier` stays `RejectAllWorkerVerifier`. The Worker Adapters milestone must wire
  minting into wherever it actually launches a vendor process.

## Pros and Cons of the Options

### Minted, run-scoped token + PID ancestry (chosen)

* Good, because the scope is exactly as narrow as the trust that's actually warranted.
* Bad, because PID-ancestry checking is inherently platform-specific and needs an explicit
  unsupported path rather than a universal implementation.

### Static shared secret per worker

* Good, because it's the simplest possible mechanism.
* Bad, because it's scoped to "a worker," not to one run's one launched process — a leaked secret
  grants far more than the minimum this system actually needs to trust.

### mTLS client certificates

* Good, because it's a well-understood, strong mechanism with revocation support.
* Bad, because it's a lot of certificate-lifecycle machinery for a same-machine boundary that
  already has Unix-socket UID admission (ADR-0004) doing the outer layer of the job — the marginal
  security benefit doesn't justify the operational complexity here.

## Links

* Narrated in `../journal.md`, commit `3172d99`
* Fills the seam left open by [ADR-0009](0009-role-based-authorization-from-the-connection-not-per-call.md)
