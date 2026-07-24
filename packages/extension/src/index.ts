// The `@satori/batman` OMP extension entry point. Registers `batman_status`
// (an LLM-callable tool), `/batman-status` (a slash command), and every
// deterministic orchestration tool (`batman_task`, `batman_worker`,
// `batman_run`, `batman_message`, `batman_approval`, `batman_reconcile`).
// All share the single cached-client path: OMP loading this extension
// starts or reconnects to the per-repository `batcave` runtime once per
// session, and every tool reuses that connection.

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

import type { BatmanClient } from "./client";
import { buildStatusContext } from "./context";
import { getRuntimeStatus, type GetRuntimeStatusContext } from "./status";
import { registerOrchestrationTools } from "./tools";
import { ensureRuntime } from "./runtime";

const TOOL_NAME = "batman_status";
const COMMAND_NAME = "batman-status";
const STATUS_DESCRIPTION = "Reports the status of the local BATMAN runtime for this repository.";

export default function batmanExtension(pi: ExtensionAPI): void {
  // Cached per extension instance (one per OMP session), closed on shutdown.
  let cachedClient: BatmanClient | undefined;

  function statusContextFor(cwd: ExtensionContext["cwd"]): GetRuntimeStatusContext {
    const { ensureRuntimeOptions } = buildStatusContext({ cwd });
    return {
      ensureRuntimeOptions,
      cache: {
        get: () => cachedClient,
        set: (client) => {
          cachedClient = client;
        },
      },
    };
  }

  /**
   * Resolves the cached client for `cwd`, connecting (or spawning) the
   * repository's runtime on first use. Shared by every orchestration tool so
   * a session holds exactly one runtime connection.
   */
  async function getClient(cwd: string): Promise<BatmanClient> {
    if (cachedClient !== undefined) {
      return cachedClient;
    }
    const { ensureRuntimeOptions } = buildStatusContext({ cwd });
    const { client } = await ensureRuntime(ensureRuntimeOptions);
    cachedClient = client;
    return client;
  }

  pi.registerTool({
    name: TOOL_NAME,
    label: "BATMAN Status",
    description: STATUS_DESCRIPTION,
    parameters: pi.zod.object({}),
    async execute(_toolCallId, _params, _signal, _onUpdate, ctx) {
      return getRuntimeStatus(statusContextFor(ctx.cwd));
    },
  });

  pi.registerCommand(COMMAND_NAME, {
    description: STATUS_DESCRIPTION,
    handler: async (_args, ctx) => {
      const result = await getRuntimeStatus(statusContextFor(ctx.cwd));
      const text = result.content.map((block) => block.text).join("\n");
      // `ctx.ui.notify` is a no-op outside interactive mode (print/RPC), so
      // write directly to stdout instead when there is no UI -- this is the
      // only way `--print` surfaces output for a locally-handled slash
      // command. In interactive mode, raw console.log would corrupt the TUI,
      // so route exclusively through `ctx.ui.notify`.
      if (!ctx.hasUI) {
        console.log(text);
      } else {
        ctx.ui.notify(text, result.isError ? "error" : "info");
      }
    },
  });

  registerOrchestrationTools(pi, { getClient });

  pi.on("session_shutdown", async () => {
    cachedClient?.close();
    cachedClient = undefined;
  });
}
