# Spec: Sysinfo text zoom, denser charts, and one zoom path for every pane

**Date:** 2026-10-18
**Status:** implemented — Part A (one zoom path) in #5128; Parts B and C (Sysinfo text zoom, density and text placement) in #5133. §3 A.1 and A.6 and §5 C.2 record what changed from the plan while building.
**Requested by:** the owner: "lets add zoom for the text of sysinfo. the charts would stay the same, but the text would change size on the same zoom system as the other panes. Also, look for any opportunities to tighten padding between panes, especially on the ones with many charts. so intelligent placement of text too"; then "lets also take this opportunity to DRY the zoom system. unless there is a good reason not to, it should be unified under a single path".
**Author:** Manoz@Area54.
**Builds on:** `SPEC_SYSINFO_CONTROLS_AND_EDGES_2026_10_16.md` (#5022: flush edges, floating control row), `SPEC_FILES_ZOOM_2026_10_08.md` (§5 lists the zoom duplication this spec removes), `SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md`, `zoom-architecture.md`, `per-pane-zoom-hover.md`.

**Files in scope:**
- Zoom core: `frontend/app/store/zoom.ts`, `frontend/app/store/zoom-factor.ts`, `frontend/app/block/pane-tab-registry.ts`, `frontend/app/block/pane-tab-host.tsx`, `frontend/app/block/block.tsx`, `frontend/app/app.tsx`
- Panes losing local zoom code: `view/term/term.tsx`, `view/editor/editor-view.tsx`, `view/agent/components/AgentShellSubblock.tsx`, `view/section-pane/section-pane.tsx`, `view/warden/warden-view.tsx`, `view/swarm/swarm-view.tsx`, `view/helpview/helpview.tsx`, `view/files/files-view.tsx`
- Sysinfo: `view/sysinfo/sysinfo.tsx`, `sysinfo-view.tsx`, `sysinfo-plot.tsx`, `sysinfo-heatmap.tsx`, `sysinfo-overview.tsx`, `sysinfo-focus.tsx`, `sysinfo-plot.scss`, `sysinfo-overview.scss`

---

## 1. Goal

1. **Sysinfo zooms with the shared pane zoom** (Ctrl/Cmd `+` `-` `0`, Ctrl+scroll, Ctrl+Shift+scroll, "Reset zoom on all panes") and contracts naturally (owner: "sysinfo rows should zoom with text, we want a naturally contracting experience"): the text, the room reserved for it, the control row and the chart rows' minimum heights all follow the zoom, so zooming out packs more charts into the pane and zooming in gives each more room. The drawing itself stays at 1:1: plot line widths, fills, heatmap cells and row hairlines, sparklines.
2. **Denser Sysinfo layouts**, most of all where there are many charts (All CPU, CPU + Mem + Net, Everything, the Advanced focus views): less space between charts and none spent on text a neighbouring chart already shows.
3. **Text placed by rule, not by reserved bands**: titles over the plot instead of above it, one time axis per column of stacked charts, one y axis per row of charts that share a range, and titles that keep clear of the floating buttons.
4. **One zoom path.** Every pane that zooms stores, steps, receives and exposes its level the same way. Panes differ only in the one place they must (how the level is applied), and they declare that rather than code it.

## 2. Zoom today (measured on main at `e79b7dc17`)

What is already shared, and works:

| Piece | Where |
|---|---|
| The level: `term:zoom` in block meta, 0.5–2.0, `null` at 1.0; `readZoom(meta)`, `clampZoom` | `store/zoom-factor.ts` |
| Steps (keys 0.1, wheel 0.05, skipping steps that don't change the rendered font size), the on-screen indicator, `zoomBlockIn/Out/Reset`, the all-panes batch | `store/zoom.ts` |
| Ctrl/Cmd `+` `-` `0` → `view:zoom:*` to the focused pane | `keybindings/defaults.ts`, `store/keymodel.ts` |
| Ctrl+scroll over a pane (`AppZoomHandler`), Ctrl+Shift+scroll over any pane (`AppAllPanesZoomHandler`), both window-level | `app.tsx` |
| Who takes part: the `paneZoom: { baseFontSize }` capability | `block/pane-tab-registry.ts` |

What is still duplicated (the Files pane spec's §5 list, re-checked today):

- **Six local Ctrl+scroll handlers**, each a copy of the same capture-phase listener (`STEP = 0.1`, clamp 0.5–2.0, `null` at 1.0, `stopPropagation`): `term.tsx:251`, `editor-view.tsx:224`, `AgentShellSubblock.tsx:442`, `section-pane.tsx:99`, `warden-view.tsx:49`, `swarm-view.tsx:84`. Because they stop propagation, the global handler never sees the wheel there, so these panes step by 0.1 with **no indicator**, and every other pane steps by 0.05 **with** one. Same gesture, two behaviours.
- **Swarm repeats the global keys** (`+` `-` `0`) in a local `keydown` handler.
- **Six copies of the write idiom** `{"term:zoom": next === 1.0 ? null : next}`.
- **Help has its own zoom**: key `help:zoom`, its own range check, keys, wheel and indicator call (`helpview.tsx:39-77`). It is left out of the all-panes batch and "Reset zoom on all panes".
- **Each CSS-zoom pane applies the level itself** with the same line, `style={{ zoom: readZoom(meta) }}` or a model memo of it: Files, Swarm, Warden, the section panes (Connectors and the others), Memory.

## 3. Part A: one zoom path (recommended: yes, with one deliberate exception)

**My view:** unify everything except *how the level is applied*. Storage, stepping, the indicator, the gestures and the reading of the level should have exactly one implementation. Applying the level legitimately differs between panes, for reasons that won't go away:

- **Font size, not CSS zoom**, for the terminal (xterm measures its character cells; CSS `zoom` on it breaks the cell grid and the selection hit-testing) and the editor (CodeMirror measures lines and the caret the same way).
- **CSS zoom** for ordinary DOM panes (Files, Swarm, Warden, Memory, the section panes, Help): everything scales together.
- **Selective** for panes that must keep part of their content at 1:1: the agent pane (its own `--agent-pane-zoom` on chosen layers) and, after Part B, Sysinfo (text only, charts fixed).

So the single path covers the level, its storage, its steps and every gesture, and stops where a pane applies the level. **As built:**

### A.1 Where each pane applies the level stays in the pane

Planned here first: a declared `apply: "css" | "font" | "custom"` mode, with the host applying `zoom` for the `"css"` panes. Dropped while building, because the element that zooms is not the same in each pane, and moving it would change behaviour:

- Warden and the section panes zoom an inner element, so their container queries keep measuring the outer one.
- Help zooms its tips but not the filter bar pinned above them.
- Files and Swarm zoom their root; the editor zooms `.editor-view` and also divides pointer deltas by the level (its splitters).
- `.block-content` itself can't take `zoom`: it has its own pixel width and height, which `zoom` would scale too.

So each pane keeps one line that applies `readZoom(meta)` on its own element; there is no apply mode in the capability.

### A.2 The host exposes the level once

`block.tsx`'s `.block-content` (the wrapper every pane's content sits in) sets `--pane-zoom: <level>` for any pane with the `paneZoom` capability, so a pane can scale only part of itself in CSS, e.g. `calc(var(--text-xs) * var(--pane-zoom))` (Sysinfo, Part B). The pane header and tab strip are outside `.block-content`, so they still zoom only with chrome zoom.

The agent pane keeps its own `--agent-pane-zoom`: its peek overlay reads it from the row it anchors to, and its history view sets it outside `.block-content`. Renaming it buys nothing.

### A.3 One way to read and write it

In `store/zoom-factor.ts` (a leaf module, so views can import it): `readZoom(meta)` (unchanged) and `writePaneZoom(setMeta, level)`, which clamps, rounds to 0.01, writes `null` at 100% and returns the level. `zoom.ts`'s `setBlockZoom` writes through it; the six local copies of the write are gone with their handlers (A.4).

### A.4 One gesture path

- **Keys:** only the global `view:zoom:*` commands. Swarm's local `keydown` and Help's are gone.
- **Wheel:** the window's `AppZoomHandler` for every pane. Panes whose content handles the wheel itself and may stop it before the window (the terminal's xterm, the editor's CodeMirror, and the agent pane's shell, a headless sub-block with no `data-blockid` of its own) use one shared hook in `zoom.ts`, `usePaneZoomWheel(el, blockId)`: capture phase, Ctrl+Shift left to the all-panes handler, and `zoomBlockByWheel(blockId, deltaY)`, which steps the block's own `term:zoom` by the shared wheel step with the font-size skipping and the indicator.
- **Deleted with no replacement** (the window handler covers them): the section panes, Warden, Swarm.
- Result: Ctrl+scroll steps by 0.05 and shows the indicator **in every pane** (it was 0.1 with no indicator in those six).

### A.5 Help joins

Help declares `paneZoom: {}`, applies `term:zoom` to its tips (the filter bar stays at 100%), and its local keys, wheel and indicator call are gone. On mount, a `help:zoom` left from before moves across once: written to `term:zoom` unless the pane already has one, then `help:zoom: null`. `help:zoom` stays in `MetaType` for that read.

### A.6 Zoom levels are retained

Requirement (owner, 2026-10-18): "we want to retain the zoom levels of the panes".

- **Kept, unchanged:** the level stays in the pane's `term:zoom` block meta, written through `SetMeta` and saved server-side, so it survives a restart, a reload, and moving the pane to another tab or window (`zoom-architecture.md`: "Zoom is metadata, not local state"). An agent's level is also saved with the agent (`ui:zoom`) and seeded into a new pane for that agent (`command-registry.ts`, `agent-model.ts`). Part A changes none of this: every path still reads and writes `term:zoom`.
- **Kept across the Help move:** A.5's one-time `help:zoom` → `term:zoom` move.
- **Layout files: zoom stays out, by an earlier decision.** Proposed here first: write `term:zoom` into layout files (`crates/srv/src/backend/layout_file.rs`). Dropped while building, because `SPEC_LAYOUT_FILES_2026_09_25.md` §3.3 deliberately never writes `term:*` runtime state (a layout opens the same on any monitor or machine), and a test guards it. Zoom is retained across restarts by the block meta above, which is what was asked. If layouts should carry zoom too, that is a change to §3.3 first. What *is* built in #5128: Sysinfo's `sysinfo:showcontrols` is written and read as `config.show_controls` (a view's own selector, which §3.3 allows), the follow-up `SPEC_SYSINFO_CONTROLS_AND_EDGES_2026_10_16.md` left open; tests in `layout_file/tests.rs`.
- **To verify in the PR:** whether a split carries the zoom. A default split copies the source's meta, so it should; the agent split (a fresh picker) and the editor split (an empty editor "with the source's settings") need checking, and the editor's should keep the zoom.

### A.7 Not unified, on purpose

- **Browser panes:** their zoom is Chromium's page zoom of the embedded browser (the page's own zoom, as in Chrome), not a pane-content zoom. Untouched.
- **Chrome zoom** (title bar, status bar, pane headers, `--zoomfactor`): a separate scope by design (`zoom.ts` header). Untouched.
- **The image viewer's** wheel zoom (`image-viewer.tsx`) zooms the image, not the pane. Untouched.

