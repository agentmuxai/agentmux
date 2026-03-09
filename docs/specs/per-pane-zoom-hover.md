# Per-Pane Zoom: Hover-Aware Scroll Wheel

## Status: Spec

## Summary

Ctrl+/- (keyboard) zooms the **focused** pane. Ctrl+Scroll (wheel) zooms the pane **under the mouse cursor**, even if it isn't focused.

## Current Behavior (v0.31.86)

Both keyboard and scroll wheel zoom target `getFocusedBlockId()` — the currently selected/focused pane. To zoom a different pane with the scroll wheel, you must first click it to focus it.

## Desired Behavior

| Input | Target |
|-------|--------|
| Ctrl+Plus / Ctrl+Minus | Focused (selected) pane |
| Ctrl+0 | Focused (selected) pane |
| Ctrl+Scroll | Pane under mouse cursor |

This matches VS Code / browser tab behavior — keyboard shortcuts affect the active editor, scroll wheel affects what's under the pointer.

## Implementation

### 1. Identify block under cursor

The scroll wheel event fires on the DOM element under the mouse. Walk up from `event.target` to find the nearest ancestor with a `data-blockid` attribute (already set on block frames).

```ts
function getBlockIdFromEvent(e: WheelEvent): string | null {
    const el = (e.target as HTMLElement).closest("[data-blockid]");
    return el?.getAttribute("data-blockid") ?? null;
}
```

### 2. Split zoom entry points

In `frontend/app/store/keymodel.ts` (or wherever Ctrl+Scroll is handled):

- **Keyboard (Ctrl+/-):** Keep calling `zoomIn(store)` / `zoomOut(store)` which uses `getFocusedBlockId()`.
- **Wheel (Ctrl+Scroll):** Pass the blockId from the event target instead.

### 3. Refactor zoom.ts

Add a `blockId` parameter to `setPaneZoom` and expose target-aware entry points:

```ts
// Zoom a specific block (for scroll wheel)
export function zoomBlockIn(blockId: string, step: number = WHEEL_STEP): void
export function zoomBlockOut(blockId: string, step: number = WHEEL_STEP): void

// Zoom focused block (for keyboard — unchanged API)
export function zoomIn(store: any, step: number = KEYBOARD_STEP): void
export function zoomOut(store: any, step: number = KEYBOARD_STEP): void
```

Internal helpers need a `blockId` param:

```ts
function getBlockZoom(blockId: string): number | null {
    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel || bcm.viewModel.viewType !== "term") return null;
    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    return blockData?.meta?.["term:zoom"] ?? 1.0;
}

function setPaneZoom(blockId: string, factor: number): void { ... }
```

### 4. Wire up the wheel handler

The global wheel handler (in `keymodel.ts` or the app-level `onWheel`) should:

1. Check `e.ctrlKey` (or `e.metaKey` on Mac).
2. Call `getBlockIdFromEvent(e)`.
3. If blockId found and it's a terminal, call `zoomBlockIn`/`zoomBlockOut` based on `e.deltaY`.
4. `e.preventDefault()` to suppress browser zoom.

### 5. Files to modify

| File | Change |
|------|--------|
| `frontend/app/store/zoom.ts` | Add `zoomBlockIn`/`zoomBlockOut`, refactor internals to accept blockId |
| `frontend/app/store/keymodel.ts` | Wire Ctrl+Scroll to `zoomBlockIn`/`zoomBlockOut` with event-derived blockId |

### 6. Edge cases

- **Mouse over non-terminal pane:** Ctrl+Scroll is a no-op (or could fall through to focused pane — TBD).
- **Mouse over focused pane:** Both keyboard and scroll target the same pane — no conflict.
- **Scroll doesn't steal focus:** The hovered pane should NOT become focused just because it was zoomed via scroll.
