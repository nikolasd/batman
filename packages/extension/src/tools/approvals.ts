// `batman_approval`: lists pending approvals and records a human decision.
// The whole tool is gated at tier `exec` with `override: true` -- an
// approval decision is a user-facing safety action that must never
// auto-approve, even for the `list` op.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_APPROVAL_TOOL_NAME = "batman_approval";

export function registerApprovalTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["list", "decide"]).describe("Which approval operation to perform."),
    runId: pi.zod.string().optional().describe("Optional run id filter for list."),
    approvalId: pi.zod.string().optional().describe("Required for decide: the approval request id."),
    decision: pi.zod.enum(["approve", "deny"]).optional().describe("Required for decide: approve or deny."),
    reason: pi.zod.string().optional().describe("Required for decide: the reason for this decision."),
  });

  pi.registerTool({
    name: BATMAN_APPROVAL_TOOL_NAME,
    label: "BATMAN Approval",
    description:
      "Lists pending BATMAN approval requests or records a human approve/deny decision. Never auto-approves.",
    parameters: params,
    approval: { tier: "exec", override: true, reason: "Approval decisions are a user-facing safety action." },
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx.cwd);
      if (input.op === "decide") {
        return callOrchestration(client, "approval/decide", {
          approvalId: input.approvalId,
          decision: input.decision,
          reason: input.reason,
        });
      }
      return callOrchestration(client, "approval/list", { runId: input.runId });
    },
  });
}
