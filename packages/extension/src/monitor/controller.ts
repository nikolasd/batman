// Wires the monitor's pure model/render layers into the live extension:
// replay-first startup (resuming from the last persisted sequence),
// continuous widget updates as events arrive, and the `/batman` /
// `/batman status <runId>` commands.

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

import type { BatmanClient } from "../client";
import { EMPTY_MONITOR_STATE, reduceEvent, type MonitorState } from "./model";
import { renderRowDetails, renderWidgetBox } from "./render";

/** The custom session-entry type the last-rendered sequence is persisted under. */
export const MONITOR_ENTRY_TYPE = "batman-monitor";

/** The widget key the monitor renders under. */
const WIDGET_KEY = "batman-monitor";

/** The slash command that opens or refreshes the monitor. */
export const MONITOR_COMMAND_NAME = "batman";

export interface MonitorControllerContext {
  getClient(cwd: string): Promise<BatmanClient>;
}

/** The subset of `pi.appendEntry`'s session-entry log the controller reads
 *  back on startup to resume from the last rendered sequence. */
export interface SessionEntryLike {
  readonly type: string;
  readonly customType?: string;
  readonly data?: unknown;
}

/**
 * Scans `entries` (oldest to newest, as `getEntries()` returns them) for
 * the most recent `batman-monitor` custom entry and returns its persisted
 * sequence, or `0` if none exists yet.
 */
export function lastPersistedSequence(entries: readonly SessionEntryLike[]): number {
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry?.type === "custom" && entry.customType === MONITOR_ENTRY_TYPE) {
      const data = entry.data as { sequence?: unknown } | undefined;
      if (typeof data?.sequence === "number") {
        return data.sequence;
      }
    }
  }
  return 0;
}

/**
 * Owns the monitor's replayable state and keeps the embedded widget in
 * sync as events arrive. One instance per OMP session.
 */
export class MonitorController {
  #state: MonitorState = EMPTY_MONITOR_STATE;
  #unsubscribe: (() => void) | undefined;
  #onUpdate: (() => void) | undefined;

  /** The current replayable state (read-only view for tests/commands). */
  getState(): MonitorState {
    return this.#state;
  }

  /**
   * Subscribes from `fromSequence`, rebuilding state from replay before
   * live notifications arrive (both flow through the same reducer, so
   * there is no separate "replay mode"). Calls `onUpdate` after every
   * applied event so the caller can re-render the widget and persist the
   * new sequence.
   */
  start(client: BatmanClient, fromSequence: number, onUpdate: () => void): void {
    this.#onUpdate = onUpdate;
    this.#unsubscribe = client.subscribe(fromSequence, (event) => {
      this.#state = reduceEvent(this.#state, event);
      this.#onUpdate?.();
    });
  }

  /** Unsubscribes from the runtime. Call on `session_shutdown`. */
  stop(): void {
    this.#unsubscribe?.();
    this.#unsubscribe = undefined;
    this.#onUpdate = undefined;
  }

  /** Full detail text for `/batman status <runId>`, or `undefined` if no
   *  row exists for that run. */
  renderStatus(runId: string): string | undefined {
    const row = this.#state.rows[runId];
    return row === undefined ? undefined : renderRowDetails(row);
  }
}

/** Registers the `/batman` command and the replay-first monitor lifecycle. */
export function registerMonitor(pi: ExtensionAPI, ctx: MonitorControllerContext): void {
  const controller = new MonitorController();
  let connected = false;

  function refresh(extCtx: ExtensionContext): void {
    extCtx.ui.setWidget(WIDGET_KEY, renderWidgetBox(controller.getState(), extCtx.ui.theme), { placement: "aboveEditor" });
    pi.appendEntry(MONITOR_ENTRY_TYPE, { sequence: Number(controller.getState().lastSequence) });
  }

  async function connect(extCtx: ExtensionContext): Promise<void> {
    if (connected) {
      return;
    }
    const fromSequence = lastPersistedSequence(extCtx.sessionManager.getEntries() as SessionEntryLike[]);
    try {
      const client = await ctx.getClient(extCtx.cwd);
      controller.start(client, fromSequence, () => refresh(extCtx));
      connected = true;
    } catch (err) {
      // The runtime is not reachable yet (e.g. no batcave binary available
      // in this environment). The monitor degrades to inactive rather than
      // failing session startup; `/batman` retries the connection.
      pi.logger.warn("batman monitor: runtime unavailable", {
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  pi.on("session_start", async (_event, extCtx) => {
    await connect(extCtx);
  });

  pi.registerCommand(MONITOR_COMMAND_NAME, {
    description: "Opens or refreshes the embedded BATMAN worker monitor. `/batman status <runId>` shows full details.",
    handler: async (args, cmdCtx) => {
      const [sub, runId] = args.trim().split(/\s+/, 2);
      if (sub === "status" && runId !== undefined && runId.length > 0) {
        const details = controller.renderStatus(runId);
        cmdCtx.ui.notify(details ?? `No BATMAN run found for ${runId}.`, details === undefined ? "warning" : "info");
        return;
      }
      await connect(cmdCtx);
      refresh(cmdCtx);
    },
  });

  pi.on("session_shutdown", async () => {
    controller.stop();
  });
}
