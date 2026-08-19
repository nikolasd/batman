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
  getClient(extCtx: ExtensionContext): Promise<BatmanClient>;
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
  let subscribedClient: BatmanClient | undefined;

  function refresh(extCtx: ExtensionContext): void {
    extCtx.ui.setWidget(WIDGET_KEY, renderWidgetBox(controller.getState(), extCtx.ui.theme), { placement: "aboveEditor" });
    pi.appendEntry(MONITOR_ENTRY_TYPE, { sequence: Number(controller.getState().lastSequence) });
  }

  async function connect(extCtx: ExtensionContext): Promise<void> {
    if (subscribedClient !== undefined && !subscribedClient.isClosed) {
      return;
    }
    if (subscribedClient !== undefined) {
      // The prior subscription is dead with it; drop it before resubscribing.
      controller.stop();
      subscribedClient = undefined;
    }
    // Resume from whichever is further ahead: what was persisted to the
    // session log, or what this controller has already reduced in memory.
    // `reduceEvent` ignores an event at or below a row's applied sequence,
    // so overlapping replay is a no-op rather than a double-count.
    const fromSequence = Math.max(lastPersistedSequence(extCtx.sessionManager.getEntries() as SessionEntryLike[]), Number(controller.getState().lastSequence));
    try {
      const client = await ctx.getClient(extCtx);
      controller.start(client, fromSequence, () => refresh(extCtx));
      subscribedClient = client;
    } catch (err) {
      pi.logger.warn("batman monitor: runtime unavailable", {
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  pi.on("session_start", async (_event, extCtx) => {
    await connect(extCtx);
    // Render immediately: a healthy runtime with no runs must show
    // "No BATMAN runs yet." rather than nothing until an event fires (R56).
    if (subscribedClient !== undefined) {
      refresh(extCtx);
    }
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
      // Deliberately asymmetric with session_start's guarded refresh: an
      // explicit user command renders unconditionally, so /batman against a
      // dead runtime still shows the (empty) monitor box rather than nothing.
      await connect(cmdCtx);
      refresh(cmdCtx);
    },
  });

  pi.on("session_shutdown", async () => {
    controller.stop();
    // Drop the client reference too, exactly as the dead-subscription repair
    // path in connect() does -- otherwise a later connect() early-returns
    // into a monitor whose subscription no longer exists (R39).
    subscribedClient = undefined;
  });
}
