// The `@satori/batman` OMP extension entry point. Registers `batman_status`
// (an LLM-callable tool) and `/batman-status` (a slash command), both backed
// by the single `getRuntimeStatus` path in `status.ts`: OMP loading this
// extension starts or reconnects to the per-repository `batcave` runtime and
// reports its status without any model call.

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

import type { BatmanClient } from "./client";
import { buildStatusContext } from "./context";
import { getRuntimeStatus, type GetRuntimeStatusContext } from "./status";

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

  pi.on("session_shutdown", async () => {
    cachedClient?.close();
    cachedClient = undefined;
  });
}
