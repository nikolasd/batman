// Renders `MonitorState` rows into the widget's concise text lines.
// `ctx.ui.setWidget` accepts at most 10 rows; a fuller view is a
// `/batman status <runId>` command lookup, never silent truncation of
// state (the model itself is unbounded, only the *rendered* widget is
// capped).

import type { Theme, ThemeColor } from "@oh-my-pi/pi-coding-agent";

import type { MonitorRow, MonitorState } from "./model";

/** The widget never renders more than this many rows at once. */
export const MAX_WIDGET_ROWS = 10;

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

/**
 * Renders up to {@link MAX_WIDGET_ROWS} concise lines, most-recently
 * active first. An empty state renders a single explanatory line rather
 * than an empty widget.
 */
export function renderWidgetLines(state: MonitorState): string[] {
  const rows = Object.values(state.rows).sort((a, b) => (a.lastEventAt < b.lastEventAt ? 1 : -1));
  if (rows.length === 0) {
    return ["No BATMAN runs yet."];
  }
  const visible = rows.slice(0, MAX_WIDGET_ROWS);
  const lines = visible.map(renderRowLine);
  if (rows.length > MAX_WIDGET_ROWS) {
    lines.push(`… ${rows.length - MAX_WIDGET_ROWS} more; use /batman status <runId> for full details.`);
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