## 4. Part B: Sysinfo text zoom

Sysinfo becomes a `paneZoom` holder with `baseFontSize: 11`, applying the level itself from `--pane-zoom` (A.2), (its titles are `--text-xs`, 11 px), so the shared keys, wheel, all-panes batch and reset reach it with no Sysinfo-specific handlers.

### B.1 What scales

| Text | Today | Zoomed |
|---|---|---|
| Chart titles (name + now / low / high), heatmap title | `--text-xs`, line-height 12px | `calc(var(--text-xs) * var(--pane-zoom))`; line-height and the space reserved for it (C.2) scale with it |
| Axis labels (uPlot canvas) | uPlot's default 12px `system-ui` | `TimeChart` builds `font: "<round(12 × zoom)>px <the pane's font family>"` (read once from the container's computed style, so the axes also match the app's font, which they don't today) |
| Axis room | y: measured from the widest label already; x: fixed `X_AXIS_PX = 22` | y: unchanged (it measures). x: `ceil(fontPx × 1.25) + tick + gap` instead of the constant |
| Hover tooltip (charts and heatmap) | `--text-xs` | `calc(... * var(--pane-zoom))` |
| Tiles (Everything): name, value, range | xs, 15px, xs | scaled the same way; the sparkline keeps 28px |
| Table view, the Disk volume list, "Paused" | sm / xs | scaled |
| The floating control row (view, window, eye, pause, table) | compact controls | `zoom: var(--pane-zoom)` on `.sysinfo-bar` (decided: it zooms with the text) |
| Chart rows: minimum heights (C.3) | fixed px | `calc(<min> * var(--pane-zoom))`: rows contract when zoomed out, so more fit; grow when zoomed in |

