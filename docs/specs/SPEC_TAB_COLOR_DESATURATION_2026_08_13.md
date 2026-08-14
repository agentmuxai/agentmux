# Spec: Desaturate tab colors, keep agent pane border colors as-is

**Date:** 2026-08-13
**Status:** Implemented (see revision note below)
**Scope:** `frontend/app/tab/tab.tsx`, `frontend/app/tab/tabbar.tsx`, `frontend/app/view/agent/agent-color.ts`, `agentmux-srv/src/backend/agent_color.rs` (doc comment only)

**Revision (same day):** the first implementation used a fully muted
`S=45%/L=32%` palette (§3 table below). User feedback: "a little too
desaturated" — go halfway between the original vivid hues and that muted
set. The shipped values are the per-hue average of the original and muted
HSL (S and L each averaged independently, hue unchanged), with lightness
nudged down slightly on 7 of the 14 colors (Orange/Amber/Yellow/Lime/
Green/Teal/Cyan) where the raw average dropped below WCAG AA. See the
"Shipped (halfway)" column in §3.

## Problem

The tab color picker (right-click a tab → color) assigns colors from the
`TAB_COLORS` array in `frontend/app/tab/tab.tsx:20-35` — 14 stock
Tailwind-500 hues, ~26° apart on the hue wheel, applied as a solid
`background: var(--tab-color)` fill on the tab (`tab.scss:141-159`). At
full Tailwind-500 saturation (81-95%) these read as neon/fluorescent,
which looks out of place in a tab strip meant to sit quietly in the
periphery.

The border colors around agent panes are **not** in scope — they should
stay exactly as they are.

## Why this isn't a one-line palette edit

`TAB_COLORS` is not tab-exclusive. `frontend/app/view/agent/agent-color.ts:18-20`
reuses the same array as `AGENT_COLOR_PALETTE`, which auto-assigns agent
pane **border** colors (`frame:bordercolor` / `frame:activebordercolor` in
`frontend/app/block/blockframe.tsx:704-750`, dimmed via `dimAgentColor` for
unfocused panes). A separate 12-hue vivid palette
(`hueToActiveBorder(hue) = hsl(hue, 65%, 52%)` in
`frontend/app/block/pane-color-menu.ts`) also drives pane border color when
a user picks a custom hue.

Editing `TAB_COLORS` in place would silently desaturate agent pane borders
too — the exact colors the user wants to keep vivid. So this spec **forks**
the palette: a new muted array feeds the tab color picker; `agent-color.ts`
and `pane-color-menu.ts` keep sourcing from the original vivid values,
unchanged.

## Proposed change

1. Add a new `TAB_COLORS` replacement (muted) in `tab.tsx`, keeping the same
   14 names/order so existing per-tab `--tab-color` values (stored as hex
   strings, e.g. in saved layouts) still resolve to a color — old saved tab
   colors will just render muted after this change, no migration needed
   since the value itself is a plain hex string, not an index.

2. Rename the current vivid array (don't delete it) to
   `AGENT_BORDER_COLORS` and move it to `frontend/app/view/agent/agent-color.ts`
   (or keep it in `tab.tsx` under a new name and re-export) so
   `AGENT_COLOR_PALETTE` keeps pointing at the original vivid hexes with no
   behavior change. `pane-color-menu.ts`'s `hueToActiveBorder` formula is
   untouched — it doesn't reference `TAB_COLORS`.

3. Muted palette values — same hue per color (so "Blue" still reads as
   blue), normalized to HSL `S=45%, L=32%`. This keeps ≥4.5:1 contrast
   against the existing white tab-label text (`rgba(255,255,255,0.95)` in
   `tab.scss`) for every color — verified below — while reading as a calm,
   professional "jewel tone" set instead of neon (this is the same
   desaturate-and-darken approach used for tab/label chips in tools like
   Linear and GitHub issue labels).

   | Name    | Current (vivid) | Muted (first pass, not shipped) | Shipped (halfway) | Contrast vs white text |
   |---------|------------------|----------------------------------|---------------------|------------------------|
   | Red     | `#ef4444`        | `#762d2d`                        | `#c22a2a`           | 5.77:1 |
   | Orange  | `#f97316`        | `#764b2d`                        | `#b75e20`           | 4.51:1 |
   | Amber   | `#f59e0b`        | `#765b2d`                        | `#9d6d1d`           | 4.51:1 |
   | Yellow  | `#eab308`        | `#76642d`                        | `#90731a`           | 4.52:1 |
   | Lime    | `#84cc16`        | `#59762d`                        | `#5a821e`           | 4.52:1 |
   | Green   | `#22c55e`        | `#2d7648`                        | `#248749`           | 4.52:1 |
   | Teal    | `#14b8a6`        | `#2d766e`                        | `#1e8479`           | 4.51:1 |
   | Cyan    | `#06b6d4`        | `#2d6c76`                        | `#1a8294`           | 4.51:1 |
   | Blue    | `#3b82f6`        | `#2d4976`                        | `#2562c5`           | 5.79:1 |
   | Indigo  | `#6366f1`        | `#2d2e76`                        | `#2d30cf`           | 8.61:1 |
   | Violet  | `#8b5cf6`        | `#432d76`                        | `#5c29d2`           | 7.77:1 |
   | Fuchsia | `#d946ef`        | `#6d2d76`                        | `#ae2ac2`           | 5.36:1 |
   | Pink    | `#ec4899`        | `#762d51`                        | `#c02b75`           | 5.45:1 |
   | Rose    | `#f43f5e`        | `#762d39`                        | `#c42742`           | 5.64:1 |

   All 14 exceed WCAG AA (4.5:1) for normal text, so no change needed to
   the tab-label text color or the `!important` white override at
   `tab.scss:145-150`.

4. Hover/active brightening in `tab.scss:219-241` (`filter:
   brightness(1.1-1.18)`) applies unchanged — verify visually that the
   brightened state doesn't overshoot back toward neon; if it does, reduce
   the hover brightness multiplier for `.tab-colored` specifically rather
   than touching the base palette further.

## Out of scope (explicitly unchanged)

- `--accent-color` and all theme tokens in `theme.scss` / `themes/*.scss`
  (active-tab top stripe, focus rings, buttons, etc.)
- `--border-color` and the translucent per-theme hairline borders (tab
  separators, modal borders, pane-tab-strip borders)
- `AGENT_COLOR_PALETTE` (agent pane border auto-assignment) and
  `hueToActiveBorder`/`hueToBorder` (custom pane border hue picker) —
  these stay on the original vivid values

## Open questions

- Should the muted palette have separate light-theme values? `TAB_COLORS`
  today is not theme-aware (same hex regardless of `data-theme`/polarity).
  The proposed `L=32%` values are tuned for white label text and read fine
  on both dark and light tab-strip backgrounds since they're solid fills,
  not translucent — but this should get a quick visual check in the 4
  light themes (`light`, `solarized-light`, `gruvbox-light`,
  `catppuccin-latte`) before shipping.
- The 12-hue custom picker in `pane-color-menu.ts` is out of scope here
  since it drives pane borders, not tabs — confirm no UI lets a user apply
  that same 12-hue vivid picker to a tab background elsewhere in the app.
