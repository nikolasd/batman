// Reconciliation bookkeeping for OMP-native subagent facts: coalesces
// noisy progress updates, lets terminal lifecycle events through
// immediately, and detects parent-scoped runs an OMP process disappeared
// without ever reporting terminal -- those become `lost`, never
// `succeeded` and never silently promoted to a runtime-scoped run.

import type { OmpNativeAgentFact, OmpNativeStatus, OmpNativeTaskCorrelation } from "./types";

/** Coalescing window for non-terminal progress updates, in ms. */
const PROGRESS_COALESCE_MS = 150;

const TERMINAL_STATUSES: ReadonlySet<OmpNativeStatus> = new Set(["succeeded", "failed", "lost"]);

/** A minimal client seam: only the one RPC call this module needs. */
export interface ReconcileOmpClient {
  request(method: string, params?: unknown): Promise<unknown>;
}

/**
 * Tracks the latest known fact per OMP-native subagent. Non-terminal facts
 * (working) are coalesced within {@link PROGRESS_COALESCE_MS}; a terminal
 * fact (succeeded/failed/lost) is committed immediately and never
 * regresses back to non-terminal from a stale, still-pending progress
 * update racing behind it.
 */
export class OmpNativeReconciler {
  readonly #facts = new Map<string, OmpNativeAgentFact>();
  readonly #pendingTimers = new Map<string, ReturnType<typeof setTimeout>>();
  readonly #onChange: (fact: OmpNativeAgentFact) => void;

  constructor(onChange: (fact: OmpNativeAgentFact) => void = () => {}) {
    this.#onChange = onChange;
  }

  /** Records a normalized fact, applying the coalescing rule above. */
  record(fact: OmpNativeAgentFact): void {
    const previous = this.#facts.get(fact.ompAgentId);
    if (previous !== undefined && TERMINAL_STATUSES.has(previous.status)) {
      return;
    }

    const pending = this.#pendingTimers.get(fact.ompAgentId);
    if (pending !== undefined) {
      clearTimeout(pending);
      this.#pendingTimers.delete(fact.ompAgentId);
    }

    if (TERMINAL_STATUSES.has(fact.status)) {
      this.#facts.set(fact.ompAgentId, fact);
      this.#onChange(fact);
      return;
    }

    this.#pendingTimers.set(
      fact.ompAgentId,
      setTimeout(() => {
        this.#pendingTimers.delete(fact.ompAgentId);
        this.#facts.set(fact.ompAgentId, fact);
        this.#onChange(fact);
      }, PROGRESS_COALESCE_MS),
    );
  }

  /** Returns the latest committed fact for `ompAgentId`, if any. */
  get(ompAgentId: string): OmpNativeAgentFact | undefined {
    return this.#facts.get(ompAgentId);
  }

  /** Returns every committed fact. */
  all(): readonly OmpNativeAgentFact[] {
    return [...this.#facts.values()];
  }

  /** Clears all pending coalesce timers. Call on `session_shutdown`. */
  dispose(): void {
    for (const timer of this.#pendingTimers.values()) {
      clearTimeout(timer);
    }
    this.#pendingTimers.clear();
  }
}

/**
 * Reconciles facts observed by a prior OMP process against the current
 * process epoch: any non-terminal fact whose recorded epoch differs from
 * `currentEpoch` is a parent-scoped run whose OMP process disappeared
 * without a terminal lifecycle event. It transitions to `lost` -- never
 * `succeeded`, and never silently rendered as still runtime-scoped.
 */
export function reconcileAcrossRestart(priorFacts: readonly OmpNativeAgentFact[], currentEpoch: string): OmpNativeAgentFact[] {
  return priorFacts.map((fact) => {
    if (fact.ompProcessEpoch === currentEpoch || TERMINAL_STATUSES.has(fact.status)) {
      return fact;
    }
    return { ...fact, status: "lost", ompProcessEpoch: currentEpoch };
  });
}

/** Generates one process-scoped epoch id. Call exactly once per OMP process. */
export function createOmpProcessEpoch(): string {
  return crypto.randomUUID();
}

/**
 * Calls the runtime's `reconcile/omp` for the task correlated with an
 * OMP-native agent, rebinding ownership to this OMP instance. A no-op
 * (resolves `undefined`) when no correlation is known -- an uncorrelated
 * fact has no runtime task to rebind.
 */
export async function reconcileWithRuntime(client: ReconcileOmpClient, correlation: OmpNativeTaskCorrelation | undefined): Promise<unknown> {
  if (correlation === undefined) {
    return undefined;
  }
  return client.request("reconcile/omp", {
    taskId: correlation.taskId,
    revision: correlation.revision,
  });
}
