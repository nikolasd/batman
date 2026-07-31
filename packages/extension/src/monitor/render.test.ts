import { expect, test } from "bun:test";

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

import { assertCompatiblePiCodingAgentVersion, PiCodingAgentVersionError } from "./compat";
import type { MonitorRow, MonitorState } from "./model";
import {
  MAX_WIDGET_ROWS,
  renderRowDetails,
  renderRowLine,
  renderWidgetLines,
  stateIcon,
  stateColor,
} from "./render";

function row(overrides: Partial<MonitorRow>): MonitorRow {
  return {
    runId: "run-1",
    taskId: "task-1",
    workerId: "worker-1",
    state: "working",
    flags: {
      degradedControl: false,
      needsReconciliation: false,
      protocolUnhealthy: false,
      policyQuarantined: false,
      workspaceDirty: false,
      childrenActive: false,
    },
    pendingApprovalCount: 0,
    firstSeenAt: "2026-01-01T00:00:00Z",
    lastEventAt: "2026-01-01T00:00:00Z",
    lastAppliedSequence: 1n,
    ...overrides,
  };
}

function stateOf(rows: readonly MonitorRow[]): MonitorState {
  const byId: Record<string, MonitorRow> = {};
  for (const r of rows) {
    byId[r.runId] = r;
  }
  return { rows: byId, lastSequence: 1n };
}

test("an empty state renders a single explanatory line", () => {
  const lines = renderWidgetLines({ rows: {}, lastSequence: 0n });
  expect(lines).toEqual(["No BATMAN runs yet."]);
});

test("renders at most MAX_WIDGET_ROWS lines with an overflow indicator", () => {
  const rows = Array.from({ length: MAX_WIDGET_ROWS + 3 }, (_, i) =>
    row({ runId: `run-${i}`, lastEventAt: `2026-01-01T00:${String(i).padStart(2, "0")}:00Z` }),
  );
  const lines = renderWidgetLines(stateOf(rows));
  expect(lines).toHaveLength(MAX_WIDGET_ROWS + 1);
  expect(lines[lines.length - 1]).toContain("more");
  expect(lines[lines.length - 1]).toContain("/batman status");
});

test("a row line includes state, harness/model, flags, and pending approvals", () => {
  const line = renderRowLine(
    row({
      state: "waitingUser",
      adapter: "claude",
      model: "claude-sonnet-4",
      flags: {
        degradedControl: true,
        needsReconciliation: false,
        protocolUnhealthy: false,
        policyQuarantined: false,
        workspaceDirty: false,
        childrenActive: false,
      },
      pendingApprovalCount: 2,
    }),
  );
  expect(line).toContain("waitingUser");
  expect(line).toContain("claude/claude-sonnet-4");
  expect(line).toContain("degraded");
  expect(line).toContain("2 pending approvals");
});

test("stateIcon returns the documented codepoint for every known run state", () => {
  expect(stateIcon("queued")).toBe("\u{F0150}");
  expect(stateIcon("starting")).toBe("\u{F14DF}");
  expect(stateIcon("working")).toBe("\u{F1461}");
  expect(stateIcon("waitingUser")).toBe("\u{F0B5A}");
  expect(stateIcon("waitingPeer")).toBe("\u{F000F}");
  expect(stateIcon("paused")).toBe("\u{F03E6}");
  expect(stateIcon("succeeded")).toBe("\u{F05E1}");
  expect(stateIcon("failed")).toBe("\u{F015A}");
  expect(stateIcon("cancelled")).toBe("\u{F073A}");
  expect(stateIcon("lost")).toBe("\u{F0BA6}");
});

test("stateIcon falls back to a generic icon for an unrecognized state", () => {
  expect(stateIcon("totally-unknown")).toBe("\u{F0625}");
});

test("stateColor returns the documented theme color for every known run state", () => {
  expect(stateColor("queued")).toBe("muted");
  expect(stateColor("starting")).toBe("accent");
  expect(stateColor("working")).toBe("accent");
  expect(stateColor("waitingUser")).toBe("warning");
  expect(stateColor("waitingPeer")).toBe("warning");
  expect(stateColor("paused")).toBe("muted");
  expect(stateColor("succeeded")).toBe("success");
  expect(stateColor("failed")).toBe("error");
  expect(stateColor("cancelled")).toBe("dim");
  expect(stateColor("lost")).toBe("error");
});

test("stateColor falls back to the theme's default text color for an unrecognized state", () => {
  expect(stateColor("totally-unknown")).toBe("text");
});

test("a row line includes the state icon alongside the state word", () => {
  const line = renderRowLine(row({ state: "succeeded" }));
  expect(line).toContain(`${stateIcon("succeeded")} succeeded`);
});

test("renderRowDetails includes worker, action-relevant fields, and timestamps for /batman status", () => {
  const details = renderRowDetails(
    row({ workspaceMode: "isolated", latestActivity: "question sent", adapter: "codex", model: "gpt-5" }),
  );
  expect(details).toContain("Run: run-1");
  expect(details).toContain("Task: task-1");
  expect(details).toContain("Worker: worker-1");
  expect(details).toContain("Harness/model: codex/gpt-5");
  expect(details).toContain("Workspace mode: isolated");
  expect(details).toContain("Latest activity: question sent");
  expect(details).toContain("First seen:");
  expect(details).toContain("Last event:");
});

// ------------------------------------------- version compatibility check

test("the installed @oh-my-pi/pi-coding-agent is within the supported range", () => {
  expect(() => assertCompatiblePiCodingAgentVersion()).not.toThrow();
});

test("a version outside the supported range throws a named PiCodingAgentVersionError", () => {
  expect(() => assertCompatiblePiCodingAgentVersion("16.9.0")).toThrow(PiCodingAgentVersionError);
  expect(() => assertCompatiblePiCodingAgentVersion("18.0.0")).toThrow(PiCodingAgentVersionError);
});

test("a version at the exact lower bound is accepted", () => {
  expect(() => assertCompatiblePiCodingAgentVersion("17.0.7")).not.toThrow();
});

// ---------------------------------- no-model fixture extension compile check

test("a no-model fixture extension compiles and runs pi.appendEntry + ctx.ui.setWidget against the installed OMP surface", () => {
  assertCompatiblePiCodingAgentVersion();

  const appendedEntries: Array<{ customType: string; data: unknown }> = [];
  const widgets: Array<{ key: string; content: unknown; options: unknown }> = [];

  const fakePi = {
    appendEntry: (customType: string, data?: unknown) => {
      appendedEntries.push({ customType, data });
    },
  } as unknown as ExtensionAPI;

  const fakeCtx = {
    ui: {
      setWidget: (key: string, content: unknown, options?: unknown) => {
        widgets.push({ key, content, options });
      },
    },
  } as unknown as ExtensionContext;

  // The exact calls the plan pins to OMP 17.0.7's public surface.
  function fixtureExtension(pi: ExtensionAPI, ctx: ExtensionContext): void {
    pi.appendEntry("batman-monitor", { sequence: 1 });
    ctx.ui.setWidget("batman-monitor", ["fixture"], { placement: "aboveEditor" });
  }

  fixtureExtension(fakePi, fakeCtx);

  expect(appendedEntries).toEqual([{ customType: "batman-monitor", data: { sequence: 1 } }]);
  expect(widgets).toEqual([{ key: "batman-monitor", content: ["fixture"], options: { placement: "aboveEditor" } }]);
});
