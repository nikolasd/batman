// `batman_artifact`: lists and fetches artifacts workers published for the
// current task (patches, commit lists, conflict reports, workspace
// manifests). Both ops are tier `read` -- neither mutates anything; fetching
// an artifact only streams bytes the runtime already stored.

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

import type { OrchestrationToolContext } from "./shared";
import { callOrchestration } from "./shared";

export const BATMAN_ARTIFACT_TOOL_NAME = "batman_artifact";

export function registerArtifactTool(pi: ExtensionAPI, ctx: OrchestrationToolContext): void {
  const params = pi.zod.object({
    op: pi.zod.enum(["list", "fetch"]).describe("Which artifact operation to perform."),
    // Closed enum on purpose: the runtime maps exactly these four strings
    // and silently treats anything else as "no filter", so an open string
    // would let a typo return every artifact while appearing to filter.
    kind: pi.zod.enum(["patch", "commitList", "conflictReport", "workspaceManifest"]).optional().describe("Optional filter for list: only return artifacts of this kind. Omit to list every kind."),
    artifactId: pi.zod.string().optional().describe("Required for fetch: the artifact id to read."),
    offset: pi.zod.number().int().optional().describe("Optional for fetch: byte offset to start from. Defaults to 0."),
    length: pi.zod.number().int().optional().describe("Optional for fetch: how many bytes to read. The runtime caps this; the response's nextOffset says where to resume."),
  });

  pi.registerTool({
    name: BATMAN_ARTIFACT_TOOL_NAME,
    label: "BATMAN Artifact",
    description:
      "Use to read the evidence a worker produced: patches, commit lists, conflict reports, and workspace manifests. Use op: 'list' to see what a run published (optionally filtered by kind), then op: 'fetch' with an artifactId to read its bytes. Fetches are chunked -- the response carries nextOffset, so pass it back as offset to continue reading a large artifact. Artifacts are scoped to the current task; a run on another task is never visible.",
    parameters: params,
    approval: "read",
    async execute(_toolCallId, input, _signal, _onUpdate, extCtx) {
      const client = await ctx.getClient(extCtx);
      switch (input.op) {
        case "list":
          return callOrchestration(client, "artifact/list", { kind: input.kind });
        case "fetch":
          return callOrchestration(client, "artifact/fetch", {
            artifactId: input.artifactId,
            offset: input.offset,
            length: input.length,
          });
      }
    },
  });
}
