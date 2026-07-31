# BATMAN monitor widget visual formatting

## Context

The `/batman` embedded monitor widget (`packages/extension/src/monitor/`) currently renders
plain, unstyled text lines above the editor — e.g. `019fa036 · queued · run queued`. It's
functionally correct but visually indistinguishable from any other plain-text notification, and
gives no at-a-glance signal of run state beyond the literal state word.

This spec covers two additive changes, both scoped to the `/batman` widget only (not the
`/batman status <runId>` detail block, and not `/batman-status`):

1. Nerd Font icons — a header icon/label, and a per-row state icon next to the existing state
   word.
2. A bordered box, color-coded rows, matching the app's own visual language (rounded corners,
   theme border color) so the widget reads as a distinct UI element rather than a plain
   notification line.

## Icons

Real Material Design Icon codepoints (Nerd Fonts v3 patches these in 1:1 under the same
codepoints, verified against pictogrammers.com — not guessed PUA values):

| Purpose | Icon name | Codepoint |
|---|---|---|
| Header (brand) | `bat` | `U+F0B5F` |
| `queued` | `clock-outline` | `U+F0150` |
| `starting` | `rocket-launch-outline` | `U+F14DF` |
| `working` | `cog-sync-outline` | `U+F1461` |
| `waitingUser` | `account-question-outline` | `U+F0B5A` |
| `waitingPeer` | `account-multiple-outline` | `U+F000F` |
| `paused` | `pause-circle-outline` | `U+F03E6` |
| `succeeded` | `check-circle-outline` | `U+F05E1` |
| `failed` | `close-circle-outline` | `U+F015A` |
| `cancelled` | `cancel` | `U+F073A` |
| `lost` | `help-rhombus-outline` | `U+F0BA6` |
| *(unrecognized state)* | `help-circle-outline` | `U+F0625` |

`MonitorRow.state` (`packages/extension/src/monitor/model.ts`) is typed as a plain `string` — the
Rust `RunState` is a newtype around `String`, not a closed enum, so `ts-rs` emits `string`, not a
literal union. The fallback icon exists because the lookup cannot be exhaustive at the type level.

The icon is a prefix alongside the existing state word (`󰅐 queued`), never a replacement — state
words stay greppable and the existing `renderRowLine`/`renderWidgetLines` tests that assert on the
literal state word keep passing unchanged.

The header is a single line, shown once above the rows (or above the empty-state line), not
repeated per row: `󰭟 BATMAN`.

## Border and color

`ExtensionContext.ui.theme` (`readonly theme: Theme` on `ExtensionUIContext`) is directly
available wherever `refresh()` already has `extCtx` — no need to route through `setWidget`'s
component-factory branch (`(tui, theme) => Component`) or import `@oh-my-pi/pi-tui`'s `Box`.
`Box`'s border draws all four edges uniformly with no title slot anyway, and what this needs is a
title embedded *in* the top border — the same technique the app's own editor chrome uses
(`getTopBorder()` in `@oh-my-pi/pi-coding-agent/src/tui/status-line.ts` builds its top border line
by hand with segments spliced in, rather than drawing a box and placing a label inside it). So the
widget is built as a plain `string[]` — the same `setWidget` branch already in use today — with
each line hand-assembled from `Theme.boxRound` (`{ topLeft, topRight, bottomLeft, bottomRight,
horizontal, vertical }`) and `theme.fg(...)`, which also means the border automatically respects
the user's configured symbol preset (`nerd`/`unicode`/`ascii`).

The widget becomes:

```
╭─ 󰭟 BATMAN ──────────────────────────────────────────────────────────╮
│ 019fa036 · 󰅐 queued · run queued                                   │
╰────────────────────────────────────────────────────────────────────╯
```

The header sits *in* the top border line itself (`topLeft + horizontal + " " + header + " " +
horizontal-fill + topRight`), not as a separate row inside the box — matching the reference
screenshot where the editor's own status segments (`ornith-large`, the repo path, the git branch)
are spliced into its top border the same way, rather than sitting on their own line above it.

