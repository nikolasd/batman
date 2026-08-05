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
    prompt: pi.zod.string().optional().describe("Required for submit: the instruction the worker executes. BATMAN stores no task text, so the task's description must be passed here."),
    taskId: pi.zod.string().optional().describe("Required for submit: the task to execute. Optional filter for list."),
    workerId: pi.zod.string().optional().describe("Required for submit and retry: the worker to execute with."),
    workspaceMode: pi.zod.string().optional().describe("Optional workspace mode for submit: 'shared' (the repository itself, the default), 'isolated' (a per-run git worktree), or 'copy' (a per-run copy of the repository). Any other value is rejected."),
    priority: pi.zod.number().int().optional().describe("Optional priority for submit."),
    runId: pi.zod.string().optional().describe("Required for get and cancel: the run id."),
    priorRunId: pi.zod.string().optional().describe("Required for retry: the terminal run id to retry."),
  });

  pi.registerTool({
    name: BATMAN_RUN_TOOL_NAME,
    label: "BATMAN Run",
    description:
      "Use to execute, monitor, or manage task execution by external workers. Use op: 'submit' to start execution (requires taskId from batman_task, workerId from batman_worker, and prompt -- the instruction text the worker executes), op: 'get' to check progress/status of a run, op: 'list' to list runs for a task, op: 'retry' to retry a terminal run (creates new runId, never resurrects the prior one), or op: 'cancel' to stop a running run. After submitting, monitor with op: 'get'. If the run fails, retry with op: 'retry' (new runId). If stuck, cancel with op: 'cancel'.",
    parameters: params,
    approval: (args) => (typeof args === "object" && args !== null && "op" in args && (args.op === "submit" || args.op === "cancel") ? "exec" : "read"),
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      switch (input.op) {
        case "submit":
          return callOrchestration(client, "run/submit", {
            taskId: input.taskId,
            prompt: input.prompt,
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
