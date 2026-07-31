# BATMAN Monitor Widget Visual Formatting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the `/batman` embedded monitor widget Nerd Font icons and a bordered, color-coded
box (matching the app's own status-line chrome), so it reads as a distinct UI element instead of
a plain text notification.

**Architecture:** `packages/extension/src/monitor/render.ts` stays a pure, framework-independent
module — it gains icon/color lookups keyed by run state, a header string, and a new
`renderWidgetBox(state, theme)` that assembles a hand-drawn rounded box (title spliced into the
top border, exactly like the app's own editor chrome) around the same content
`renderWidgetLines` already produces. `packages/extension/src/monitor/controller.ts` changes in
exactly one place: `refresh()` calls `renderWidgetBox(controller.getState(), extCtx.ui.theme)`
instead of `controller.renderLines()`, and the now-dead `renderLines()` method is removed.

**Tech Stack:** TypeScript, Bun test runner, `@oh-my-pi/pi-coding-agent`'s `Theme`/`ThemeColor`
types (already a dependency — no new packages).

## Global Constraints

- Scope is the `/batman` widget only. Do not touch `renderRowDetails` or the
  `/batman status <runId>` detail block, and do not touch `/batman-status`.
- The icon is always a prefix alongside the existing state word, never a replacement — every
  existing assertion on the literal state word (e.g. `expect(line).toContain("waitingUser")`)
  must keep passing unchanged.
- No config flag and no plain-text/no-icon fallback mode.
- `MonitorRow.state` is a plain `string`, not a closed union (the Rust `RunState` is a newtype
  around `String`, so `ts-rs` emits `string`) — every state lookup needs a fallback branch, it
  cannot be an exhaustive switch.
- Real codepoints only, already verified against pictogrammers.com — do not substitute or
  "simplify" them:
  - Header (bat): `U+F0B5F`
  - `queued`: `U+F0150` · `starting`: `U+F14DF` · `working`: `U+F1461` · `waitingUser`: `U+F0B5A`
  - `waitingPeer`: `U+F000F` · `paused`: `U+F03E6` · `succeeded`: `U+F05E1` · `failed`: `U+F015A`
  - `cancelled`: `U+F073A` · `lost`: `U+F0BA6` · fallback: `U+F0625`
- State-to-color map (all values are real members of `ThemeColor` from
  `@oh-my-pi/pi-coding-agent`): `queued`→`muted`, `starting`→`accent`, `working`→`accent`,
  `waitingUser`→`warning`, `waitingPeer`→`warning`, `paused`→`muted`, `succeeded`→`success`,
  `failed`→`error`, `cancelled`→`dim`, `lost`→`error`, fallback→`text`.
- Full spec: `docs/superpowers/specs/2026-07-31-batman-monitor-widget-visual-formatting-design.md`.

---

## Task 1: State icon and color lookups, threaded into `renderRowLine`

**Files:**
- Modify: `packages/extension/src/monitor/render.ts:1-10` (imports, add lookups above
  `renderWidgetLines`), `render.ts:31-32` (the first line of `renderRowLine`)
- Test: `packages/extension/src/monitor/render.test.ts`

**Interfaces:**
- Produces: `stateIcon(state: string): string`, `stateColor(state: string): ThemeColor` — both
  exported, both used by Task 3's `renderWidgetBox`.

- [ ] **Step 1: Write the failing tests**

Add to `packages/extension/src/monitor/render.test.ts` (near the top, after the existing
imports — add `stateIcon`, `stateColor` to the existing `import { ... } from "./render"` line):

```ts
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bun test packages/extension/src/monitor/render.test.ts`
Expected: FAIL — `stateIcon`/`stateColor` are not exported from `./render`, and the new
`toContain` assertion on the icon prefix fails against the current plain `row.state` output.

- [ ] **Step 3: Implement the lookups and thread the icon into `renderRowLine`**

In `packages/extension/src/monitor/render.ts`, change the top of the file from:

```ts
import type { MonitorRow, MonitorState } from "./model";

/** The widget never renders more than this many rows at once. */
export const MAX_WIDGET_ROWS = 10;
```

to:

```ts
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
```

Then change `renderRowLine`'s first line, from:

```ts
export function renderRowLine(row: MonitorRow): string {
  const parts = [shortId(row.runId), row.state];
```

to:

```ts
export function renderRowLine(row: MonitorRow): string {
  const parts = [shortId(row.runId), `${stateIcon(row.state)} ${row.state}`];
```

Leave the rest of `renderRowLine`, and everything below it in the file, unchanged.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bun test packages/extension/src/monitor/render.test.ts`
Expected: PASS, including the pre-existing tests (`a row line includes state, harness/model,
flags, and pending approvals` still passes — it asserts `toContain("waitingUser")`, and the
state word is still present, just with an icon prefix now).

- [ ] **Step 5: Commit**

```bash
git add packages/extension/src/monitor/render.ts packages/extension/src/monitor/render.test.ts
git commit -m "feat(batman-monitor): add per-state icon and color lookups"
```

---

## Task 2: `renderWidgetHeader()`

**Files:**
- Modify: `packages/extension/src/monitor/render.ts` (add below the `stateColor` block from
  Task 1)
- Test: `packages/extension/src/monitor/render.test.ts`

**Interfaces:**
- Consumes: `BAT_ICON`, `WIDGET_HEADER_TEXT` (module-private constants from Task 1).
- Produces: `renderWidgetHeader(): string`, used by Task 3's `renderWidgetBox`.

- [ ] **Step 1: Write the failing test**

Add to `render.test.ts`:

```ts
test("renderWidgetHeader returns the bat icon and the BATMAN label", () => {
  expect(renderWidgetHeader()).toBe("\u{F0B5F} BATMAN");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test packages/extension/src/monitor/render.test.ts -t "renderWidgetHeader"`
Expected: FAIL — `renderWidgetHeader` is not exported from `./render`.

- [ ] **Step 3: Implement it**

In `render.ts`, directly below the `stateColor` function from Task 1, add:

```ts
/** The widget's brand header: bat icon + "BATMAN", uncolored — the caller
 *  (`renderWidgetBox`) applies theme color, so this stays a plain data
 *  producer with no `Theme` dependency of its own. */
export function renderWidgetHeader(): string {
  return `${BAT_ICON} ${WIDGET_HEADER_TEXT}`;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test packages/extension/src/monitor/render.test.ts -t "renderWidgetHeader"`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add packages/extension/src/monitor/render.ts packages/extension/src/monitor/render.test.ts
git commit -m "feat(batman-monitor): add renderWidgetHeader"
```

---

## Task 3: `renderWidgetBox` — the bordered, color-coded widget

**Files:**
- Modify: `packages/extension/src/monitor/render.ts:17-28` (extract row-selection into a shared
  helper, used by both the existing `renderWidgetLines` and the new `renderWidgetBox`), plus a
  new `assembleBox` + `renderWidgetBox` added after `renderRowLine`
- Test: `packages/extension/src/monitor/render.test.ts`

**Interfaces:**
- Consumes: `stateIcon`, `stateColor` (Task 1), `renderWidgetHeader` (Task 2), `renderRowLine`
  (existing), `MAX_WIDGET_ROWS` (existing).
- Produces: `renderWidgetBox(state: MonitorState, theme: Theme): string[]` — this is what
  Task 4's `controller.ts` change calls.

This task also refactors `renderWidgetLines` to share its row-selection logic with the new
function, via a private `selectRows` helper. This is a pure refactor — `renderWidgetLines`'s
observable behavior (and its existing tests) must not change.

- [ ] **Step 1: Write the failing tests**

Add to `render.test.ts` (add `renderWidgetBox` to the `import { ... } from "./render"` line, and
add this helper near the top, alongside the existing `row`/`stateOf` helpers):

```ts
function fakeTheme(): Theme {
  return {
    boxRound: {
      topLeft: "╭",
      topRight: "╮",
      bottomLeft: "╰",
      bottomRight: "╯",
      horizontal: "─",
      vertical: "│",
      cross: "┼",
      teeDown: "┬",
      teeUp: "┴",
      teeRight: "├",
      teeLeft: "┤",
    },
    fg: (color: ThemeColor, text: string) => `[${color}]${text}[/${color}]`,
  } as unknown as Theme;
}
```

(This mirrors the existing `fixtureExtension` test's pattern of faking only the SDK members
actually used, cast through `as unknown as`, rather than constructing a real `Theme`.)

Then add:

```ts
test("renderWidgetBox embeds the accent-colored header in the top border", () => {
  const lines = renderWidgetBox({ rows: {}, lastSequence: 0n }, fakeTheme());
  expect(lines[0]).toContain("╭─");
  expect(lines[0]).toContain(`[accent]${renderWidgetHeader()}[/accent]`);
});

test("renderWidgetBox wraps the empty-state line in the border, uncolored", () => {
  const lines = renderWidgetBox({ rows: {}, lastSequence: 0n }, fakeTheme());
  expect(lines).toHaveLength(3); // top border, empty-state line, bottom border
  expect(lines[1]).toContain("[text]No BATMAN runs yet.[/text]");
  expect(lines[1].startsWith("[border]│[/border]")).toBe(true);
  expect(lines[1].endsWith("[border]│[/border]")).toBe(true);
});

test("renderWidgetBox colors each row by its state and ends with a plain bottom border", () => {
  const succeededRow = row({ runId: "run-1", state: "succeeded" });
  const lines = renderWidgetBox(stateOf([succeededRow]), fakeTheme());

  expect(lines).toHaveLength(3);
  expect(lines[1]).toContain(`[success]${renderRowLine(succeededRow)}[/success]`);

  const bottom = lines[lines.length - 1];
  expect(bottom.startsWith("[border]╰")).toBe(true);
  expect(bottom.endsWith("╯[/border]")).toBe(true);
});

test("renderWidgetBox appends a muted overflow line beyond MAX_WIDGET_ROWS", () => {
  const rows = Array.from({ length: MAX_WIDGET_ROWS + 2 }, (_, i) =>
    row({ runId: `run-${i}`, lastEventAt: `2026-01-01T00:${String(i).padStart(2, "0")}:00Z` }),
  );
  const lines = renderWidgetBox(stateOf(rows), fakeTheme());

  // top border + MAX_WIDGET_ROWS rows + 1 overflow line + bottom border
  expect(lines).toHaveLength(MAX_WIDGET_ROWS + 3);
  const overflowLine = lines[lines.length - 2];
  expect(overflowLine).toContain("[muted]");
  expect(overflowLine).toContain("more; use /batman status <runId> for full details.");
});

test("renderWidgetLines is unaffected by the renderWidgetBox refactor", () => {
  const lines = renderWidgetLines({ rows: {}, lastSequence: 0n });
  expect(lines).toEqual(["No BATMAN runs yet."]);
});

test("renderWidgetBox produces a top border, every content line, and the bottom border at equal total width", () => {
  // A `fg` that returns text unchanged, unlike `fakeTheme()`'s tagging `fg` — the
  // color-tag wrapper length would otherwise interfere with measuring raw visual
  // width, which is exactly what this test checks.
  const plainTheme = {
    boxRound: fakeTheme().boxRound,
    fg: (_color: ThemeColor, text: string) => text,
  } as unknown as Theme;

  const rows = [
    row({ runId: "run-1", state: "succeeded", lastEventAt: "2026-01-01T00:00:00Z" }),
    row({ runId: "run-2", state: "queued", lastEventAt: "2026-01-01T00:01:00Z" }),
  ];
  const lines = renderWidgetBox(stateOf(rows), plainTheme);

  const widths = new Set(lines.map((line) => line.length));
  expect(widths.size).toBe(1);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bun test packages/extension/src/monitor/render.test.ts`
Expected: FAIL — `renderWidgetBox` is not exported from `./render`, and `Theme`/`ThemeColor`
are not yet imported into the test file.

- [ ] **Step 3: Implement `selectRows`, `assembleBox`, and `renderWidgetBox`**

First, in `render.test.ts`, add `Theme`, `ThemeColor` to the imports:

```ts
import type { Theme, ThemeColor } from "@oh-my-pi/pi-coding-agent";
```

Now in `render.ts`, replace:

```ts
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
```

with:

```ts
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
```

Then, after `renderRowLine` (i.e. directly above `/** Renders the full detail block for
\`/batman status <runId>\`. */`), add:

```ts
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
  const contentWidth = Math.max(...lines.map((line) => line.length)) + 2;
  const width = Math.max(contentWidth, header.length + 4);

  const top =
    theme.fg("border", `${topLeft}${horizontal} `) +
    theme.fg("accent", header) +
    theme.fg("border", ` ${horizontal.repeat(width - header.length - 3)}${topRight}`);

  const body = lines.map((line, index) => {
    const pad = width - line.length - 1;
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `bun test packages/extension/src/monitor/render.test.ts`
Expected: PASS — every test in the file, including the pre-existing ones (the `selectRows`
refactor must not change `renderWidgetLines`'s output).

- [ ] **Step 5: Commit**

```bash
git add packages/extension/src/monitor/render.ts packages/extension/src/monitor/render.test.ts
git commit -m "feat(batman-monitor): add renderWidgetBox with border and per-state color"
```

---

## Task 4: Wire `renderWidgetBox` into the live widget

**Files:**
- Modify: `packages/extension/src/monitor/controller.ts:10` (import),
  `controller.ts:87-90` (remove dead method), `controller.ts:105-108` (`refresh`)

**Interfaces:**
- Consumes: `renderWidgetBox` (Task 3), `ExtensionContext.ui.theme` (`readonly theme: Theme` on
  `ExtensionUIContext`, already part of the installed `@oh-my-pi/pi-coding-agent` SDK — no new
  dependency).

There is no unit test file for `controller.ts` today (its wiring is exercised by the manual
walkthrough in `docs/manual-testing.md` §2, not a Bun test), so this task's verification is the
full test suite plus a live manual check, not a new automated test.

- [ ] **Step 1: Update the import and remove the now-dead `renderLines()` method**

In `packages/extension/src/monitor/controller.ts`, change:

```ts
import { renderRowDetails, renderWidgetLines } from "./render";
```

to:

```ts
import { renderRowDetails, renderWidgetBox } from "./render";
```

Then remove this method from the `MonitorController` class (it has no remaining callers once
Step 2 lands):

```ts
  /** The widget's current concise lines. */
  renderLines(): string[] {
    return renderWidgetLines(this.#state);
  }

```

(Delete the whole block, including its doc comment and the blank line after it. Leave
`getState()` above it and `renderStatus()` below it untouched.)

- [ ] **Step 2: Change `refresh()` to build the bordered widget**

Change:

```ts
  function refresh(extCtx: ExtensionContext): void {
    extCtx.ui.setWidget(WIDGET_KEY, controller.renderLines(), { placement: "aboveEditor" });
    pi.appendEntry(MONITOR_ENTRY_TYPE, { sequence: Number(controller.getState().lastSequence) });
  }
```

to:

```ts
  function refresh(extCtx: ExtensionContext): void {
    extCtx.ui.setWidget(WIDGET_KEY, renderWidgetBox(controller.getState(), extCtx.ui.theme), { placement: "aboveEditor" });
    pi.appendEntry(MONITOR_ENTRY_TYPE, { sequence: Number(controller.getState().lastSequence) });
  }
```

- [ ] **Step 3: Run the full TypeScript test suite**

Run: `bun test packages`
Expected: PASS — no test references `MonitorController.renderLines` (confirmed: it was the only
caller), so removing it breaks nothing.

- [ ] **Step 4: Rebuild the extension**

Run: `bun run build`
Expected: bundles cleanly to `packages/extension/dist/index.js` with no TypeScript errors (in
particular, confirms `ExtensionContext.ui.theme` really is on the installed SDK's public type,
not just something seen in `node_modules` source).

- [ ] **Step 5: Manually verify the live widget in a real session**

This repo already has a running `batcave` daemon with a `queued` run in its journal (from the
existing `docs/manual-testing.md` §3 walkthrough), so this is a fast check, not a fresh setup:

```bash
export OMP_BATMAN_BINARY="$PWD/target/debug/batcave"
EXT="$PWD/packages/extension/dist/index.js"
omp --extension "$EXT"
```

Type `/batman`. Expect a rounded-border box with `🦇 BATMAN` spliced into the top border line,
and the existing `queued` run rendered as a border-wrapped, muted-colored row with the clock
icon next to the word `queued`. Compare against the design doc's mockup:

```
╭─ 🦇 BATMAN ──────────────────────────────────────────────────────────╮
│ 019fa036 · 🕐 queued · run queued                                   │
╰────────────────────────────────────────────────────────────────────╯
```

(Exact icon glyphs render as Nerd Font symbols in a real terminal, not the emoji shown here —
this block is illustrative of the layout, not a literal terminal capture.) If the border,
header-in-border-line, and row color don't match, stop and re-check Task 3's `assembleBox`
against this task's `refresh()` change before proceeding — do not move on with a mismatch
unexplained.

- [ ] **Step 6: Commit**

```bash
git add packages/extension/src/monitor/controller.ts
git commit -m "feat(batman-monitor): render the bordered widget in the live extension"
```
