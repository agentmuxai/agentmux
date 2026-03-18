# Pane Drag-to-Rearrange Architecture: React → SolidJS Migration Analysis

**Date:** 2026-03-18
**Context:** Pane drag broken since SolidJS migration (PR #120). Fixed with `createEffect`-based drag handle registration.

---

## Original React Implementation (react-dnd)

### How It Worked

Used `react-dnd` library — a higher-level abstraction over HTML5 DnD:

```jsx
// DisplayNode — drag source
const [{ isDragging }, drag, dragPreview] = useDrag(() => ({
    type: tileItemType,
    canDrag: () => !(isEphemeral || isMagnified),
    item: () => node,
    collect: (monitor) => ({ isDragging: monitor.isDragging() }),
}));

// CRITICAL: drag registered ONLY on the header, not the tile
useEffect(() => {
    drag(nodeModel.dragHandleRef);
}, [drag, nodeModel.dragHandleRef.current]);

// OverlayNode — drop target
const [, drop] = useDrop(() => ({
    accept: tileItemType,
    canDrop: (_, monitor) => monitor.isOver({ shallow: true }) && dragItem.id !== node.id,
    drop: (_, monitor) => { if (!monitor.didDrop()) layoutModel.onDrop(); },
    hover: throttle(50, (_, monitor) => { /* compute drop direction */ }),
}));

// TileLayout — global state
const { activeDrag, dragClientOffset } = useDragLayer((monitor) => ({
    activeDrag: monitor.isDragging(),
    dragClientOffset: monitor.getClientOffset(),
}));
```

### What react-dnd Provided

| Feature | How |
|---------|-----|
| Drag handle isolation | `drag(dragHandleRef)` — only header drags |
| Global drag monitoring | `useDragLayer` — any component can check drag state |
| Drop target validation | `monitor.isOver({ shallow: true })` — no nested target confusion |
| Auto preview management | `dragPreview` API |
| State coherence | Jotai atoms + `collect()` callback |
| Lifecycle cleanup | Automatic via React fiber |

---

## SolidJS Implementation (HTML5 DnD)

### What PR #120 Did

Replaced `react-dnd` with raw HTML5 DnD. The critical mistake: set `draggable=true` on the **entire tile-node** instead of just the header.

```tsx
// BROKEN: entire tile is draggable
<div class="tile-node"
    draggable={!isEphemeral() && !isMagnified()}
    onDragStart={onDragStart}
    onDragEnd={onDragEnd}
>
```

### Why It Appeared to Work

Terminal panes (xterm.js) capture all mouse events in their canvas, so drag from content area was naturally blocked. Only the header strip could initiate drag — **by accident, not by design.**

Non-terminal panes (agent, forge, web) didn't have this protection, but nobody tested pane rearrangement with those views.

---

## The Fix: createEffect-Based Drag Handle Registration

### Approach

Register `draggable` and event listeners directly on the `dragHandleRef` element (the pane header), not the tile-node. This is the SolidJS equivalent of `drag(nodeModel.dragHandleRef)`.

```tsx
createEffect(() => {
    const handle = dragHandleRef?.current;
    if (!handle) return;
    const canDrag = !isEphemeral() && !isMagnified();
    handle.draggable = canDrag;

    const handleDragStart = (e: DragEvent) => {
        if (!canDrag) { e.preventDefault(); return; }
        e.dataTransfer?.setData(DRAG_DATA_KEY, props.node.id);
        // set preview image...
        globalDragNodeId = props.node.id;
        globalDragLayoutModel = props.layoutModel;
        props.layoutModel.activeDrag._set(true);
        setIsDragging(true);
    };

    const handleDragEnd = (e: DragEvent) => { /* cleanup */ };

    handle.addEventListener("dragstart", handleDragStart);
    handle.addEventListener("dragend", handleDragEnd);
    onCleanup(() => {
        handle.removeEventListener("dragstart", handleDragStart);
        handle.removeEventListener("dragend", handleDragEnd);
        handle.draggable = false;
    });
});
```

### Why createEffect

- **Reactive tracking:** Re-runs when `isEphemeral()` or `isMagnified()` signals change
- **Ref timing:** `dragHandleRef.current` may be null during first render; effect re-runs when it's set
- **Cleanup:** `onCleanup` removes old listeners before attaching new ones

### Tile-Node Changes

Removed `draggable`, `onDragStart`, `onDragEnd` from the tile-node div:

```tsx
// BEFORE (broken)
<div class="tile-node" draggable={true} onDragStart={...} onDragEnd={...}>

// AFTER (fixed)
<div class="tile-node">
```

---

## Complete DnD Data Flow

```
1. DRAGSTART (header)
   User drags pane header
   → handleDragStart fires on dragHandleRef
   → setData("application/x-tile-node-id", nodeId)
   → setDragImage(previewImage)
   → globalDragNodeId = nodeId
   → layoutModel.activeDrag = true
   → overlayTransform moves overlay from top:10000px to top:0 (visible)

2. DRAGOVER (overlay nodes)
   Cursor moves over overlay node
   → handleDragOver (throttled 50ms)
   → determineDropDirection(rect, cursorOffset) → Top|Bottom|Left|Right|Center
   → treeReducer(ComputeMove) → pendingTreeAction updated
   → placeholderTransform memo recalculates → placeholder shows target position

3. DRAGLEAVE (overlay nodes)
   Cursor leaves overlay node
   → handleDragLeave checks relatedTarget
   → treeReducer(ClearPendingAction) → placeholder hidden

4. WINDOW DRAGOVER (bounds check)
   Cursor near edge or outside layout
   → checkForCursorBounds (debounced 100ms)
   → If outside: ClearPendingAction

5. DROP (overlay node)
   User releases mouse over valid target
   → handleDrop → layoutModel.onDrop()
   → treeReducer(CommitPendingAction) → tree mutated
   → Nodes rearranged → all memos recompute → UI updates
   → persistToBackend()

6. DRAGEND (header)
   Drag completes (drop or cancel)
   → handleDragEnd on dragHandleRef
   → globalDragNodeId = null
   → layoutModel.activeDrag = false
   → overlayTransform moves back to top:10000px (hidden)
```

---

## Key Differences Summary

| Aspect | React (react-dnd) | SolidJS (HTML5 DnD) |
|--------|-------------------|---------------------|
| Drag handle | `drag(dragHandleRef)` — library managed | `createEffect` + addEventListener on ref |
| Drop targets | `useDrop` hook | `onDragOver`/`onDrop` on overlay divs |
| Global state | `useDragLayer` monitor | Module-level globals |
| Preview | `dragPreview()` API | Manual `toPng()` + `setDragImage()` |
| Bounds detection | Library event bubbling | Manual window `dragover` + debounce |
| Cleanup | React fiber lifecycle | `onCleanup()` in `createEffect` |
| Type safety | `monitor.getItem<T>()` | `dataTransfer.getData()` string |

---

## Overlay + Placeholder Architecture

### Overlay Container

Off-screen (`top: 10000px`) flexbox mirror of the visible layout. During drag, moves to `top: 0` to receive drop events. Each `OverlayNode` matches a visible `DisplayNode` 1:1.

### Placeholder

Visual indicator of where the pane will land. Position calculated by `getPlaceholderTransform(pendingAction)` based on the target node's rect and drop direction. CSS-animated transitions.

### Magnified Pane Overlay

Added in PR #134. Magnified pane renders in a dedicated overlay outside `display-container` to bypass CSS stacking context issues.

---

## Files

| File | Purpose |
|------|---------|
| `frontend/layout/lib/TileLayout.tsx` | DisplayNode (drag), OverlayNode (drop), Placeholder, global listeners |
| `frontend/layout/lib/layoutNodeModels.ts` | `dragHandleRef` creation |
| `frontend/layout/lib/layoutModel.ts` | `treeReducer`, `onDrop`, `overlayTransform`, `placeholderTransform` |
| `frontend/layout/lib/types.ts` | `LayoutTreeActionType`, `DropDirection`, `NodeModel` |
| `frontend/layout/lib/utils.ts` | `determineDropDirection()` |
| `frontend/layout/lib/layoutTree.ts` | `computeMoveNode()`, `moveNode()` |
| `frontend/app/block/blockframe.tsx` | Sets `dragHandleRef.current` on pane header |

---

## Edge Cases

1. **Rapid tab switch during drag** — `globalDragLayoutModel` points to old tab's model. Orphaned drags discarded by node ID check.
2. **Preview timing** — `toPng()` is async; if dragstart fires before ready, blank ghost. Mitigated by generating on `pointerenter`.
3. **State change during drag** — If pane magnified while dragging, drag continues (canDrag gate only applies to new drags).
4. **macOS movableByWindowBackground** — Intercepted all background drags. Fixed by removing the call.
