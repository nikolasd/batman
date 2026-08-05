// Registers every deterministic BATMAN orchestration tool: `batman_task`,
// `batman_worker`, `batman_profile`, `batman_run`, `batman_workspace`,
// `batman_artifact`, `batman_child`, `batman_violation`, `batman_message`,
// `batman_approval`, and `batman_reconcile`. Each tool is a thin validated
// adapter over the runtime's JSON-RPC methods -- no worker selection,
// retry, merge, or lifecycle inference happens in TypeScript.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import { registerApprovalTool } from "./approvals";
import { registerArtifactTool } from "./artifacts";
import { registerChildTool } from "./children";
import { registerMessageTool } from "./messages";
import { registerProfileTool } from "./profiles";
import { registerReconcileTool } from "./reconcile";
import { registerRunTool } from "./runs";
import { registerTaskTool } from "./tasks";
import { registerViolationTool } from "./violations";
import { registerWorkerTool } from "./workers";
import { registerWorkspaceTool } from "./workspaces";
import type { OrchestrationToolContext } from "./shared";

export type { OrchestrationToolContext } from "./shared";
export { BATMAN_TASK_TOOL_NAME } from "./tasks";
export { BATMAN_WORKER_TOOL_NAME } from "./workers";
export { BATMAN_RUN_TOOL_NAME } from "./runs";
export { BATMAN_MESSAGE_TOOL_NAME } from "./messages";
export { BATMAN_APPROVAL_TOOL_NAME } from "./approvals";
export { BATMAN_RECONCILE_TOOL_NAME } from "./reconcile";
export { BATMAN_PROFILE_TOOL_NAME } from "./profiles";
export { BATMAN_WORKSPACE_TOOL_NAME } from "./workspaces";
export { BATMAN_ARTIFACT_TOOL_NAME } from "./artifacts";
export { BATMAN_CHILD_TOOL_NAME } from "./children";
export { BATMAN_VIOLATION_TOOL_NAME } from "./violations";

/** Registers every orchestration tool against the extension API. */
export function registerOrchestrationTools(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  // Registration order is the order the model sees these tools in, and is
  // asserted verbatim by `tools.test.ts`: identity, then execution, then
  // the evidence and decision surfaces, then messaging.
  registerTaskTool(pi, ctx);
  registerWorkerTool(pi, ctx);
  registerProfileTool(pi, ctx);
  registerRunTool(pi, ctx);
  registerWorkspaceTool(pi, ctx);
  registerArtifactTool(pi, ctx);
  registerChildTool(pi, ctx);
  registerViolationTool(pi, ctx);
  registerMessageTool(pi, ctx);
  registerApprovalTool(pi, ctx);
  registerReconcileTool(pi, ctx);
}
