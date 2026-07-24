# Repository-scoped daemon singleton via kernel flock

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

Exactly one `batcave` daemon should serve any given repository at a time, no matter how many OMP
sessions connect to it concurrently, and a crashed daemon must never leave behind a stale marker
that blocks the next start. What mechanism guarantees the singleton and self-heals after a crash
without any manual cleanup?

## Decision Drivers

* Multiple OMP sessions in the same repository must converge on one daemon, not race to spawn
  several.
* A crashed daemon (kill -9, OOM, panic) must not require a human or a script to clean up a stale
  marker before the next start succeeds.
* Liveness checking should not depend on PID reuse being rare enough to ignore.

## Considered Options

* An exclusive kernel `flock` on a persistent `runtime.lock` file, never deleted; the loser of the
  race reads the winner's metadata (written under the held lock) and exits with a distinct code.
* A PID file, with liveness checked by `kill(pid, 0)`.
* Delegate singleton enforcement to an OS service manager (launchd/systemd).

## Decision Outcome

Chosen option: kernel `flock`. `serve` takes `flock(LOCK_EX | LOCK_NB)` on `runtime.lock` before
doing anything else; the loser exits with code **73**, printing machine-readable `already_running`
JSON. `stop`/`status` probe liveness by *attempting the same flock*, never by inspecting a PID.

### Positive Consequences

* Staleness is implicit: the kernel releases the flock the instant the owning process dies, crash
  or clean exit alike — there is no window where a stale lock blocks a legitimate new start.
* No pid-recycling hazard: the lock, not a remembered PID, is the fact being checked.
* The lock file is never deleted, which removes an entire class of race ("what if two processes
  try to recreate the file at once") by construction.

### Negative Consequences

* `flock` semantics have platform nuances (notably, unreliable over network filesystems) — fine
  given the state root is always local, but worth knowing if that assumption is ever revisited.
* The lock file existing forever (never deleted) means anyone unfamiliar with the design might be
  tempted to "clean it up" in a script; this is explicitly documented as harmless-to-the-daemon
  but disruptive to `status`/`stop`'s ability to read its metadata.

## Pros and Cons of the Options

### Kernel flock, never-deleted lock file (chosen)

* Good, because liveness-check and lock-acquisition are the same operation — there's no second
  mechanism to keep in sync with the first.
* Bad, because it needs a filesystem that supports `flock` correctly (true for every target this
  project supports).

### PID file + `kill(pid, 0)`

* Good, because it's a well-known, simple pattern.
* Bad, because PID reuse is a real hazard on long-running systems, and it needs a *second*
  mechanism (typically also a lock) to be race-free anyway — so it doesn't actually save
  complexity, it just hides where the real guarantee comes from.

### OS service manager enforcement

* Good, because launchd/systemd already have mature process-supervision primitives.
* Bad, because it would require installing a service definition per repository (or a generic one
  that doesn't fit "one daemon per repository, started on demand"), which conflicts with the
  zero-install goal behind connect-or-spawn (ADR-0008).

## Links

* Narrated in `../journal.md`, commit `18a76fd`
* Paired with [ADR-0008](0008-connect-or-spawn-with-idle-self-shutdown.md)
