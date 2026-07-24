// The human approval dialog: shown only when OMP policy indicates
// `humanRequired: true`. Displays worker, requested action, arguments
// after redaction, policy reason, and approval ID. A timeout (the
// underlying `ui.select`/`ui.input` resolving `undefined`) returns no
// decision and leaves the request pending -- it never auto-approves.

import type { ExtensionUIContext } from "@oh-my-pi/pi-coding-agent";

/** The subset of an approval request the dialog needs to render. */
export interface PendingApproval {
  readonly approvalId: string;
  readonly workerId?: string;
  readonly action: string;
  readonly arguments: unknown;
  readonly policyReason: string;
  readonly humanRequired: boolean;
}

/** A human decision collected from the dialog. */
export interface ApprovalDialogResult {
  readonly decision: "approve" | "deny";
  readonly reason: string;
}

/** How long the dialog waits for a selection/reason before timing out. */
export const APPROVAL_DIALOG_TIMEOUT_MS = 5 * 60 * 1000;

const SECRET_KEY_PATTERN = /token|secret|password|apikey|api_key|credential/i;

/**
 * Shows the human approval dialog for `approval`. Returns `undefined`
 * immediately -- without prompting -- when `approval.humanRequired` is
 * false, and also on timeout: in both cases the request is left pending.
 */
export async function showApprovalDialog(
  ui: ExtensionUIContext,
  approval: PendingApproval,
): Promise<ApprovalDialogResult | undefined> {
  if (!approval.humanRequired) {
    return undefined;
  }

  ui.notify(renderApprovalMessage(approval), "info");

  const selection = await ui.select(`Approval required: ${approval.action}`, ["Approve", "Deny"], {
    timeout: APPROVAL_DIALOG_TIMEOUT_MS,
  });
  if (selection === undefined) {
    return undefined;
  }

  const decision: "approve" | "deny" = selection === "Approve" ? "approve" : "deny";
  const reason = await ui.input(
    decision === "approve" ? "Reason for approving" : "Reason for denying",
    "",
    { timeout: APPROVAL_DIALOG_TIMEOUT_MS },
  );
  if (reason === undefined) {
    return undefined;
  }

  return { decision, reason };
}

/**
 * Renders the dialog message: worker, requested action, arguments after
 * redaction, policy reason, and approval ID.
 */
export function renderApprovalMessage(approval: PendingApproval): string {
  const lines: string[] = [`Approval ID: ${approval.approvalId}`];
  if (approval.workerId !== undefined) {
    lines.push(`Worker: ${approval.workerId}`);
  }
  lines.push(`Action: ${approval.action}`);
  lines.push(`Arguments: ${JSON.stringify(redactArguments(approval.arguments))}`);
  lines.push(`Policy reason: ${approval.policyReason}`);
  return lines.join("\n");
}

/**
 * Redacts arguments whose key looks secret-bearing before display. The
 * runtime already redacts secrets before persistence; this is defense in
 * depth for whatever raw shape a caller passes through to the dialog.
 */
function redactArguments(value: unknown): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return value;
  }
  const redacted: Record<string, unknown> = {};
  for (const [key, entryValue] of Object.entries(value as Record<string, unknown>)) {
    redacted[key] = SECRET_KEY_PATTERN.test(key) ? "<redacted>" : entryValue;
  }
  return redacted;
}
