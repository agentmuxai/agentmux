# Spec: Sysinfo pane — flush top-right controls, a hide-the-selectors button, flush right-hand chart edge

**Date:** 2026-10-16
**Status:** implemented — in #5022, with the owner's decisions of 2026-10-16 (§5). Only the layout export/import of the new setting is left.
**Files in scope:**
- [`frontend/app/view/sysinfo/sysinfo-bar.tsx`](../../frontend/app/view/sysinfo/sysinfo-bar.tsx) — the control row
- [`frontend/app/view/sysinfo/sysinfo-view.tsx`](../../frontend/app/view/sysinfo/sysinfo-view.tsx) — owns the bar's state, lays out the charts
- [`frontend/app/view/sysinfo/sysinfo-plot.scss`](../../frontend/app/view/sysinfo/sysinfo-plot.scss) — `.sysinfo-bar`, `.sysinfo-bar-actions`
- [`frontend/app/view/sysinfo/sysinfo-plot.tsx`](../../frontend/app/view/sysinfo/sysinfo-plot.tsx) — `TimeChart` padding and x labels
- [`frontend/app/view/sysinfo/sysinfo-chart.ts`](../../frontend/app/view/sysinfo/sysinfo-chart.ts) — `PLOT_RIGHT_PX`
- [`frontend/app/view/sysinfo/sysinfo-overview.scss`](../../frontend/app/view/sysinfo/sysinfo-overview.scss) — the Advanced view's padding
- [`frontend/app/view/sysinfo/sysinfo-heatmap.tsx`](../../frontend/app/view/sysinfo/sysinfo-heatmap.tsx) — uses `PLOT_RIGHT_PX`
- [`frontend/app/view/sysinfo/sysinfo-model.ts`](../../frontend/app/view/sysinfo/sysinfo-model.ts) — pane meta

Three changes to the Sysinfo pane, one PR. The owner's decisions: the simple and the Advanced views get the same treatment through shared code; the view and window buttons are **hidden by default**; **text sits 1px from the pane's borders and graphics 0px**; the buttons come from the line-style component set (`element/ui`).

---

## 1. Today

The control row (`SysinfoBar`) is one flex row: the **view** buttons (CPU, Cores, …), the **time window** buttons (1m … 15m), then a right-aligned `.sysinfo-bar-actions` group holding a "Paused" label, **pause** and **table** (the figures as numbers).

- `.sysinfo-bar` has `padding: 2px 3px`, so the pause and table buttons sit 3px in from the right border and 2px down from the top.
- A simple view's charts are `TimeChart ... tight`. Its uPlot `padding` is `[7, null, null, 0]`: `null` on the right means uPlot reserves room for half of the last x label, so the plot ends short of the pane's right edge.
- The Advanced view's charts are not `tight`: `padding` right is `PLOT_RIGHT_PX` (36px), shared with the core heatmap (`sysinfo-heatmap.tsx`) so the two line up. `.sysinfo-overview` also has `padding: 6px`.

## 2. Wanted

### 2.1 Pause and table flush to the top-right corner

The right edge of the rightmost button touches the pane's right border, and the top edge of the buttons touches the pane's top border.

- `.sysinfo-bar`: `padding: 0 0 0 1px` (nothing on the top or right; the 1px text inset on the left).
- The button's drawn box must reach the border, not just its hit area: `IconButton` is square with its own padding, so if its box leaves a gap to the border, remove the gap on the last button's outer side rather than shifting the glyph.
- The bar is a two-column grid: the view and window buttons in column 1 (`.sysinfo-bar-selectors`, which wraps onto further lines as the pane narrows) and the actions in column 2 (`.sysinfo-bar-actions`), so the actions stay on the first line, in the corner, at any width.
- The bar floats over the pane (`position: absolute` at the top, `z-index` above the charts, `pointer-events: none` except on its buttons and the selector panel). It takes no height in the layout, so the charts and their figures (the CPU numbers) are in the same place whether the view buttons are shown or hidden. In the table view the table starts below the buttons (`padding-top` on `.sysinfo-table-scroll`).

### 2.2 A third button: show / hide the view selectors

A new `IconButton` in the actions group, a toggle (`pressed` = the view buttons are shown):