### B.2 What does not scale

Plot line widths, fills, the heatmap's cells and row hairlines, sparkline heights, the 2px gap between charts. A chart at 150% has bigger labels, more room around them and a taller minimum row; its strokes and fills are drawn exactly as at 100%. At 50% a row's minimum is half as tall, so twice as many cores fit before the pane scrolls.

### B.3 How

- `--pane-zoom` comes from the host (A.2). `.sysinfo-pane` defines `--sysinfo-zoom: var(--pane-zoom, 1)` once, and every text size, line-height and zoomed minimum in Sysinfo's SCSS is `calc(<value> * var(--sysinfo-zoom))`. The floating control row takes `zoom: var(--sysinfo-zoom)` (its own text is then 100% inside it, so it isn't scaled twice).
- `TimeChart` reads the zoom from `SysinfoZoomContext` (provided by `sysinfo-view.tsx` from `readZoom(model.meta())`; 1 outside it), so no caller threads it through. The axis font is `<round(12 × zoom)>px` in the pane's own font family; the time axis is `ceil(font × 1.25) + tick + gap` high, the top padding half a label plus 1px. The zoom is one of the inputs that rebuilds the chart (with `series`, the time axis, the y labels, the bare state and the theme), because uPlot reads fonts and axis sizes only when it builds; a zoom step is a user action, so one rebuild per step is fine.
- The text-inset rule from #5022 stays: text 1px from the pane's edges, graphics 0px, at any zoom.

