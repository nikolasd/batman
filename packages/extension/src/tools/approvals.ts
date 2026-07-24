// `batman_approval`: lists pending approvals and records a human decision.
// The whole tool is gated at tier `exec` with `override: true` -- an
// approval decision is a user-facing safety action that must never
// auto-approve, even for the `list` op.
//
// `decide` checks the approval's `humanRequired` flag before trusting the
// caller-provided decision: when true and interactive UI is available, it
// shows the human approval dialog (see `../approval-ui`) and decides with
// the human's actual answer instead, redacting arguments before display.
// A dialog timeout leaves the request pending rather than falling back to
// the model-provided decision.

import type { AgentToolResult, ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import { showApprovalDialog, type PendingApproval } from "../approval-ui";
import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_APPROVAL_TOOL_NAME = "batman_approval";

/** Fetches the pending approval matching `approvalId`, if still pending. */
async function findPendingApproval(
  client: { request(method: string, params?: unknown): Promise<unknown> },
  approvalId: string,
): Promise<PendingApproval | undefined> {
  const result = await client.request("approval/list", {});
  if (typeof result !== "object" || result === null || !("approvals" in result)) {
    return undefined;
  }
  const approvals = (result as { approvals: unknown }).approvals;
  if (!Array.isArray(approvals)) {
    return undefined;
  }
  const match = approvals.find(
    (entry): entry is Record<string, unknown> =>
      typeof entry === "object" && entry !== null && (entry as Record<string, unknown>).approvalId === approvalId,
  );
  if (match === undefined) {
    return undefined;
  }
  return {
    approvalId,
    action: typeof match.action === "string" ? match.action : "",
    arguments: match.arguments,
    policyReason: typeof match.policyReason === "string" ? match.policyReason : "",
    humanRequired: match.humanRequired === true,
  };
}

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
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx): Promise<AgentToolResult<unknown>> {
      const client = await ctx.getClient(extCtx.cwd);
      if (input.op !== "decide") {
        return callOrchestration(client, "approval/list", { runId: input.runId });
      }
      if (input.approvalId === undefined) {
        return callOrchestration(client, "approval/decide", {
          approvalId: input.approvalId,
          decision: input.decision,
          reason: input.reason,
        });
      }

      if (extCtx.hasUI) {
        const pending = await findPendingApproval(client, input.approvalId);
        if (pending?.humanRequired === true) {
          const human = await showApprovalDialog(extCtx.ui, pending);
          if (human === undefined) {
            return {
              content: [{ type: "text", text: `Approval dialog timed out; ${input.approvalId} remains pending.` }],
              details: { approvalId: input.approvalId, outcome: "pending" },
            };
          }
          return callOrchestration(client, "approval/decide", {
            approvalId: input.approvalId,
            decision: human.decision,
            reason: human.reason,
          });
        }
      }

      return callOrchestration(client, "approval/decide", {
        approvalId: input.approvalId,
        decision: input.decision,
        reason: input.reason,
      });
    },
  });
}
