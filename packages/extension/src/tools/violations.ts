// `batman_violation`: decides a recorded policy violation. A winning
// "release" resolves that specific violation but lifts quarantine only if
// it was the *last* unresolved violation on the run -- a different,
// still-open violation on the same run keeps it quarantined even though
// this one was decided. A "cancel" ends the run outright. Tier `exec` --
// a decision resumes or kills real work.
//
// Single op by design: the protocol has only `policy/violation/decide`.
// There is no list RPC, so violations are discovered from the event stream
// (the monitor renders them) rather than polled here.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_VIOLATION_TOOL_NAME = "batman_violation";

export function registerViolationTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["decide"]).describe("Which violation operation to perform."),
    violationId: pi.zod.string().describe("The recorded violation to decide."),
    resolution: pi.zod.string().describe("How the violation is resolved, e.g. release the quarantined run or cancel it."),
  });

  pi.registerTool({
    name: BATMAN_VIOLATION_TOOL_NAME,
    label: "BATMAN Violation",
    description:
      "Use to resolve a policy violation that quarantined a run -- for example a worker that spawned a nested child when policy forbids it. Pass the violationId from the violation event and a resolution describing the decision. The deciding identity is taken from your session automatically. A \"release\" only lifts quarantine if this was the last unresolved violation on the run -- check the result's quarantineCleared field (true/false/absent) to tell whether it did; if false, a different violation is still open and must be found via the event stream or the /batman monitor. Until every violation on a run is decided, the run makes no further progress.",
    parameters: params,
    approval: "exec",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx);
      // The runtime takes the deciding identity from the connection
      // principal, so no owner field is sent: an OMP-supplied identity
      // would be unverified and could impersonate another instance.
      return callOrchestration(client, "policy/violation/decide", {
        violationId: input.violationId,
        resolution: input.resolution,
      });
    },
  });
}
