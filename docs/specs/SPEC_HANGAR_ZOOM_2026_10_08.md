# Zoom in the Hangar, through the shared pane zoom

**Status:** active — built in #NNNN.
**Date:** 2026-10-08.
**Requested by:** repo owner (asafebgi): "lets introduce zoom into the hangar .. write a separate spec to file"; "that too
should be DRY .. from my understanding zoom is already well DRYly defined currently in the codebase".
**Author:** AgentO.
**Related:** `zoom-architecture.md`, `per-pane-zoom-hover.md`, `SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md`,
`SPEC_FILE_BROWSER_PANE_2026_10_01.md` (the Hangar; says nothing about zoom).

---

## 1. Goal

The Hangar (the Files pane, view `files`) zooms like the other panes: Ctrl/Cmd `+` / `-` / `0`, Ctrl+scroll over the
pane, Ctrl+Shift+scroll and "Reset zoom on all panes". The level is kept per pane and survives a restart.

## 2. The shared zoom, as it is

Pane zoom already has one mechanism; a pane joins it by declaring a capability and applying the level:

| Piece | Where |
|---|---|
| Range and the stored value: `term:zoom` in block meta, 0.5–2.0, `null` at 1.0; `readZoom(meta)`, `clampZoom` | `app/store/zoom-factor.ts` |
| Steps (keys 0.1, wheel 0.05, skipping steps that don't change the font size), writing the level, the on-screen indicator; `zoomBlockIn/Out/Reset`; the all-panes batch | `app/store/zoom.ts` |
| Ctrl/Cmd `+` / `-` / `0` → `view:zoom:in/out/reset`, sent to the focused pane | `keybindings/defaults.ts`, `store/keymodel.ts` |
| Ctrl+scroll over a pane (`AppZoomHandler`), Ctrl+Shift+scroll over any pane (`AppAllPanesZoomHandler`) | `app.tsx` |
| Who takes part: the `paneZoom` capability (`{ baseFontSize }`) on a pane's manifest | `block/pane-tab-registry.ts` |

The agent pane is the model to follow: it declares `paneZoom`, applies `readZoom(meta)` as CSS `zoom`, and has no zoom
handlers of its own (its old local handlers fought the global ones and were removed).

## 3. Design

1. **Join:** `filesPaneTab.capabilities` gains `paneZoom: { baseFontSize: 12 }` (`files.tsx`). 12 px is the Hangar's
   font size (`files.scss`), so the step logic that skips zoom steps which don't change the rendered font size measures
   the right thing. That alone routes the keys, Ctrl+scroll, the all-panes gesture and "Reset zoom on all panes" to it.
2. **Apply:** `FilesView`'s root `.files-view` gets `style={{ zoom: readZoom(ctx.meta()) }}`. CSS `zoom`, not font
   size: the toolbar, Places, the list rows, the thumbnail tiles and their icons all scale together, and the pane has
   no canvas or terminal that needs a 1:1 pixel ratio.
3. **No handlers of its own.** Nothing in the Hangar listens for Ctrl+scroll or Ctrl `+`/`-`; the global ones do it
   all, with the shared steps and the indicator.

### Behaviour under zoom (checked, not changed)

- **Narrow layout.** `.files-view` is also the `@container` that drops Places in a narrow pane. With `zoom` on that
  element the container measures itself in zoomed pixels, so zooming in narrows the pane in effect and Places drops
  sooner, the same way Help's and the section panes' container queries respond to their zoom.
- **The windowed list.** Rows (24 px), thumbnail tiles (112 × 132) and the `scrollTop` / `clientHeight` /
  `clientWidth` the list windows from are all in the zoomed subtree's own pixels, so the maths is unchanged; the list's
  ResizeObserver fires when the zoom changes the pane's size in those pixels, so the visible rows and grid columns are
  recomputed.
- **Menus and overlays** (the context menu, the conflict dialog) render in a portal outside the zoom, at the pointer.
  Drag and drop goes by target elements, not coordinates.
- **Thumbnails** are generated at 2× the tile size: sharp up to 2× zoom on a normal display, slightly soft at 2× on a
  high-density one.

## 4. Tests

- `block-registry.test.ts`: `files` is among the `paneZoom` holders, with base font size 12.
- `files-view.test.tsx`: the root's `zoom` follows `term:zoom` in the block meta (1 when unset).
- `zoom.test.ts`'s all-panes tests cover the batch for every holder.

## 5. Later: the duplication that is left (not in this change)

The core is shared, but five panes still carry the same local Ctrl+scroll handler (`STEP = 0.1`, clamp 0.5–2.0, `null`
at 1.0, capture phase, stop propagation): the terminal (`term.tsx`), the editor (`editor-view.tsx`), the section panes
(`section-pane.tsx`), Warden (`warden-view.tsx`) and Swarm (`swarm-view.tsx`, which also repeats the global `+`/`-`/`0`
keys). In those panes Ctrl+scroll steps by 0.1 with no indicator; everywhere else it steps by 0.05 with one. Help keeps
its own key (`help:zoom`), range and handlers.

- Only panes whose content swallows the wheel (xterm, CodeMirror) need a local hook; one shared
  `usePaneZoomWheel(el, blockId)` in `zoom.ts` calling `zoomBlockIn/Out` would replace those two.
- The section panes, Warden and Swarm can drop theirs: the global handler already covers them.
- A `writeZoom(setMeta, z)` helper for the `null`-at-1.0 idiom.
- Help onto `paneZoom` and `term:zoom` (it would then join the all-panes zoom).