| | |
|---|---|
| Control | just an eye: a compact `IconButton` from the component set, icon `eye` while the view buttons are shown and `eye-slash` while they are hidden, `pressed` while shown. Pause and table are compact `IconButton`s too |
| Accessible name | `Hide view buttons` when shown, `Show view buttons` when hidden |
| Position | leftmost in the group: `[eye] [pause] [table]`, so pause and table do not move when it is toggled |
| Hides | **both** `SegmentedControl`s: the views (CPU, Cores, …) **and** the time window (1m … 15m) |
| Keeps | the "Paused" label, pause, table and the toggle itself |

- Hidden, the bar is one compact row of three buttons at the top right. Shown, the selectors sit on a panel (the main background at 90% opacity) over the top-left of the charts, covering their titles while open; the layout underneath does not move. Each chart names itself in its title, so the view is still readable without the buttons.
- It works in the simple and the Advanced views, and with the table showing.
- Accessibility: `aria-pressed` comes from `IconButton`'s `pressed`; focus stays on the toggle after the press. Tab order is unchanged for the buttons that remain.
- **Hidden until chosen, remembered per pane:** saved in the block's meta as `sysinfo:showcontrols` (boolean; absent = hidden), through `ctx.setMeta`, like `graph:windowsecs`. A reload keeps it. `sysinfo-model.ts` has `controlsShown()` and `setControlsShown()`; `SysinfoBar` takes `controls: boolean` and `onControls`. The key is added to `MetaType` in `frontend/types/srv-types.d.ts` (hand-maintained).
  - Follow-up, not done: `layout_file.rs` copies `sysinfo:focus` and `graph:windowsecs` when a layout is exported or imported; it does not yet copy `sysinfo:showcontrols`.
- Not a global setting: a user can have one pane tidy and another full.

### 2.3 The chart's right side flush against the pane's right border

The newest sample is drawn at the right-hand edge of the plot, and that edge is the pane's right border.

- **One layout for every chart (DRY):** the `tight` option of `TimeChart` is gone; every chart, in the simple and the Advanced views, is built flush. uPlot `padding` is `[7, 0, null, 1]`: nothing on the right, 1px of text inset on the left, room for the top y label. The last x label would be centred on the edge and cut off, so `values` blanks the label of any tick whose label (plus the 1px inset) would cross the plot's right edge (the tick mark stays). The line is 2px wide, so its end is half clipped at the edge, the same as on the left.
- `PLOT_LEFT_PX` and `PLOT_RIGHT_PX` are removed. The core heatmap runs the full width, no gutters. Consequence: the charts' y axes are only as wide as their labels, so in Everything the plot areas of the charts and the heatmap no longer line up to the pixel (the simple views already worked this way).
- `.sysinfo-overview` padding is `0`: the tiles (bordered boxes) and the charts touch the pane's borders. Text inside a tile keeps the tile's own padding. Chart titles sit `--sysinfo-text-inset` (1px) from the top and left.
- The hover tooltip (`.sysinfo-chart-tip`) is centred on the cursor, so at the right edge it would be half off-pane. Its `left` is clamped to the chart's width.

---

## 3. Tests (written)

- `sysinfo-bar.render.test.tsx`: the toggle renders and calls `onControls`; with `controls` false both segmented controls are absent and pause, table and the toggle remain; the Paused label still shows.
- `sysinfo-model.window.test.ts`: hidden with nothing in meta, shown when `sysinfo:showcontrols` is true; a choice applies at once and is saved.
- `sysinfo-plot.render.test.tsx`: the x label rule blanks a label that would cross the right edge and keeps the rest; the uPlot padding is `[7, 0, null, 1]`.
- Manual, in `task dev`: screenshot a narrow and a wide pane, simple and Advanced, selectors shown and hidden, table on; check the pause button touches the right and top borders and the newest point touches the right border.

## 4. Out of scope

- A hover-reveal bar that hides itself (this is an explicit toggle).
- Moving the window buttons to the right group.
- Chart colours, ranges, sample rate.

## 5. Decisions (were open questions)

1. Advanced-view tiles run flush too.
2. The view and window buttons are hidden for a pane with no saved choice.
3. The toggle is just an eye (`eye` / `eye-slash`), an `IconButton` from the component set. (It was first a labelled "Views" button.)
4. The control row floats over the pane instead of taking its own row, and the selector panel is 90% opaque.
