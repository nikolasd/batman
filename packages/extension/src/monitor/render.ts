// Renders `MonitorState` rows into the widget's concise text lines.
// `ctx.ui.setWidget` accepts at most 10 rows; a fuller view is a
// `/batman status <runId>` command lookup, never silent truncation of
// state (the model itself is unbounded, only the *rendered* widget is
// capped).

import type { Theme, ThemeColor } from "@oh-my-pi/pi-coding-agent";

import type { MonitorRow, MonitorState } from "./model";

/** The widget never renders more than this many rows at once. */
export const MAX_WIDGET_ROWS = 10;

/**
 * Counts Unicode code points rather than UTF-16 code units. Every Nerd Font
 * icon this module uses (`BAT_ICON`, every `STATE_ICONS` entry) is on the
 * astral plane (code point > U+FFFF), so it's stored as a UTF-16 surrogate
 * pair — `"\u{F0B5F}".length === 2`, not 1. `.length`-based width/pad math
 * would measure any icon-bearing string 1 unit too wide per icon relative to
 * how many character cells it actually occupies. `Array.from` iterates a
 * string by code point, correctly counting each surrogate pair as one unit.
 */
function codePointLength(text: string): number {
  return Array.from(text).length;
}

const BAT_ICON = "\u{F0B5F}";
const WIDGET_HEADER_TEXT = "BATMAN";

const STATE_ICONS: Record<string, string> = {
  queued: "\u{F0150}",
  starting: "\u{F14DF}",
  working: "\u{F1461}",
  waitingUser: "\u{F0B5A}",
  waitingPeer: "\u{F000F}",
  paused: "\u{F03E6}",
  succeeded: "\u{F05E1}",
  failed: "\u{F015A}",
  cancelled: "\u{F073A}",
  lost: "\u{F0BA6}",
};
const FALLBACK_STATE_ICON = "\u{F0625}";

/**
 * Nerd Font icon for a run state, or a generic fallback for a state this
 * lookup doesn't recognize. `MonitorRow.state` is a plain `string` (the Rust
 * `RunState` is a newtype around `String`, not a closed enum), so this can
 * never be an exhaustive switch.
 */
export function stateIcon(state: string): string {
  return STATE_ICONS[state] ?? FALLBACK_STATE_ICON;
}

const STATE_COLORS: Record<string, ThemeColor> = {
  queued: "muted",
  starting: "accent",
  working: "accent",
  waitingUser: "warning",
  waitingPeer: "warning",
  paused: "muted",
  succeeded: "success",
  failed: "error",
  cancelled: "dim",
  lost: "error",
};
const FALLBACK_STATE_COLOR: ThemeColor = "text";

/** Theme color for a run state, or the theme's default text color for a
 *  state this lookup doesn't recognize. */
export function stateColor(state: string): ThemeColor {
  return STATE_COLORS[state] ?? FALLBACK_STATE_COLOR;
}

/** The widget's brand header: bat icon + "BATMAN", uncolored — the caller
 *  (`renderWidgetBox`) applies theme color, so this stays a plain data
 *  producer with no `Theme` dependency of its own. */
export function renderWidgetHeader(): string {
  return `${BAT_ICON} ${WIDGET_HEADER_TEXT}`;
}

/**
 * Renders up to {@link MAX_WIDGET_ROWS} concise lines, most-recently
 * active first. An empty state renders a single explanatory line rather
 * than an empty widget.
 */
/** Sorts rows most-recently-active first and caps the visible slice at
 *  {@link MAX_WIDGET_ROWS}, returning the total count separately so callers
 *  can still detect and report truncation. Shared by `renderWidgetLines`
 *  and `renderWidgetBox` so they can never disagree on which rows are
 *  visible. */
function selectRows(state: MonitorState): { rows: MonitorRow[]; totalCount: number } {
  const rows = Object.values(state.rows).sort((a, b) => (a.lastEventAt < b.lastEventAt ? 1 : -1));
  return { rows: rows.slice(0, MAX_WIDGET_ROWS), totalCount: rows.length };
}

export function renderWidgetLines(state: MonitorState): string[] {
  const { rows, totalCount } = selectRows(state);
  if (totalCount === 0) {
    return ["No BATMAN runs yet."];
  }
  const lines = rows.map(renderRowLine);
  if (totalCount > MAX_WIDGET_ROWS) {
    lines.push(`… ${totalCount - MAX_WIDGET_ROWS} more; use /batman status <runId> for full details.`);
  }
  return lines;
}

