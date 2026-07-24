# External OMP extension with a separate Rust daemon

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

BATMAN needs to supervise worker processes, speak low-level adapter protocols, persist durable
state safely, and recover after crashes — none of which are things an in-process OMP extension is
well suited to own directly. At the same time, OMP extensions run as Bun/TypeScript code inside
OMP's own process, and the project's constraints rule out forking OMP or depending on any private
OMP API. Where should the process-supervision and persistence logic actually live?

## Decision Drivers

* Must remain an external OMP extension — no OMP fork, no private API dependency.
* Process supervision, filesystem/security-sensitive work, and durable storage benefit from a
  language with strong static guarantees and mature low-level libraries (process spawning, file
  locking, SQLite drivers).
* OMP's own tool/command surface (Zod schemas, `ExtensionAPI`) is native to TypeScript; nothing is
  gained by re-implementing that surface in Rust.
* The extension must never be the *only* durable copy of anything — if OMP restarts or the
  extension reloads, no state should be lost.

## Considered Options

* Pure TypeScript extension, handling process supervision, persistence, and IPC to adapters all
  in-process.
* A separate Rust daemon (`batcave`), one per repository, communicating with the TypeScript
  extension over a local socket; the extension owns OMP-facing tools/UI, the daemon owns
  everything else.
* Fork or patch OMP itself to add native hooks for process supervision.

## Decision Outcome

Chosen option: a separate Rust daemon plus a TypeScript extension, because it is the only option
that satisfies "external extension, no fork" while still getting Rust's process/security/storage
strengths. The daemon persists all durable state (SQLite journal); the extension never owns the
only copy of anything — durability always lives in the daemon.

### Positive Consequences

* Clean separation of concerns: OMP decides *what* to do, the extension exposes *that* to OMP, the
  daemon does the *how* of running and persisting it.
* The daemon can be developed, tested, and restarted independently of any running OMP session.
* Crash safety is a Rust-daemon concern; the extension can reload freely without losing state.

### Negative Consequences

* Two languages, two toolchains, two test runners to keep in sync (mitigated by generated
  bindings — see ADR-0002).
* Every operation crossing the boundary needs a defined wire protocol (see ADR-0004) and pays a
  small IPC cost that an in-process call wouldn't.
* A new failure mode exists that wouldn't otherwise: "the daemon is unreachable," which every
  extension code path must handle explicitly.

## Pros and Cons of the Options

### Pure TypeScript extension

* Good, because it avoids IPC entirely and needs only one toolchain.
* Bad, because Bun/Node's process-supervision, file-locking, and SQLite ergonomics are weaker than
  Rust's, and any crash-safety bug would corrupt the daemon's own state with no separate recovery
  boundary.

### Separate Rust daemon + TypeScript extension (chosen)

* Good, because it plays to each language's strengths and keeps OMP's own API surface untouched.
* Bad, because it requires a real, versioned, tested wire protocol between the two.

### Fork/patch OMP

* Good, because it could avoid IPC overhead entirely.
* Bad, because it violates the project's explicit constraint against depending on private OMP
  APIs or maintaining a fork, and would make every OMP upgrade a merge conflict.

## Links

* Narrated in `../journal.md`, commit `e62e5ec` ("build: scaffold batman workspaces")
* Enables [ADR-0004](0004-json-rpc-2-over-bounded-ndjson-on-a-unix-socket.md) (the wire protocol
  this boundary needs)
