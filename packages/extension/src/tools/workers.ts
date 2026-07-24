// `batman_worker`: creates, lists, and fetches logical worker identities.
// `create` is tier `exec` -- it provisions a harness/profile identity that
// later runs execute against.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_WORKER_TOOL_NAME = "batman_worker";

export function registerWorkerTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["create", "list", "get"]).describe("Which worker operation to perform."),
    fingerprint: pi.zod.string().optional().describe("Required for create: a fingerprint of the harness binary + version."),
    adapter: pi.zod.string().optional().describe("Required for create: the adapter name, e.g. claude, codex, copilot, ompNative."),
    model: pi.zod.string().optional().describe("Required for create: the model identifier this worker uses."),
    permissionEnvelope: pi.zod.record(pi.zod.string(), pi.zod.unknown()).optional(),
    parentWorkerId: pi.zod.string().optional().describe("Parent worker id, if spawned as a child."),
    workerId: pi.zod.string().optional().describe("Required for get: the worker id to fetch."),
  });

  pi.registerTool({
    name: BATMAN_WORKER_TOOL_NAME,
    label: "BATMAN Worker",
    description: "Creates, lists, or fetches BATMAN worker identities backing OMP-selected harnesses.",
    parameters: params,
    approval: (args) =>
      typeof args === "object" && args !== null && "op" in args && args.op === "create" ? "exec" : "read",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      switch (input.op) {
        case "create":
          return callOrchestration(client, "worker/create", {
            fingerprint: input.fingerprint,
            adapter: input.adapter,
            model: input.model,
            permissionEnvelope: input.permissionEnvelope,
            parentWorkerId: input.parentWorkerId,
          });
        case "list":
          return callOrchestration(client, "worker/list", {});
        case "get":
          return callOrchestration(client, "worker/get", { workerId: input.workerId });
      }
    },
  });
}