/** Renders one row as a single concise line. */
export function renderRowLine(row: MonitorRow): string {
  const parts = [shortId(row.runId), `${stateIcon(row.state)} ${row.state}`];
  const harness = harnessLabel(row);
  if (harness !== undefined) {
    parts.push(harness);
  }
  const flags = activeFlagLabels(row.flags);
  if (flags.length > 0) {
    parts.push(`[${flags.join(",")}]`);
  }
  if (row.pendingApprovalCount > 0) {
    parts.push(`${row.pendingApprovalCount} pending approval${row.pendingApprovalCount === 1 ? "" : "s"}`);
  }
  if (row.workspaceMode !== undefined) {
    parts.push(row.workspaceMode);
  }
  if (row.latestActivity !== undefined) {
    parts.push(row.latestActivity);
  }
  return parts.join(" · ");
}

/**
 * Assembles a rounded box around `lines`, each colored per `colors[i]`, with
 * `header` spliced into the top border itself (matching the app's own
 * editor chrome, which embeds its status segments in its top border rather
 * than rendering them as a separate row) rather than as a separate row
 * inside the box. `width` is chosen so the top border, every content line,
 * and the bottom border all come out to the same total length: the content
 * requirement is `longest line + 2` (one space of padding on each side);
 * the header requirement is `header + 4` (corner, one leading dash, one
 * space on each side of the header, before the closing corner) — whichever
 * is larger wins. Requires `lines` to be non-empty (both `renderWidgetBox`
 * call sites always pass at least the empty-state line).
 */
function assembleBox(header: string, lines: string[], colors: ThemeColor[], theme: Theme): string[] {
  const { topLeft, topRight, bottomLeft, bottomRight, horizontal, vertical } = theme.boxRound;
  const contentWidth = Math.max(...lines.map((line) => codePointLength(line))) + 2;
  const width = Math.max(contentWidth, codePointLength(header) + 4);

  const top =
    theme.fg("border", `${topLeft}${horizontal} `) +
    theme.fg("accent", header) +
    theme.fg("border", ` ${horizontal.repeat(width - codePointLength(header) - 3)}${topRight}`);

  const body = lines.map((line, index) => {
    const pad = width - codePointLength(line) - 1;
    return (
      theme.fg("border", vertical) +
      " " +
      theme.fg(colors[index] ?? "text", line) +
      " ".repeat(pad) +
      theme.fg("border", vertical)
    );
  });

  const bottom = theme.fg("border", `${bottomLeft}${horizontal.repeat(width)}${bottomRight}`);

  return [top, ...body, bottom];
}

/**
 * The full bordered widget: a title-in-top-border box wrapping the same
 * content `renderWidgetLines` produces, with each row additionally colored
 * by `stateColor`. This is what `controller.ts` passes to `ui.setWidget`.
 */
export function renderWidgetBox(state: MonitorState, theme: Theme): string[] {
  const { rows, totalCount } = selectRows(state);

  let lines: string[];
  let colors: ThemeColor[];
  if (totalCount === 0) {
    lines = ["No BATMAN runs yet."];
    colors = ["text"];
  } else {
    lines = rows.map(renderRowLine);
    colors = rows.map((row) => stateColor(row.state));
    if (totalCount > MAX_WIDGET_ROWS) {
      lines.push(`… ${totalCount - MAX_WIDGET_ROWS} more; use /batman status <runId> for full details.`);
      colors.push("muted");
    }
  }

  return assembleBox(renderWidgetHeader(), lines, colors, theme);
}

/** Renders the full detail block for `/batman status <runId>`. */
export function renderRowDetails(row: MonitorRow): string {
  const lines = [
    `Run: ${row.runId}`,
    `Task: ${row.taskId}`,
    `Worker: ${row.workerId}`,
    `State: ${row.state}`,
  ];
  const harness = harnessLabel(row);
  if (harness !== undefined) {
    lines.push(`Harness/model: ${harness}`);
  }
  const flags = activeFlagLabels(row.flags);
  lines.push(`Flags: ${flags.length > 0 ? flags.join(", ") : "none"}`);
  lines.push(`Pending approvals: ${row.pendingApprovalCount}`);
  if (row.workspaceMode !== undefined) {
    lines.push(`Workspace mode: ${row.workspaceMode}`);
  }
  if (row.latestActivity !== undefined) {
    lines.push(`Latest activity: ${row.latestActivity}`);
  }
  lines.push(`First seen: ${row.firstSeenAt}`);
  lines.push(`Last event: ${row.lastEventAt}`);
  return lines.join("\n");
}

function harnessLabel(row: MonitorRow): string | undefined {
  if (row.adapter === undefined) {
    return undefined;
  }
  return row.model === undefined ? row.adapter : `${row.adapter}/${row.model}`;
}

function activeFlagLabels(flags: MonitorRow["flags"]): string[] {
  const labels: string[] = [];
  if (flags.degradedControl) labels.push("degraded");
  if (flags.needsReconciliation) labels.push("needsReconciliation");
  if (flags.protocolUnhealthy) labels.push("protocolUnhealthy");
  if (flags.policyQuarantined) labels.push("policyQuarantined");
  if (flags.workspaceDirty) labels.push("workspaceDirty");
  if (flags.childrenActive) labels.push("childrenActive");
  return labels;
}

function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}
