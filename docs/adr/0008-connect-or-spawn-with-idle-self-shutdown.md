# Connect-or-spawn daemon lifecycle with idle self-shutdown

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

A daemon that has to be manually started before OMP can use it is a setup step every user would
have to remember and every CI environment would have to script around. A daemon that never stops
is a resource leak nobody notices until dozens have accumulated across old repositories. How should
the daemon's lifecycle be tied to actual demand?

## Decision Drivers

* The extension must work immediately after `npm install`, with no separate daemon-install step.
* Multiple concurrent OMP sessions in the same repository should not each try to spawn a daemon.
* A daemon nobody is using should eventually go away on its own.

## Considered Options

* Client-driven connect-or-spawn: the extension tries to connect first; if nothing answers, it
  selects a binary, spawns it detached, and retries with backoff. The daemon itself exits after an
  idle timeout with zero connections and zero active runs.
* An always-on system service, installed once at extension-install time.
* A daemon spawned explicitly at OMP `session_start` and torn down explicitly at
  `session_shutdown`, one per session.

## Decision Outcome

Chosen option: connect-or-spawn plus idle self-shutdown. `ensureRuntime` tries to connect; on
failure, it validates/selects a binary and spawns it detached (`stdio: "ignore"`, `.unref()`,
deliberately without `--foreground` so the daemon owns its own log file), then retries connecting
with bounded exponential backoff for up to five seconds. The daemon exits on its own after
`--idle-seconds` with no connections and no active runs. If a concurrent caller already won the
startup race (via the flock, ADR-0007), this caller simply connects to the winner instead of
failing.

### Positive Consequences

* Zero-install: the first tool call that needs the daemon starts it; nothing to set up in advance.
* Self-healing: an idle daemon exits on its own, and the next caller that needs it just starts a
  fresh one — no accumulation of forgotten processes.
* Multiple OMP sessions in the same repository converge on one daemon automatically, because the
  flock decides the race, not application-level coordination.

### Negative Consequences

* Idle-timeout tuning is a real tradeoff: too short and the daemon respawns (and re-migrates,
  re-binds) more often than useful; too long and an idle daemon lingers longer than it needs to.
  Currently fixed at `DEFAULT_IDLE_SECONDS = 1800`.
* A daemon tied to a *repository*, not to any one OMP session, means it outlives the session that
  spawned it — which is the intended behavior (so a second session reconnects instead of
  respawning) but is worth stating explicitly, since it's easy to assume session-scoped lifetime
  by default.

## Pros and Cons of the Options

### Connect-or-spawn + idle self-shutdown (chosen)

* Good, because it needs zero setup and self-heals without any external supervision.
* Bad, because idle-timeout tuning has no universally correct value.

### Always-on system service

* Good, because the daemon would always be immediately available, no cold-start latency.
* Bad, because it needs a per-platform service installer, elevated permissions on some platforms,
  and runs even for repositories nobody is actively working in.

### Explicit spawn/teardown per OMP session

* Good, because lifetime maps exactly to a session someone can reason about.
* Bad, because two concurrent sessions in the same repository would each spawn (and each tear
  down) their own daemon, defeating "one daemon per repository" and multiplying migration/startup
  cost for no benefit.

## Links

* Narrated in `../journal.md`, commit `18a76fd`
* Paired with [ADR-0007](0007-repository-scoped-singleton-via-kernel-flock.md)
