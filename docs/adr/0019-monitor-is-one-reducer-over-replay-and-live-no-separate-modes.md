# The monitor is one reducer over replay and live — no separate modes

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

The embedded `/batman` monitor needs to show correct state both when a session first connects
(potentially hours of history to catch up on) and as new events arrive live. Should "catching up"
and "reacting live" be two different code paths, or can they be unified — and if unified, how does
that avoid double-applying an event on a reconnect?

## Decision Drivers

* Two code paths implementing conceptually the same operation ("apply this event to the current
  state") is a well-known source of subtle divergence bugs — the two paths drift apart over time
  even when written correctly on day one.
* Reconnecting must be safe: replaying history that's already been applied must never duplicate
  rows or double-count anything.
* The widget must degrade gracefully (never a silent truncation) when there's more state than fits
  in a compact view.

## Considered Options

* One pure function, `reduceEvent(state, envelope) -> MonitorState`, driving both the initial
  `events/replay` drain and every subsequent live `events/event` notification identically; the
  reducer is a no-op for any event whose `sequence` is not newer than what's already applied to
  that run's row, making replay of an already-seen event idempotent by construction.
* Separate logic: a dedicated "load initial state" query plus separate "apply a live delta" logic.
* No incremental state at all — re-fetch a full snapshot from the runtime on every update (an
  approach closer to polling a `runtime/status`-shaped endpoint).

## Decision Outcome

Chosen option: one reducer, no separate modes. `client.subscribe(fromSequence, onEvent)` itself
drains `events/replay` before delivering live notifications, but every event — replayed or live —
flows through the exact same `reduceEvent` call inside `MonitorController`. The last-applied
sequence is persisted via `pi.appendEntry` after every applied event, so a fresh session resumes
from exactly where it left off rather than from zero.

### Positive Consequences

* Reconnect-safety is a direct consequence of the reducer's own idempotency check — no special
  "am I replaying or live" flag needs to exist anywhere, because the reducer behaves identically
  either way.
* One function (`model.ts::reduceEvent`) is exhaustively unit-tested
  (`model.test.ts`) and that single test suite covers both operating modes at once, rather than
  needing a second suite for a second code path.
* The widget's bounded, ten-row view (with `/batman status <runId>` as the always-available full
  detail) never silently truncates — it's a deliberate rendering choice on top of a reducer that
  itself has no size limit.

### Negative Consequences

* Every mutation's `RuntimeEvent` payload must carry enough context for the reducer to reconstruct
  display state without any extra runtime query — the reducer cannot "go ask the runtime for more
  detail" mid-reduction, so event payload design and monitor design are coupled decisions.

## Pros and Cons of the Options

### One reducer, no separate modes (chosen)

* Good, because it collapses two conceptually identical operations into one, tested once.
* Bad, because it puts real design pressure on every event payload to be self-sufficient.

### Separate initial-load and live-delta logic

* Good, because each path could in principle be optimized independently.
* Bad, because "optimized independently" is exactly how the two paths drift out of sync over time,
  and the two paths must still agree on the same final state or the monitor becomes unreliable.

### Full re-fetch on every update

* Good, because it would never need to reason about incremental state at all.
* Bad, because it reintroduces polling and its latency/cost tradeoffs, exactly what the
  replay-then-subscribe design (built on the broadcast channel from ADR-0020) exists to avoid.

## Links

* Narrated in `../journal.md`, commit `aabc950`
* Depends on [ADR-0020](0020-per-mutation-event-broadcast-is-not-optional.md) for live delivery
