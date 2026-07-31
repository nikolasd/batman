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

`ExtensionContext.ui.setWidget` accepts either a plain `string[]` or a component factory
`(tui: TUI, theme: Theme) => Component` (`@oh-my-pi/pi-coding-agent`'s
`ExtensionUiComponentFactory`). `@oh-my-pi/pi-tui` exports a real `Box` component that takes a
`border: { chars, color }` — this is the same primitive the app's own bordered chrome (loaders,
dialogs) is built from, and `Theme.boxRound` (`{ topLeft, topRight, bottomLeft, bottomRight,
horizontal, vertical }`) gives the exact rounded-corner glyphs already used elsewhere in the app,
respecting the user's configured symbol preset (`nerd`/`unicode`/`ascii`) automatically.

The widget becomes:

```
╭──────────────────────────────────────╮
│ 󰭟 BATMAN                              │
│ 019fa036 · 󰅐 queued · run queued      │
╰──────────────────────────────────────╯
```

- Border color: `theme.fg("border", ...)`, matching the theme's semantic border color rather than
  a hardcoded ANSI color.
- Header: `theme.fg("accent", header)` — same treatment as the accent-colored titles in
  `renderStatusLine` (`@oh-my-pi/pi-coding-agent/src/tui/status-line.ts`), for consistency with the
  rest of the app's status chrome.
- Each row is additionally color-coded by state, using the theme's semantic colors (not hardcoded
  ANSI):

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

- `renderWidgetHeader(): string` — returns the bat icon + `"BATMAN"`, uncolored (color is applied
  by the caller, not baked into the pure string, so the function stays a plain data producer).
- A `stateIcon(state: string): string` lookup (the table above), threaded into `renderRowLine` as
  a prefix before the existing state word.
- A `stateColor(state: string): ThemeColor` lookup (the color table above), exported for the
  component layer to consume — `render.ts` does not import `Theme` or apply color itself, it only
  maps state → the *name* of a semantic color, keeping the module theme-agnostic and its existing
  tests untouched.

A new file, `packages/extension/src/monitor/widget-component.ts`, owns the theme-dependent
presentation:

- `buildWidgetComponent(state: MonitorState): ExtensionUiComponentFactory` — returns a function
  that, given `(tui, theme)`, constructs a `Box` (rounded border, `theme.fg("border", ...)`
  colorizer), adds a `Text` child for the accent-colored header, then a `Text` child per widget
  line (from `renderWidgetLines`), each colored via `theme.fg(stateColor(row.state), line)` for
  rows, or left uncolored for the empty-state line.

`controller.ts`'s `refresh()` changes from:

```ts
extCtx.ui.setWidget(WIDGET_KEY, controller.renderLines(), { placement: "aboveEditor" });
```

to:

```ts
extCtx.ui.setWidget(WIDGET_KEY, buildWidgetComponent(controller.getState()), { placement: "aboveEditor" });
```

This is the only change to `controller.ts`. Everything else (subscribe/replay lifecycle,
`/batman status <runId>`, the `MONITOR_ENTRY_TYPE` persistence) is untouched — this spec is
presentation-only.

## Testing

- `render.ts`: extend the existing pure-function tests in `render.test.ts` — `stateIcon`/
  `stateColor` return the expected value for every known state and the fallback for an unknown
  one; `renderRowLine` contains the icon prefix; `renderWidgetHeader` returns the expected string.
  No changes needed to the existing empty-state/overflow tests (they call `renderWidgetLines`
  directly, which is unchanged).
- `widget-component.ts`: a fake `Theme` object satisfying only the methods actually called
  (`fg`, `boxRound` getter) — consistent with this codebase's existing convention of fakes at
  injection seams (see `CONTRIBUTING.md`'s "Non-Negotiable Invariants" and the existing
  `fixtureExtension` test in `render.test.ts`, which already fakes `ExtensionContext.ui.setWidget`
  rather than driving a real terminal). Assert the returned component is a `Box` with the expected
  number of `Text` children and that `theme.fg`/`boxRound` were consulted — not a full terminal
  render, which is out of scope for a unit test.

## Out of scope

- `/batman status <runId>` (`renderRowDetails`) — unchanged, per the earlier design decision to
  scope this to widget rows only.
- A plain-text/no-icon fallback mode or config flag — not needed; the app's own status bar already
  assumes Nerd Font glyph support in this environment.
