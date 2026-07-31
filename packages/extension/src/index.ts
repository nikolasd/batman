// The `@satori/batman` OMP extension entry point. Registers `batman_status`
// (an LLM-callable tool), `/batman-status` (a slash command), and every
// deterministic orchestration tool (`batman_task`, `batman_worker`,
// `batman_run`, `batman_message`, `batman_approval`, `batman_reconcile`).
// All share the single cached-client path: OMP loading this extension
// starts or reconnects to the per-repository `batcave` runtime once per
// session, and every tool reuses that connection.

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import {
  TASK_SUBAGENT_EVENT_CHANNEL,
  TASK_SUBAGENT_LIFECYCLE_CHANNEL,
  TASK_SUBAGENT_PROGRESS_CHANNEL,
  type SubagentEventPayload,
  type SubagentLifecyclePayload,
  type SubagentProgressPayload,
} from "@oh-my-pi/pi-coding-agent/task";

import type { BatmanClient } from "./client";
import { buildStatusContext } from "./context";
import { normalizeEventPayload, normalizeLifecyclePayload, normalizeProgressPayload } from "./omp-native/events";
import { OmpNativeReconciler, createOmpProcessEpoch } from "./omp-native/reconcile";
import { getRuntimeStatus, type GetRuntimeStatusContext } from "./status";
import { runDoctorCommand, buildDoctorContext, type DoctorContext } from "./doctor";
import { registerOrchestrationTools } from "./tools";
import { registerMonitor } from "./monitor/controller";
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
  registerMonitor(pi, { getClient });
  /**
   * Context builder for the doctor command: resolves the batcave binary path
   * and repository state for direct CLI invocation.
   */
  function doctorContextFor(cwd: ExtensionContext["cwd"]): DoctorContext {
    return buildDoctorContext(cwd);
  }

  pi.registerTool({
    name: "batman_doctor",
    label: "BATMAN Doctor",
    description: "Runs diagnostic checks on the BATMAN runtime state and configuration.",
    parameters: pi.zod.object({}),
    async execute(_toolCallId, _params, _signal, _onUpdate, ctx) {
      return runDoctorCommand(doctorContextFor(ctx.cwd));
    },
  });

  pi.registerCommand("batman-doctor", {
    description: "Run diagnostic checks on the BATMAN runtime state and configuration.",
    handler: async (_args, ctx) => {
      const result = await runDoctorCommand(doctorContextFor(ctx.cwd));
      const text = result.content.map((block) => block.text).join("\n");
      if (!ctx.hasUI) {
        console.log(text);
      } else {
        ctx.ui.notify(text, result.isError ? "error" : "info");
      }
    },
  });

  // OMP-native subagent lifecycle mirroring: one epoch per extension
  // process, normalized facts recorded by the reconciler, listeners
  // registered on session_start and removed on session_shutdown.
  const ompProcessEpoch = createOmpProcessEpoch();
  const reconciler = new OmpNativeReconciler();
  let unsubscribers: Array<() => void> = [];

  pi.on("session_start", async () => {
    unsubscribers = [
      // The event bus is untyped (`EventBus.on` receives `unknown`); these
      // three channels are SDK-internal and documented at
      // `@oh-my-pi/pi-coding-agent/task`, with no runtime schema exported to
      // validate against, so the cast is the pinned public contract itself.
      pi.events.on(TASK_SUBAGENT_LIFECYCLE_CHANNEL, (data) => {
        const payload = data as SubagentLifecyclePayload;
        reconciler.record(normalizeLifecyclePayload(payload, ompProcessEpoch, Date.now()));
      }),
      pi.events.on(TASK_SUBAGENT_PROGRESS_CHANNEL, (data) => {
        const payload = data as SubagentProgressPayload;
        reconciler.record(normalizeProgressPayload(payload, ompProcessEpoch, Date.now()));
      }),
      pi.events.on(TASK_SUBAGENT_EVENT_CHANNEL, (data) => {
        const payload = data as SubagentEventPayload;
        const fact = normalizeEventPayload(payload);
        if (fact !== undefined) {
          reconciler.record(fact);
        }
      }),
    ];
  });

  pi.on("session_shutdown", async () => {
    cachedClient?.close();
    cachedClient = undefined;
    for (const unsubscribe of unsubscribers) {
      unsubscribe();
    }
    unsubscribers = [];
    reconciler.dispose();
  });
}

// Export conformance utilities for external use.
export { runConformance, formatConformanceSummary } from "./conformance";
export type { ConformanceConfig, ConformanceReport, ConformanceTestResult, AdapterKind, ConformanceMode } from "./conformance";
