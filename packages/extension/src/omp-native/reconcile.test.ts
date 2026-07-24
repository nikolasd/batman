import { expect, test } from "bun:test";

import type {
  SubagentLifecyclePayload,
  SubagentProgressPayload,
} from "@oh-my-pi/pi-coding-agent/task";

import { normalizeEventPayload, normalizeLifecyclePayload, normalizeProgressPayload } from "./events";
import {
  OmpNativeReconciler,
  createOmpProcessEpoch,
  reconcileAcrossRestart,
  reconcileWithRuntime,
} from "./reconcile";
import type { OmpNativeAgentFact } from "./types";

const EPOCH_A = "epoch-a";

// The reconciler's coalescing window is a real 150ms `setTimeout`; testing
// it deterministically would require injecting a mockable timer into
// `OmpNativeReconciler`, which isn't worth the added surface for a single
// debounce behavior. These tests genuinely exercise the platform clock.
function delay(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

function lifecyclePayload(
  status: SubagentLifecyclePayload["status"],
  overrides: Partial<SubagentLifecyclePayload> = {},
): SubagentLifecyclePayload {
  return {
    id: "agent-1",
    agent: "task",
    agentSource: "bundled",
    status,
    index: 0,
    description: "do the thing",
    sessionFile: "/tmp/session-1.jsonl",
    ...overrides,
  };
}

function progressPayload(
  status: SubagentProgressPayload["progress"]["status"],
  overrides: Partial<SubagentProgressPayload["progress"]> = {},
): SubagentProgressPayload {
  return {
    index: 0,
    agent: "task",
    agentSource: "bundled",
    task: "do the thing",
    sessionFile: "/tmp/session-1.jsonl",
    progress: {
      index: 0,
      id: "agent-1",
      agent: "task",
      agentSource: "bundled",
      status,
      task: "do the thing",
      recentTools: [],
      recentOutput: [],
      toolCount: 0,
      requests: 0,
      tokens: 0,
      cost: 0,
      durationMs: 0,
      ...overrides,
    },
  };
}

// ------------------------------------------------------- pure normalization

test("normalizeLifecyclePayload maps started to working without mutating the payload", () => {
  const payload = lifecyclePayload("started");
  const frozen = structuredClone(payload);
  const fact = normalizeLifecyclePayload(payload, EPOCH_A, 1000);
  expect(fact).toEqual({
    ompAgentId: "agent-1",
    status: "working",
    description: "do the thing",
    sessionFile: "/tmp/session-1.jsonl",
    artifactRefs: [],
    ompProcessEpoch: EPOCH_A,
    observedAtMs: 1000,
  });
  expect(payload).toEqual(frozen);
});

test("normalizeLifecyclePayload maps completed to succeeded and failed/aborted to failed", () => {
  expect(normalizeLifecyclePayload(lifecyclePayload("completed"), EPOCH_A, 0).status).toBe("succeeded");
  expect(normalizeLifecyclePayload(lifecyclePayload("failed"), EPOCH_A, 0).status).toBe("failed");
  expect(normalizeLifecyclePayload(lifecyclePayload("aborted"), EPOCH_A, 0).status).toBe("failed");
});

test("normalizeProgressPayload maps pending and running to working", () => {
  expect(normalizeProgressPayload(progressPayload("pending"), EPOCH_A, 0).status).toBe("working");
  expect(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 0).status).toBe("working");
});

test("normalizeProgressPayload maps completed to succeeded and failed/aborted to failed", () => {
  expect(normalizeProgressPayload(progressPayload("completed"), EPOCH_A, 0).status).toBe("succeeded");
  expect(normalizeProgressPayload(progressPayload("failed"), EPOCH_A, 0).status).toBe("failed");
  expect(normalizeProgressPayload(progressPayload("aborted"), EPOCH_A, 0).status).toBe("failed");
});

test("normalizeEventPayload returns undefined: raw session events carry no lifecycle status", () => {
  const fact = normalizeEventPayload({ id: "agent-1", event: { type: "message_end" } as never });
  expect(fact).toBeUndefined();
});

// --------------------------------------------------------------- coalescing

test("OmpNativeReconciler coalesces rapid non-terminal progress within the window", async () => {
  const changes: OmpNativeAgentFact[] = [];
  const reconciler = new OmpNativeReconciler((fact) => changes.push(fact));

  reconciler.record(normalizeProgressPayload(progressPayload("pending"), EPOCH_A, 0));
  reconciler.record(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 10));
  reconciler.record(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 20));

  // Nothing committed yet: still inside the coalescing window.
  expect(reconciler.get("agent-1")).toBeUndefined();
  expect(changes).toHaveLength(0);

  await delay(200);

  // Only the final coalesced update commits.
  expect(changes).toHaveLength(1);
  expect(reconciler.get("agent-1")?.observedAtMs).toBe(20);

  reconciler.dispose();
});

