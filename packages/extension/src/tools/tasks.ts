// `batman_task`: the extension-side front for OMP-owner `task/upsert` and
// `task/get`. A worker-scoped MCP tool of the same display name runs in a
// different process/tool registry and exposes read-only task context; this
// tool is the ompExtension-authorized counterpart.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_TASK_TOOL_NAME = "batman_task";

export function registerTaskTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["upsert", "get"]).describe("Which task operation to perform."),
    taskId: pi.zod.string().optional().describe("Task id: required for get; optional for upsert (creates when omitted)."),
    ownerClientInstanceId: pi.zod.string().optional().describe("Required for upsert: the OMP client instance id that owns this task."),
    revision: pi.zod.number().int().nonnegative().optional().describe("Required for upsert: the monotonic OMP revision of this task."),
  });

  pi.registerTool({
    name: BATMAN_TASK_TOOL_NAME,
    label: "BATMAN Task",
    description:
      "Upserts or fetches a BATMAN-tracked task record mirroring OMP-owned task intent. Never creates or edits the OMP task graph itself.",
    parameters: params,
    approval: "write",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      if (input.op === "upsert") {
        return callOrchestration(client, "task/upsert", {
          taskId: input.taskId,
          ownerClientInstanceId: input.ownerClientInstanceId,
          revision: input.revision,
        });
      }
      return callOrchestration(client, "task/get", { taskId: input.taskId });
    },
  });
}
