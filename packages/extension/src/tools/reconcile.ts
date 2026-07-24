// `batman_reconcile`: rebinds a task's owning OMP client instance after a
// disconnect/reconnect. The runtime only accepts the rebind when task id
// and monotonic OMP revision match; it journals the old/new owner ids.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_RECONCILE_TOOL_NAME = "batman_reconcile";

export function registerReconcileTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    taskId: pi.zod.string().describe("The task id to rebind to this OMP client instance."),
    revision: pi.zod.number().int().nonnegative().describe("The monotonic OMP revision that must match the stored task."),
  });

  pi.registerTool({
    name: BATMAN_RECONCILE_TOOL_NAME,
    label: "BATMAN Reconcile",
    description:
      "Rebinds a BATMAN-tracked task from a disconnected OMP client instance to this one, only on a matching revision.",
    parameters: params,
    approval: "write",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      return callOrchestration(client, "reconcile/omp", {
        taskId: input.taskId,
        revision: input.revision,
      });
    },
  });
}
