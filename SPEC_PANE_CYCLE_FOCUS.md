# Spec: Tab / Ctrl+Tab Pane Focus Cycling (Spiral)

**Date:** 2026-03-05
**Author:** Agent2
**Branch:** agent2/tab-pane-cycling
**Status:** Implemented

---

## Motivation

Users need a quick way to cycle focus between panes (blocks) without using the mouse or remembering directional shortcuts. `Ctrl+Shift+Arrow` exists for directional navigation, but a simple `Tab` / `Ctrl+Tab` cycle is more intuitive — similar to how `Alt+Tab` cycles windows.

## Behavior

### `Tab` — Spiral Inward (Clockwise)

Cycles focus through panes in a **clockwise spiral** pattern that moves from the outer edges toward the center:

```
For a 2x2 grid:                For a 3-pane layout:
┌─────┬─────┐                  ┌─────┬─────┐
│  1  │  2  │                  │     │  2  │
│     │     │                  │  1  ├─────┤
├─────┼─────┤                  │     │  3  │
│  4  │  3  │                  └─────┴─────┘
└─────┴─────┘
Tab: 1→2→3→4→1...              Tab: 1→2→3→1...
```

### `Ctrl+Tab` — Spiral Outward (Counter-Clockwise)

Reverses the spiral order.

### Focus Guard

Tab is only intercepted for cycling when:
- No text input/textarea/contenteditable is focused
- Not inside a terminal (`.xterm`) — Tab is needed for shell completion
- AI panel does not have focus

Otherwise Tab passes through normally.

## Implementation

### Algorithm

Uses actual screen coordinates (`additionalProps[nodeId].rect`) to compute clockwise spiral order:

1. Find bounding box of all remaining panes
2. Identify outer ring (panes touching bounding box edges)
3. Sort outer ring clockwise by angle from center using `atan2`
4. Add to result, remove from remaining, repeat with interior panes

### Files Modified

- `frontend/layout/lib/layoutGeometry.ts` — Added `computeSpiralOrder()` function
- `frontend/layout/lib/layoutModel.ts` — Added `spiralLeafOrder` derived atom
- `frontend/app/store/keymodel.ts` — Added `Tab`/`Ctrl+Tab` key bindings, `cyclePaneFocus()`, `shouldInterceptTabForCycle()`

## Testing

- [ ] 2x2 grid: Tab cycles TL→TR→BR→BL→TL (clockwise spiral)
- [ ] 2x2 grid: Ctrl+Tab cycles reverse
- [ ] Side-by-side: Tab cycles L→R→L
- [ ] L + 2 stacked R: Tab cycles L→TR→BR→L
- [ ] Tab passes through to terminal for shell completion
- [ ] Tab passes through to AI panel input
- [ ] Single-pane: no-op
- [ ] Magnified pane: only one visible, cycles through all