- Border color: `theme.fg("border", ...)`, matching the theme's semantic border color rather than
  a hardcoded ANSI color.
- Header: `theme.fg("accent", header)` — same treatment as the accent-colored titles in
  `renderStatusLine` (`@oh-my-pi/pi-coding-agent/src/tui/status-line.ts`), for consistency with the
  rest of the app's status chrome.
- Each content row is wrapped in `theme.fg("border", vertical)` on both sides and additionally
  color-coded by state, using the theme's semantic colors (not hardcoded ANSI):

| state | color |
|---|---|
| `queued` | `muted` |
| `starting` | `accent` |
| `working` | `accent` |
| `waitingUser` | `warning` |
| `waitingPeer` | `warning` |
| `paused` | `muted` |
| `succeeded` | `success` |
| `failed` | `error` |
| `cancelled` | `dim` |
| `lost` | `error` |
| *(unrecognized state)* | `text` (theme default, unstyled) |

## Architecture

`render.ts` keeps its existing character: pure, framework-independent string functions, already
covered by exact-equality tests. It gains:

- `stateIcon(state: string): string` lookup (the icon table above), threaded into `renderRowLine`
  as a prefix before the existing state word.
- `stateColor(state: string): ThemeColor` lookup (the color table above).
- `renderWidgetBox(state: MonitorState, theme: Theme): string[]` — the new theme-aware entry
  point `controller.ts` calls instead of `renderWidgetLines`. It builds the hand-assembled top
  border (with the accent-colored header spliced in), one border-wrapped and state-colored line
  per row (or the existing empty-state line, border-wrapped but uncolored) via `renderRowLine`,
  and the plain bottom border — all using `theme.boxRound` and `theme.fg`. `renderWidgetLines` and
  `renderRowLine` stay exactly as they are today (still exercised directly by the existing exact-
  equality tests); `renderWidgetBox` is a thin wrapper around them, so `render.ts` does gain a
  `Theme` dependency, but stays a plain string-in-string-out module — no `pi-tui` import, no
  component classes, no side effects.

`controller.ts`'s `refresh()` changes from:

```ts
extCtx.ui.setWidget(WIDGET_KEY, controller.renderLines(), { placement: "aboveEditor" });
```

to:

```ts
extCtx.ui.setWidget(WIDGET_KEY, renderWidgetBox(controller.getState(), extCtx.ui.theme), { placement: "aboveEditor" });
```

This is the only change to `controller.ts`. Everything else (subscribe/replay lifecycle,
`/batman status <runId>`, the `MONITOR_ENTRY_TYPE` persistence) is untouched — this spec is
presentation-only.

## Testing

Extend the existing pure-function tests in `render.test.ts`:

- `stateIcon`/`stateColor` return the expected value for every known state and the fallback for an
  unknown one.
- `renderRowLine` contains the icon prefix (in addition to the existing assertions on state word,
  harness/model, flags, and pending approvals).
- `renderWidgetBox`: construct a minimal fake `Theme` satisfying only the two members actually
  called — the `boxRound` getter and `fg(color, text)` — consistent with this codebase's existing
  convention of fakes at injection seams (see `CONTRIBUTING.md`'s "Non-Negotiable Invariants" and
  the existing `fixtureExtension` test in `render.test.ts`, which already fakes
  `ExtensionContext.ui.setWidget` rather than driving a real terminal). Assert: the first line
  contains the header text between the top-left corner and the border fill; every content line is
  wrapped in the vertical border character on both sides; the last line is the plain bottom
  border; row lines carry the color tag from `stateColor` (the fake `fg` can just tag text with
  its color name, e.g. `` `[${color}]${text}` ``, so assertions can check for the tag rather than
  a real ANSI escape).

## Out of scope

- `/batman status <runId>` (`renderRowDetails`) — unchanged, per the earlier design decision to
  scope this to widget rows only.
- A plain-text/no-icon fallback mode or config flag — not needed; the app's own status bar already
  assumes Nerd Font glyph support in this environment.