test("OmpNativeReconciler commits a terminal lifecycle event immediately, bypassing coalescing", () => {
  const changes: OmpNativeAgentFact[] = [];
  const reconciler = new OmpNativeReconciler((fact) => changes.push(fact));

  reconciler.record(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 0));
  reconciler.record(normalizeLifecyclePayload(lifecyclePayload("completed"), EPOCH_A, 10));

  expect(changes).toHaveLength(1);
  expect(reconciler.get("agent-1")?.status).toBe("succeeded");

  reconciler.dispose();
});

test("OmpNativeReconciler never regresses a terminal fact from a stale pending update", async () => {
  const reconciler = new OmpNativeReconciler();

  reconciler.record(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 0));
  reconciler.record(normalizeLifecyclePayload(lifecyclePayload("completed"), EPOCH_A, 5));
  // A stale progress update racing in after the terminal lifecycle event.
  reconciler.record(normalizeProgressPayload(progressPayload("running"), EPOCH_A, 10));

  await delay(200);

  expect(reconciler.get("agent-1")?.status).toBe("succeeded");
  reconciler.dispose();
});

// ------------------------------------------------------- restart -> lost

test("reconcileAcrossRestart transitions an omitted non-terminal run to lost, never succeeded", () => {
  const priorEpoch = createOmpProcessEpoch();
  const currentEpoch = createOmpProcessEpoch();
  expect(priorEpoch).not.toBe(currentEpoch);

  const runningUnderPriorProcess: OmpNativeAgentFact = {
    ompAgentId: "agent-1",
    status: "working",
    artifactRefs: [],
    ompProcessEpoch: priorEpoch,
    observedAtMs: 0,
  };
  const idleUnderPriorProcess: OmpNativeAgentFact = {
    ompAgentId: "agent-2",
    status: "working",
    artifactRefs: [],
    ompProcessEpoch: priorEpoch,
    observedAtMs: 0,
  };
  const alreadySucceeded: OmpNativeAgentFact = {
    ompAgentId: "agent-3",
    status: "succeeded",
    artifactRefs: [],
    ompProcessEpoch: priorEpoch,
    observedAtMs: 0,
  };

  const reconciled = reconcileAcrossRestart(
    [runningUnderPriorProcess, idleUnderPriorProcess, alreadySucceeded],
    currentEpoch,
  );

  expect(reconciled.find((f) => f.ompAgentId === "agent-1")?.status).toBe("lost");
  expect(reconciled.find((f) => f.ompAgentId === "agent-2")?.status).toBe("lost");
  // A run that already reached a terminal state before the restart keeps it.
  expect(reconciled.find((f) => f.ompAgentId === "agent-3")?.status).toBe("succeeded");
});

test("reconcileAcrossRestart leaves facts from the current epoch untouched", () => {
  const currentEpoch = createOmpProcessEpoch();
  const stillLive: OmpNativeAgentFact = {
    ompAgentId: "agent-1",
    status: "working",
    artifactRefs: [],
    ompProcessEpoch: currentEpoch,
    observedAtMs: 0,
  };
  const reconciled = reconcileAcrossRestart([stillLive], currentEpoch);
  expect(reconciled[0]?.status).toBe("working");
});

// ----------------------------------------------------------- reconcile/omp

test("reconcileWithRuntime calls reconcile/omp with the correlated task id and revision", async () => {
  const calls: Array<{ method: string; params: unknown }> = [];
  const fakeClient = {
    request: async (method: string, params?: unknown) => {
      calls.push({ method, params });
      return { taskId: "task-1", newOwnerClientInstanceId: "omp-2", sequence: 5 };
    },
  };

  const result = await reconcileWithRuntime(fakeClient, { taskId: "task-1", revision: 3 });

  expect(calls).toEqual([{ method: "reconcile/omp", params: { taskId: "task-1", revision: 3 } }]);
  expect(result).toEqual({ taskId: "task-1", newOwnerClientInstanceId: "omp-2", sequence: 5 });
});

test("reconcileWithRuntime is a no-op when no task correlation is known", async () => {
  const fakeClient = {
    request: async () => {
      throw new Error("must not be called");
    },
  };
  const result = await reconcileWithRuntime(fakeClient, undefined);
  expect(result).toBeUndefined();
});
