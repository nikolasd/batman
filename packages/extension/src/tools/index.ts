// Registers every deterministic BATMAN orchestration tool: `batman_task`,
// `batman_worker`, `batman_run`, `batman_message`, `batman_approval`,
// `batman_reconcile`, `batman_profile`, and `batman_workspace`. Each tool is
// a thin validated adapter over the runtime's JSON-RPC methods -- no worker
// selection, retry, merge, or lifecycle inference happens in TypeScript.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import { registerApprovalTool } from "./approvals";
import { registerMessageTool } from "./messages";
import { registerProfileTool } from "./profiles";
import { registerReconcileTool } from "./reconcile";
import { registerRunTool } from "./runs";
import { registerTaskTool } from "./tasks";
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

/** Registers every orchestration tool against the extension API. */
export function registerOrchestrationTools(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  registerTaskTool(pi, ctx);
  registerWorkerTool(pi, ctx);
  registerRunTool(pi, ctx);
  registerMessageTool(pi, ctx);
  registerApprovalTool(pi, ctx);
  registerReconcileTool(pi, ctx);
  registerProfileTool(pi, ctx);
  registerWorkspaceTool(pi, ctx);
}
