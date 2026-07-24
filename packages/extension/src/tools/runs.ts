// `batman_run`: submits, lists, fetches, retries, and cancels runs.
// `submit` and `cancel` are tier `exec` -- they start or stop adapter
// processes. `retry` creates a distinct run (never mutates the prior one).

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_RUN_TOOL_NAME = "batman_run";

export function registerRunTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["submit", "list", "get", "retry", "cancel"]).describe("Which run operation to perform."),
    taskId: pi.zod.string().optional().describe("Required for submit: the task to execute. Optional filter for list."),
    workerId: pi.zod.string().optional().describe("Required for submit and retry: the worker to execute with."),
    workspaceMode: pi.zod.string().optional().describe("Optional workspace mode for submit (shared or isolated)."),
    priority: pi.zod.number().int().optional().describe("Optional priority for submit."),
    runId: pi.zod.string().optional().describe("Required for get and cancel: the run id."),
    priorRunId: pi.zod.string().optional().describe("Required for retry: the terminal run id to retry."),
  });

  pi.registerTool({
    name: BATMAN_RUN_TOOL_NAME,
    label: "BATMAN Run",
    description:
      "Submits, lists, fetches, retries, or cancels a BATMAN run. A retry always creates a new run id, never resurrects the prior one.",
    parameters: params,
    approval: (args) =>
      typeof args === "object" && args !== null && "op" in args && (args.op === "submit" || args.op === "cancel")
        ? "exec"
        : "read",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      switch (input.op) {
        case "submit":
          return callOrchestration(client, "run/submit", {
            taskId: input.taskId,
            workerId: input.workerId,
            workspaceMode: input.workspaceMode,
            priority: input.priority,
          });
        case "list":
          return callOrchestration(client, "run/list", { taskId: input.taskId });
        case "get":
          return callOrchestration(client, "run/get", { runId: input.runId });
        case "retry":
          return callOrchestration(client, "run/retry", {
            priorRunId: input.priorRunId,
            workerId: input.workerId,
          });
        case "cancel":
          return callOrchestration(client, "run/cancel", { runId: input.runId });
      }
    },
  });
}
