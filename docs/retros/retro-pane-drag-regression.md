# Retro: Pane Title Drag-to-Rearrange Regression

**Date:** 2026-03-18
**Severity:** High — core layout interaction broken on both platforms
**Introduced:** PR #120 (SolidJS migration, commit `f030661`, 2026-03-09) — Windows
**Worsened:** PR #145 (macOS resize fix, commit `458f18f`, 2026-03-15) — macOS
**Fixed:** This session (2026-03-18)

---

## Symptoms

- **Windows:** Dragging a pane title bar does nothing — no drag ghost, no rearrangement
- **macOS:** Dragging a pane title bar moves the entire application window instead of rearranging panes

## Root Causes (Two Separate Bugs)

### Bug 1: SolidJS Migration Lost Drag Handle Registration (Windows + macOS)

**Original React code** (`TileLayout.tsx`, pre-PR #120):
```jsx
// React: register drag specifically on the header element
useEffect(() => {
    drag(nodeModel.dragHandleRef);
}, [drag, nodeModel.dragHandleRef.current]);
```

This used a DnD library (`react-dnd` or similar) to register the drag handler on the `dragHandleRef` — the pane header element. Only the header initiated drag.

**SolidJS migration** (PR #120) replaced this with:
```tsx
<div class="tile-node"
    draggable={!isEphemeral() && !isMagnified()}
    onDragStart={onDragStart}
>
```

This made the **entire tile** `draggable=true`, not just the header. The `dragHandleRef` was still set on the header but **never used to gate drag initiation**. The `onDragStart` handler accepted drag from anywhere in the tile.

**Why it appeared to work:** On a terminal pane, the xterm.js canvas captures all mouse events, so drag never bubbled up from the content area. You could only initiate drag from the thin header strip — which happened to be the correct behavior by accident. But it was still technically broken for non-terminal panes (agent, forge, etc.) where content doesn't capture mouse events.

**Fix applied:**
```tsx
const onDragStart = (e: DragEvent) => {
    // Only allow drag from the header (dragHandleRef), not the entire tile
    const handle = dragHandleRef?.current;
    if (handle && !handle.contains(e.target as Node)) {
        e.preventDefault();
        return;
    }
    // ... proceed with drag
};
```

### Bug 2: macOS `setMovableByWindowBackground` Captures All Drags (macOS only)

**PR #145** (macOS resize fix) added:
```rust
ns_window.setMovableByWindowBackground(true);
```

This tells macOS to treat ANY click-drag on the window background as a window move. Since the pane header is "background" from NSWindow's perspective (it's not a native control), macOS intercepts the pointer event before HTML5 DnD can fire.

This was carried into the platform module refactor (PR #165) in `platform/macos.rs`.

**Fix applied:** Removed `setMovableByWindowBackground(true)`. Window drag on macOS now uses `data-tauri-drag-region` on the window header element — same mechanism as Windows.

## Timeline

| Date | PR | What Happened |
|------|-----|---------------|
| 2026-03-09 | #120 SolidJS migration | `drag(dragHandleRef)` replaced with `draggable=true` on tile-node. Drag handle gating lost. |
| 2026-03-15 | #145 macOS resize fix | `setMovableByWindowBackground(true)` added. macOS now intercepts all background drags for window move. |
| 2026-03-17 | #161 Window drag dead spots | Unrelated to this bug, but investigation led to discovery. |
| 2026-03-18 | #165 Platform module | Carried `movableByWindowBackground` into `macos.rs`. macOS traffic lights fixed but pane drag still broken. |
| 2026-03-18 | This fix | Both bugs fixed. |

## Why It Wasn't Caught

1. **Terminal panes masked it:** xterm.js canvas captures mouse events, so drag from terminal content area was always blocked. The header was the only place drag could start — making it seem like drag-handle gating was working.
2. **macOS-only second bug:** `movableByWindowBackground` only affects macOS. Windows testers wouldn't see the window-move behavior.
3. **No automated layout DnD test:** The tile rearrangement interaction is complex (HTML5 DnD + layout model) and not covered by any test.
4. **SolidJS migration was large:** PR #120 was a full framework migration. The subtle difference between "register drag on handle" vs "draggable on tile" was easy to miss in review.

## Lessons

1. **When migrating framework-specific APIs (react-dnd → HTML5 DnD), verify the drag initiation point.** The React library handled drag-handle restriction internally; the vanilla HTML5 DnD replacement needs an explicit check.

2. **`setMovableByWindowBackground` is a footgun.** It sounds like "drag from the header background" but it means "drag from ANY background pixel in the entire window." Never use it when the app has its own DnD interactions.

3. **Test pane rearrangement on every PR that touches drag/layout/window chrome.** Manual test checklist item: "drag a pane title to rearrange — verify it rearranges the pane, doesn't move the window."

## Files Changed

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.tsx` | Gate `onDragStart` to only fire from `dragHandleRef` (header) |
| `src-tauri/src/platform/macos.rs` | Remove `setMovableByWindowBackground(true)` |
