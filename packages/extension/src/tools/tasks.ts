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
    description: pi.zod.string().describe("What the task should do (natural language description)."),
    taskId: pi.zod.string().optional().describe("Optional: Reuse an existing task ID (for resume). Auto-generated if omitted."),
  });

  pi.registerTool({
    name: BATMAN_TASK_TOOL_NAME,
    label: "BATMAN Task",
    description:
      "Use when you need to create a persistent, cross-session unit of work that will be executed by an external AI harness (Claude, Codex, Copilot, or OMP-RPC) -- not OMP's native in-process task subagent. Persists across session disconnects (stored in SQLite journal), executes via external harness processes, and can be retried, cancelled, or reconciled after failure. Auto-generates a task ID and uses your OMP session as owner. After creating, select a worker with batman_worker { op: 'list' } and submit execution with batman_run { op: 'submit', taskId, workerId }.",
    parameters: params,
    approval: "write",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      
      // Auto-generate taskId if not provided (new task)
      const taskId = input.taskId ?? crypto.randomUUID();
      
      // Use OMP session ID as owner
      const ownerClientInstanceId = extCtx.sessionManager.getSessionId();
      
      // Create a new task (always upsert with revision 0)
      return callOrchestration(client, "task/upsert", {
        taskId,
        ownerClientInstanceId,
        revision: 0,
      });
    },
  });
}
