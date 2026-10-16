# Spec: Sysinfo pane — flush top-right controls, a hide-the-selectors button, flush right-hand chart edge

**Date:** 2026-10-16
**Status:** proposed — not implemented
**Files in scope:**
- [`frontend/app/view/sysinfo/sysinfo-bar.tsx`](../../frontend/app/view/sysinfo/sysinfo-bar.tsx) — the control row
- [`frontend/app/view/sysinfo/sysinfo-view.tsx`](../../frontend/app/view/sysinfo/sysinfo-view.tsx) — owns the bar's state, lays out the charts
- [`frontend/app/view/sysinfo/sysinfo-plot.scss`](../../frontend/app/view/sysinfo/sysinfo-plot.scss) — `.sysinfo-bar`, `.sysinfo-bar-actions`
- [`frontend/app/view/sysinfo/sysinfo-plot.tsx`](../../frontend/app/view/sysinfo/sysinfo-plot.tsx) — `TimeChart` padding and x labels
- [`frontend/app/view/sysinfo/sysinfo-chart.ts`](../../frontend/app/view/sysinfo/sysinfo-chart.ts) — `PLOT_RIGHT_PX`
- [`frontend/app/view/sysinfo/sysinfo-overview.scss`](../../frontend/app/view/sysinfo/sysinfo-overview.scss) — the Advanced view's padding
- [`frontend/app/view/sysinfo/sysinfo-heatmap.tsx`](../../frontend/app/view/sysinfo/sysinfo-heatmap.tsx) — uses `PLOT_RIGHT_PX`
- [`frontend/app/view/sysinfo/sysinfo-model.ts`](../../frontend/app/view/sysinfo/sysinfo-model.ts) — pane meta

Three changes to the Sysinfo pane, one PR.

---

## 1. Today

The control row (`SysinfoBar`) is one flex row: the **view** buttons (CPU, Cores, …), the **time window** buttons (1m … 15m), then a right-aligned `.sysinfo-bar-actions` group holding a "Paused" label, **pause** and **table** (the figures as numbers).

- `.sysinfo-bar` has `padding: 2px 3px`, so the pause and table buttons sit 3px in from the right border and 2px down from the top.
- A simple view's charts are `TimeChart ... tight`. Its uPlot `padding` is `[7, null, null, 0]`: `null` on the right means uPlot reserves room for half of the last x label, so the plot ends short of the pane's right edge.
- The Advanced view's charts are not `tight`: `padding` right is `PLOT_RIGHT_PX` (36px), shared with the core heatmap (`sysinfo-heatmap.tsx`) so the two line up. `.sysinfo-overview` also has `padding: 6px`.

## 2. Wanted

### 2.1 Pause and table flush to the top-right corner

The right edge of the rightmost button touches the pane's right border, and the top edge of the buttons touches the pane's top border.

- `.sysinfo-bar`: `padding: 0 0 2px 3px` (top and right 0; left and bottom unchanged).
- The button's drawn box must reach the border, not just its hit area: `IconButton` is square with its own padding, so if its box leaves a gap to the border, remove the gap on the last button's outer side rather than shifting the glyph.
- When the bar wraps onto a second line (narrow pane), the actions group keeps `margin-left: auto` and stays on the first line's right edge; the wrapped controls start the second line at the left.

### 2.2 A third button: show / hide the view selectors

A new `IconButton` in the actions group, a toggle (`pressed` = selectors hidden):

| | |
|---|---|
| Icon | `sliders` (Font Awesome Free solid) |
| Label / tooltip | `Hide view buttons` when shown, `Show view buttons` when hidden |
| Position | leftmost in the group: `[sliders] [pause] [table]`, so pause and table do not move when it is toggled |
| Hides | **both** `SegmentedControl`s: the views (CPU, Cores, …) **and** the time window (1m … 15m) |
| Keeps | the "Paused" label, pause, table and the toggle itself |

- Hidden, the bar is one compact row of three buttons at the top right; the charts (or the table) get the freed height. Each chart already names itself in its title, so the view is still readable without the buttons.
- It works in the simple and the Advanced views, and with the table showing.
- Accessibility: `aria-pressed` comes from `IconButton`'s `pressed`; focus stays on the toggle after the press. Tab order is unchanged for the buttons that remain.
- **Remembered per pane:** saved in the block's meta as `sysinfo:hidecontrols` (boolean; absent = shown), through `ctx.setMeta`, like `graph:windowsecs`. A reload or a layout restore keeps it. `sysinfo-model.ts` gets a `controlsHidden()` accessor and `setControlsHidden()`; `SysinfoBar` takes `controls: boolean` and `onControls`.
  - Add the key to `MetaType` (`frontend/types/srv-types.d.ts` is generated: add it where the Rust meta struct defines `sysinfo:focus`, then regenerate). `layout_file.rs` copies `sysinfo:focus` and `graph:windowsecs` when exporting and importing a layout; copy this key there too so an exported layout keeps it.
- Not a global setting: a user can have one pane tidy and another full.

### 2.3 The chart's right side flush against the pane's right border

The newest sample is drawn at the right-hand edge of the plot, and that edge is the pane's right border.

- **Simple views (`tight`):** uPlot `padding` becomes `[7, 0, null, 0]`. The last x label would then be centred on the edge and cut off, so `values` blanks the label of any tick whose label would cross the plot's right edge (the tick mark stays). The line is 2px wide, so its end is half clipped at the edge; that is accepted, and the same as the left.
- **Advanced views (not `tight`):** `PLOT_RIGHT_PX` goes from `36` to `0` with the same label rule. Because `sysinfo-heatmap.tsx` computes its plot width from the same constant, the core heatmap moves with the charts and stays aligned. `PLOT_LEFT_PX` is unchanged (the y axis).
- `.sysinfo-overview` padding: `6px` → `6px 0 6px 6px` so its charts reach the right border too. Tiles are boxes with a border, not charts: they keep an inset (see Open question 1).
- The hover tooltip (`.sysinfo-chart-tip`) is centred on the cursor, so at the right edge it would be half off-pane. Clamp its `left` so it stays inside the chart's width.

---

## 3. Tests

- `sysinfo-bar.render.test.tsx`: the toggle renders; pressing it calls `onControls`; with `controls` false both segmented controls are absent and pause, table and the toggle remain; the Paused label still shows.
- `sysinfo-view.render.test.tsx`: the hidden state comes from meta, and toggling writes `sysinfo:hidecontrols`.
- `sysinfo-plot.render.test.tsx` / `sysinfo-chart.test.ts`: the x label rule blanks a label that would cross the right edge and keeps the rest; the uPlot padding right is 0.
- `sysinfo-heatmap`: plot width uses the new constant (no test should hard-code 36).
- Manual, in `task dev`: screenshot a narrow and a wide pane, simple and Advanced, selectors shown and hidden, table on; check the pause button touches the right and top borders and the newest point touches the right border.

## 4. Out of scope

- A hover-reveal bar that hides itself (this is an explicit toggle).
- Moving the window buttons to the right group.
- Chart colours, ranges, sample rate.

## 5. Open questions

1. **Advanced-view tiles:** keep their inset from the right border (my default, they are bordered boxes), or run them flush too?
2. **Default:** selectors shown for a pane that has no saved choice (my default).
3. **Icon:** `sliders`, or `eye` / `eye-slash`? `sliders` does not change shape between states; `eye-slash` says "hidden" more literally.
