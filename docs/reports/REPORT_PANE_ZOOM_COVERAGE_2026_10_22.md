# Report — Which panes still lack the shared pane zoom, and how to add it

**Date:** 2026-10-22
**Author:** Maricon@charlie
**Type:** Report (an audit, and a plan)
**Status:** proposed
**Requested by:** the operator: *"lets introduce zoom to any newly recently added panes that lack it, we have a unified zoom system, i notice tower is still does have zoom, write a report to file"*
**Builds on:** `docs/specs/SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md` (the one zoom path, #5128), `docs/specs/zoom-architecture.md`, `docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md`, `docs/specs/SPEC_HANGAR_ZOOM_2026_10_08.md`.

## 1. Summary

Pane zoom has one path since #5128. A pane takes part by declaring the `paneZoom` capability in its manifest and applying `readZoom(meta)` to one of its own elements. Everything else (Ctrl/Cmd `+` `-` `0`, Ctrl+scroll with the indicator, Ctrl+Shift+scroll for all panes, "Reset zoom on all panes", saving the level in `term:zoom`) then works with no code of its own.

Six of the panes a user can open don't declare `paneZoom`. Three take it the simple way, with one line in the manifest and one on the root element: **Tower**, **Toolchain** and **Settings**. Two need more care: **Drone**, which has its own canvas zoom, and **installed widgets**, which run in an iframe the window's gestures can't reach. The **Launcher** probably isn't worth it. Browser panes stay out on purpose.

## 2. Which panes zoom today

Code read on main at `cc46a8c21`. The built-in panes are registered in `frontend/app/block/block-registry.ts`; the widgets the user picks from are in `crates/srv/src/config/widgets.json`.

| Pane (view) | Shared zoom | Added | Notes |
|---|---|---|---|
| Terminal (`term`), Agent (`agent`), Editor (`editor`), Hangar (`files`), Sysinfo (`sysinfo`), Help (`help`), Swarm (`swarm`), Warden (`warden`, also Kanban and Vault), Connectors (`connectors`), Memory (`memory`) | **yes** | | |
| **Tower** (`tower`) | **no** | 2026-10-08 | `capabilities: { noPadding: true }` only (`view/tower/tower.tsx`) |
| **Installed widgets** (`ext:<id>/<pane>`) | **no** | 2026-10-10 | `capabilities: { noPadding: true }` (`block/sandboxed-widget-host.tsx`) |
| **Toolchain** (`toolchain`) | **no** | 2026-06-25 | no capabilities |
| **Settings** (`settings`) | **no** | 2026-06-25 | no capabilities |
| **Drone** (`drone`) | **no** | 2026-05-18 | has its own graph zoom (plain wheel, 0.2–2.5) |
| Launcher (`launcher`) | no | 2026-03-03 | the picker an empty pane shows; `header: "none"` |
| Identity (`identity`) | no | 2026-05-08 | legacy: no current UI opens one (`view/bundle-summary.tsx`) |
| Browser (`browser`), and the Messengers group (Discord, Slack, …, all browser panes) | no, on purpose | | Chromium's own page zoom (the one-zoom-path spec, A.7) |

What "no" means for the user today: in those panes, Ctrl/Cmd `+` `-` `0` and Ctrl+scroll do nothing, the all-panes gesture and "Reset zoom on all panes" skip them, and only chrome zoom (title bar, status bar, pane headers) changes them.

**A test gap that let this slip:** `frontend/app/block/block-registry.test.ts` asserts the full list of `paneZoom` holders, but only over its own `VIEWS` list. That list doesn't include `tower` or `files`, so a new pane that forgets zoom can't fail the test. Hangar does zoom, and is checked on another line.

## 3. Per pane: what it takes

### 3.1 Tower: simple, and the one you noticed

- **Content:** DOM tables, sort headers, a sticky header row, and an SVG history chart (`tower-history.tsx`). The chart's hover maps the pointer with `(clientX - box.left) / box.width`. Both values are in screen pixels and the ratio doesn't depend on zoom, so it stays correct under CSS zoom.
- **Change:** `paneZoom: { baseFontSize: 13 }` (its text is `--text-base`, 13 px) and `style={{ zoom: readZoom(ctx.meta()) }}` on `.tower-view` (`tower-view.tsx:93`).
- **Container queries:** Tower has eight `@container` rules (`tower-view.scss`), which drop columns at 680 / 560 / 480 px. See D1 for whether they should respond to zoom.
- **Heads-up:** Korp@narko changed Tower today (the Panes view). Tell them before editing.

### 3.2 Toolchain: simple

- **Content:** a DOM list (`toolchain-view.tsx:612`, `.toolchain-view.ui-density-compact`), with no container queries and no pointer math.
- **Change:** `paneZoom: { baseFontSize: 12 }` (`--text-sm`) and `zoom` on `.toolchain-view`.

### 3.3 Settings: simple

- **Content:** a DOM form with a search bar and a container named `settings` on `.settings-view-container` (`settings.scss:14`), whose layout changes below 479 px and above 1024 px.
- **Change:** `paneZoom: { baseFontSize: 12 }` and `zoom` on `.settings-view-container` or an element inside it (D1).

### 3.4 Drone: needs a decision

Drone already zooms its graph: a plain wheel over the canvas scales `.drone-viewport` by `translate(x, y) scale(z)` (`drone-view.tsx:160-186`), and its node drag and wheel anchoring work in screen pixels divided by that scale. Two things follow:

1. **Gesture clash.** Drone's `onWheel` doesn't check Ctrl/Cmd and doesn't stop the event, so with `paneZoom` a Ctrl+scroll over the canvas would zoom both the graph and the pane. Drone's handler has to let a Ctrl/Cmd wheel through to the shared handler.
2. **Pointer math under CSS zoom.** If CSS `zoom` is applied above the canvas, the pointer offsets (screen pixels) and the viewport translate (the zoomed element's own pixels) disagree by the pane zoom. Nodes would drift under the cursor while being dragged, and the wheel would anchor at the wrong point. The editor deals with this by dividing pointer deltas by the level.

**Recommended:** pane zoom scales Drone's text and controls (`.drone-bar`, `.drone-controls`, the node fields, `.drone-runpanel`), but not the canvas, which keeps its own graph zoom. This is the selective pattern Sysinfo uses (`--pane-zoom` from the host, A.2 of the one-zoom-path spec). The alternative is to zoom the whole pane and divide every pointer path by the level.

### 3.5 Installed widgets: needs the SDK

A widget runs in a sandboxed iframe, a separate document. A Ctrl+scroll or a key press inside it never reaches the window's handlers, and CSS `zoom` on the `<iframe>` element is not a reliable way to scale what's inside. The parts:

1. **The level reaches the widget.** The SDK sends the zoom in the connect context and on every change, as it already does for the theme (`sdk/widget-sdk/v1.d.ts`: `theme`, `meta`). `am-widget.css` applies it on `:root` (e.g. `font-size: calc(... * var(--am-zoom))` or `zoom`), so a widget built on the theme stylesheet zooms with no code of its own.
2. **Gestures leave the iframe.** The SDK forwards Ctrl/Cmd+wheel and `+` `-` `0` to the host, which steps the pane's `term:zoom` (`zoomBlockByWheel`, `zoomBlockIn/Out/Reset`).
3. **The host declares `paneZoom: {}`**, so the all-panes gesture and the reset include widgets.

This adds to the published SDK v1 surface (one optional field and one event), so it's a separate PR with its own `v1.d.ts` and README change.

### 3.6 Launcher: not recommended

It's the chooser an empty pane shows, briefly, before it becomes another pane. Zooming it means the level would carry into whatever pane is picked next, which is surprising. Skip it unless you want it.

### 3.7 Identity: no

It's legacy: no current UI creates one.

## 4. Plan

1. **PR 1: Tower, Toolchain, Settings.** In each manifest, `paneZoom` with its base font size, plus one `zoom` line on its root element. Add `tower` and `files` to `VIEWS` in `block-registry.test.ts`, so the holder list covers every built-in pane. The holder list gains `settings`, `toolchain` and `tower`. A test per pane checks that Ctrl+scroll over it steps its `term:zoom` through the shared handler (as Warden's and Swarm's do).
2. **PR 2: Drone.** It declares `paneZoom`, its `onWheel` lets Ctrl/Cmd through, and `--pane-zoom` scales its bar, controls, node fields and run panel, but not the canvas. Tests: a Ctrl+wheel over the canvas steps the pane zoom and leaves the graph zoom alone, and a plain wheel still zooms the graph.
3. **PR 3: Widgets.** The SDK gets the zoom field and change event, `am-widget.css` applies it, gestures are forwarded, and the host declares `paneZoom`, with the README and `v1.d.ts` updated.

Each zooming pane is then checked by hand in `task dev`: keys, Ctrl+scroll with the indicator, Ctrl+Shift+scroll, and the reset, at 50%, 100%, 150% and 200%, narrow and wide.

## 5. Decisions for the operator

| # | Question | Recommended | Alternative |
|---|---|---|---|
| D1 | Do Tower's and Settings' narrow layouts respond to zoom? | **Yes**: apply `zoom` on or above the container, so at 200% a 900 px pane lays out like a 450 px one, as a browser's zoom does (the text needs the room) | Zoom an element inside the container, so the layout keeps its width breakpoints (Warden's and the section panes' choice) |
| D2 | Drone | **Zoom its text and controls, not the graph**, which keeps its own zoom | Zoom the whole pane, and divide every pointer path by the level |
| D3 | Widgets in this round? | **Yes, as PR 3** (they're new and will multiply) | Later |
| D4 | Launcher | **No** | Zoom it too |
