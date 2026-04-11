# Bug: Right-click context menu stops working on empty tab after closing all panes

**Date:** 2026-04-09
**Severity:** UX bug — blocks discoverability of widget launcher

## Symptom

1. Fresh startup with no panes → right-click on blank background → widget menu appears (works)
2. Open a terminal (or any pane) → close it → back to blank background
3. Right-click on blank background → nothing happens (broken)

## Root Cause

The TileLayout's `.display-container` div is `position: absolute; width: 100%; height: 100%` with a z-index, creating a full-size invisible overlay that intercepts the `contextmenu` event.

### Why it works on first startup

On first startup, the `TileLayout` component mounts but the `.display-container` has no content. The right-click event propagates from the empty `.display-container` through to the parent div (in `tabcontent.tsx:94`) because there are no child elements to consume it.

**Actually, the real reason it works initially:** On first launch, the tab may not exist yet (Show gate at `tabcontent.tsx:97` returns false), so TileLayout never mounts. The parent div receives the click directly.

### Why it breaks after closing panes

After opening and closing panes:
1. The tab exists (`tabData() != null` → Show gate stays true)
2. TileLayout stays mounted with an empty `.display-container`
3. The `.display-container` is a 100% × 100% absolutely-positioned div
4. It sits **above** the parent div in z-order
5. The `contextmenu` event fires on `.display-container`, not on the parent div
6. Nothing handles it on `.display-container` → event is consumed with no effect

The parent div's `onContextMenu` handler (tabcontent.tsx:94) never fires because the event target is the `.display-container`, and the event doesn't bubble to the parent div's handler since `.display-container` is a child of `.tile-layout`, which IS inside the parent div — so bubbling should work.

**Wait — bubbling SHOULD work.** The parent div contains `.tile-layout` which contains `.display-container`. A `contextmenu` event on `.display-container` should bubble up to the parent div.

### Revised diagnosis: the handler's guard clause

Looking more carefully at the handler:

```typescript
const handleContextMenu = (e: MouseEvent) => {
    const tab = tabData();
    if (!tab || (tab.blockids?.length ?? 0) > 0) return;  // LINE 82
    ...
};
```

The guard checks `tab.blockids?.length`. After closing the last pane:
- `DeleteBlock` removes the block from `tab.blockids` on the backend
- But the frontend's reactive `tabData()` atom may not have updated yet
- If `tabData()` still shows the old `blockids` with the deleted block ID, `length > 0` → handler returns early

**This is a race condition:** The `DeleteBlock` RPC completes and the block is removed from the DB, but the frontend's WaveObj atom for the tab hasn't received the update event yet. During this window, the context menu handler sees stale `blockids` and bails out.

After the atom updates, subsequent right-clicks should work — but by then the user has already concluded it's broken.

**Alternative theory:** The blockids array IS updated, but there's another issue. The `onNodeDelete` callback in the TileLayout triggers `DeleteBlock`, which updates the tab's blockids. The layout tree's rootnode becomes undefined, but the tab atom updates reactively. The handler should see `blockids.length === 0` on the next right-click.

Let me reconsider: **Does the tab get a new blockids update when the last block is deleted?**

In `block.rs:delete_block()`:
```rust
tab.blockids.retain(|id| id != block_id);
store.update(&mut tab)?;
```

The tab IS updated in the DB. The frontend receives a WaveObjUpdate for the tab via WPS. So `tabData().blockids` should eventually become `[]`.

### Most likely cause: event target and pointer-events

The `.display-container` has `pointer-events: auto` by default. Even when empty, it captures mouse events. The `contextmenu` event fires on `.display-container` and bubbles up to the parent div — **this should work**.

Unless: the TileLayout's `.tile-layout` div or the `.display-container` has an `onContextMenu` handler that calls `stopPropagation()`.

**Check for stopPropagation on display-container or tile-layout.**

## Proposed Fix

Regardless of the exact cause, the fix is straightforward — add the context menu handler directly to the TileLayout's display-container (or the tile-layout div), so it works whether or not events bubble correctly:

### Option A: Forward context menu from TileLayout (cleanest)

Add an `onContextMenu` prop to TileLayout that fires when clicking empty space:

```typescript
// In TileLayout.win32.tsx — on the display-container div:
onContextMenu={(e) => {
    // Only fire if clicking empty space (no leaf node under cursor)
    if (e.target === displayContainerRef.current) {
        props.contents.onEmptyContextMenu?.(e);
    }
}}
```

### Option B: Add handler in tabcontent.tsx on the tile-layout wrapper

Wrap TileLayout in a div that handles contextmenu and doesn't rely on bubbling.

### Option C: Use pointer-events: none on empty display-container

```scss
.display-container:empty {
    pointer-events: none;
}
```

This lets clicks pass through to the parent when the container has no children. But `:empty` might not work if there are whitespace text nodes.

## Files

| File | Line | Role |
|------|------|------|
| `frontend/app/tab/tabcontent.tsx` | 80-94 | Context menu handler + guard clause |
| `frontend/layout/lib/TileLayout.win32.tsx` | 129-151 | Display container that may intercept events |
| `frontend/layout/lib/tilelayout.scss` | 12-27 | Absolute positioning + z-index on display-container |
| `agentmux-srv/src/backend/wcore/block.rs` | 36-46 | Backend delete_block removes from tab.blockids |
