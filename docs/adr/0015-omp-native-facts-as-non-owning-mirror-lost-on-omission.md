# OMP-native facts as a non-owning mirror, `lost` on omission

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

OMP has its own native subagents, with their own lifecycle, entirely independent of BATMAN's
`Run` records. The extension can observe OMP's `task:subagent:lifecycle|progress|event` bus, but
must never mistake observation for ownership — and must handle the case where the OMP process that
was reporting on a subagent disappears and a new one starts, without either fabricating a false
"succeeded" or silently promoting an unverified fact into runtime-scoped state. How should these
facts be represented so that uncertainty is visible rather than guessed away?

## Decision Drivers

* Explicit constraint: OMP-native task agents remain parent-scoped, and become `lost` when their
  OMP process disappears — never `succeeded`, and never silently promoted to a runtime-scoped run.
* A restart (new OMP process) must reconcile safely against whatever facts a *prior* process last
  recorded, without assuming the prior process's last-known status is still true.
* Progress updates arrive noisily and out of order relative to terminal events; the reconciliation
  logic must not let a stale in-flight update regress a fact that has already gone terminal.

## Considered Options

* A dedicated `OmpNativeStatus` vocabulary (`working`/`succeeded`/`failed`/`lost`), deliberately
  distinct from `RunState`, tagged with an `ompProcessEpoch` per observing OMP process;
  `reconcileAcrossRestart` demotes any non-terminal fact from a prior epoch to `lost` when a new
  epoch starts and the subagent isn't re-reported.
* Promote OMP-native agents into full `Run` records the moment they're observed, treating
  observation as equivalent to runtime supervision.
* Ignore OMP-native agents entirely until a tool explicitly mirrors one.

## Decision Outcome

Chosen option: the non-owning mirror with epoch-based reconciliation.
`OmpNativeReconciler` coalesces non-terminal `progress` updates for 150ms (avoiding re-render
thrash from a noisy "still running" stream) but lets every terminal lifecycle event through
immediately, and never lets a stale coalesced update regress a fact that already went terminal.
`reconcileAcrossRestart(priorFacts, currentEpoch)` is the mechanism that turns "a new OMP process
doesn't mention this subagent" into `lost`, deterministically, every time.

### Positive Consequences

* No false positives: a subagent is never reported `succeeded` unless OMP itself said so.
* No silent promotion: an OMP-native fact never becomes a `Run` row just by being observed —
  keeping ADR-0011's authority boundary intact even for facts about OMP's *own* subagents.
* Restart safety is a named, tested function (`reconcileAcrossRestart`), not an implicit property
  of whatever state happened to survive.

### Negative Consequences

* Two parallel status vocabularies now exist (`RunState` for runtime-owned runs, `OmpNativeStatus`
  for observed-only facts) that a reader must keep straight — mitigated by the naming
  (`OmpNativeStatus` is never used where a `RunState` is expected, and vice versa) and by
  `architecture.md` documenting the distinction explicitly.

## Pros and Cons of the Options

### Non-owning mirror, epoch-reconciled (chosen)

* Good, because uncertainty (a subagent an OMP process stopped mentioning) is represented
  honestly, as `lost`, rather than guessed either optimistically or pessimistically.
* Bad, because it requires a second status vocabulary and an explicit reconciliation function to
  reason about correctly.

### Promote to full `Run` on observation

* Good, because it would unify the two vocabularies into one.
* Bad, because it would mean BATMAN starts treating an OMP-native subagent as something it
  supervises, which it doesn't and structurally can't — directly violating the authority boundary
  in ADR-0011.

### Ignore until explicitly mirrored

* Good, because it would need the least code.
* Bad, because it would silently drop exactly the information (OMP's own subagent activity) this
  feature exists to surface, and would defeat the restart-safety requirement entirely — there
  would be nothing to reconcile.

## Links

* Narrated in `../journal.md`, commit `bfd6620`
* Respects [ADR-0011](0011-omp-retains-task-graph-authority.md)