## 5. Part C: denser charts and smarter text placement

### C.1 Where the space goes today

Per chart in a simple view (`TimeChart`, built flush since #5022):

| Band | Height |
|---|---|
| Title band above the plot (`.sysinfo-chart-canvas--titled` margin) | 14px |
| uPlot top padding (room for the top y label) | 7px |
| Time axis (`X_AXIS_PX`) | 22px |
| **Text overhead per chart** | **43px** |

A simple-view row is at least 100px (`minmax(100px, 1fr)`), so text takes 43% of a minimum-height chart. All CPU on a 16-core machine is 8 rows × 2 columns: 8 time axes (176px) and 16 titles stacked in one column of the pane, and 16 identical "0%–100%" y axes, the right-hand column's repeating the left's. The Advanced view adds gaps of 8px (sections), 6px (tiles) and 10px (charts), 8px between focus charts, 6/8px tile padding, and a 14px title band over the heatmap.

### C.2 Text placement rules

1. **Titles sit on the plot, not above it.** The title band goes. The title is centred in the plot's top edge, over the plot, and reads over a line through it with a text halo in the pane background (`paint-order: stroke`, or a `text-shadow` ring in `--main-bg-color`), with no box. The top padding that keeps the top y label whole (7px) stays. Saves 14px per chart.
2. **One time axis per column.** In a stack of charts sharing one window (all of them), only the bottom chart of each column draws time labels. The others keep their tick marks and grid (`timeAxis={false}`, a 4px strip), as the Advanced focus view already does. The simple grid gets it from `showsTimeAxis(index, count, columns)`; the focus view already marked its bottom chart. Saves 18px per chart above the bottom row.
3. **One y axis per row when charts share a range.** When every chart in a row has the same y range (All CPU's cores: 0–100%), only the leftmost chart in the row draws y labels; the others keep their grid lines and a tick-only axis. Charts with different ranges (CPU + Mem + Net) keep their own. Saves the y axis's width (about 33px at 100% zoom) on every chart but the first in each row.
4. **Titles keep clear of the floating buttons.** The charts in the top row get `padding-inline-end` equal to the action group's width (eye, pause, table; a CSS variable the bar sets on `.sysinfo-pane`), and centre in the width that is left. A long title still ends in an ellipsis rather than running under the buttons.
5. **Labels never collide with the title.** The title is centred and the top y label sits at the left, so they meet only in a very narrow chart; there the title gives way (ellipsis) and the y label stays whole.
6. **Plot areas line up down a column.** Stacked charts read time straight down, and the shared crosshair (`syncKey`) must not jump sideways between them. Each chart's y axis is only as wide as its own labels, so a CPU chart ("100%") and a network chart ("750 KB/s") would start their plots at different x. `createAxisColumns` (`sysinfo-layout.ts`): each `TimeChart` reports the width its labels need (`onAxisWidth`) and takes its column's widest (`axisWidth`), redrawing its axes when that changes. Used by the simple grid (per column) and the Advanced focus view (one column). Not by Everything, whose columns are decided by CSS from the pane's width.
7. **Tick density follows the chart's height.** y ticks keep uPlot's default 30px apart, scaled by the zoom (`space`), so a short chart shows only its 0 and its top value instead of crowding labels.
8. **Detail drops in steps as a chart shrinks** (owner: "a naturally contracting experience"). A chart shorter than `minFullChartPx(zoom, timeLabels)` (28px of plot × zoom, plus its top padding and time axis) is drawn bare: no axes or grid, its title, line, fill and tooltip kept. Short charts also get fewer ticks first (rule 7). All of it in `sysinfo-layout.ts`, shared by every Sysinfo chart.

Practice behind these (2026-10-18, dataviz guidance plus standard small-multiples practice): share scales when panels compare like with like, and label only the outer edges; never two scales on one chart; recessive 1px solid gridlines; the title names a single series (no legend box) and carries the live figures in text colour; small even gutters instead of a box per panel; a synced crosshair across panels; text scales for legibility, data strokes don't.

### C.3 Spacing

- **One gap for Sysinfo,** `--sysinfo-gap: 2px`, used between simple-view charts (already 2px), Advanced charts (10px today), focus charts (8px), the sections of Everything (8px) and the tiles (6px). Written once, in `sysinfo-plot.scss`.
- **Tiles:** padding 6px 8px → 3px 4px (the text inside still keeps its own room; the border is the tile's edge).
- **Minimum chart heights** (at 100% zoom; they scale with the zoom, B.1): with the text overhead down from 43px to about 11px for charts above the bottom row, a simple-view row can drop from 100px to **64px** and still keep almost the same plot height as a minimum-height chart today (53px against 57px) in two thirds of the space. Advanced chart rows: 120px → **84px**. Focus charts: 70px → **56px**. All CPU on 16 cores then fits in about 8 × 64 = 512px without scrolling, where today it needs 800px+.
- **Heatmap:** its title keeps a reserved band (text over coloured cells reads worse than over a line), but the band follows the zoomed line-height (B.1) instead of a fixed 14px.

### C.4 Before / after, one All CPU chart above the bottom row (100% zoom)

| | Today | After |
|---|---|---|
| Title band | 14px | 0 (title over the plot) |
| Top padding | 7px | 7px |
| Time axis | 22px | 4px (ticks only) |
| y axis width (right-hand column) | ~33px | ~4px (ticks only) |
| Row minimum | 100px | 64px |

## 6. Tests

- **Zoom core:** `zoom-factor.test.ts`: `writePaneZoom` clamps, rounds and writes `null` at 1.0. `zoom.test.ts`: `zoomBlockByWheel` and `usePaneZoomWheel` step by `WHEEL_STEP`, show the indicator, zoom a headless sub-block, ignore a plain wheel and Ctrl+Shift. `block-registry.test.ts`: the holder list gains Help (Part A) and Sysinfo (Part B).
- **Each migrated pane:** its own Ctrl+scroll test now asserts the shared step and indicator (or, for the panes whose handler is deleted, that the global handler zooms them). Help: the one-time `help:zoom` → `term:zoom` move.
- **Sysinfo:** a zoomed pane builds its axes with the scaled font and a larger x axis; titles carry the zoomed size; the plot's line width is unchanged. `showsTimeAxis` and the shared-y rule, for 1, 2 and 3 columns. The title's end padding follows the action group's width.
- **By hand, in `task dev`:** each pane zooms by keys, Ctrl+scroll (with the indicator) and Ctrl+Shift+scroll; Help keeps a previously saved zoom; Sysinfo at 50%, 100%, 150%, 200% in All CPU, CPU + Mem + Net, Everything and each focus view, narrow and wide.

## 7. Phases (two PRs)

1. **Part A, the zoom path**, with this spec. No visible change except that Ctrl+scroll in the terminal, editor, the agent pane's shell, section panes, Warden and Swarm now steps by 0.05 with the indicator, like everywhere else, and Help joins the all-panes zoom and reset.
2. **Parts B and C, Sysinfo text zoom and density**, on top of 1 (B needs `--pane-zoom`; C's sizes are written in its terms).

## 8. Open questions

Taken with the recommended defaults when the owner said to build it (2026-10-18, "do the sysinfo unified zoom, take it to the end"); each is easy to change later:

1. **Ctrl+scroll step:** the shared 0.05 with the indicator, everywhere (A.4).
2. ~~Does the control row zoom?~~ **Decided:** yes, and the chart rows' minimum heights zoom too (B.1).
3. **Gap:** 2px everywhere (C.3).
4. **A new pane's starting zoom:** 100%, as now; an agent still starts at its saved level.
5. **"Between panes":** the charts inside a Sysinfo pane; the gap between AgentMux panes is out of scope.
