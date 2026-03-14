# Spec: macOS Window Resize Handles Missing After SolidJS Migration

## Problem

Window border resize handles work on Windows but not macOS. Worked on macOS before the SolidJS migration (PR #120), broke after.

## What Changed

The Tauri config, Rust code, CSS, and HTML structure are **identical** between React and SolidJS builds. The only relevant changes are in the JS layer:

### 1. react-dnd removed, replaced with native HTML5 drag

React version:
```tsx
import { useDrag, useDrop, useDragLayer } from "react-dnd";
import { HTML5Backend } from "react-dnd-html5-backend";

<DndProvider backend={HTML5Backend}>
    <Workspace />
    <CrossWindowDragMonitor />
</DndProvider>
```

SolidJS version:
```tsx
// No DnD provider. Native HTML5 drag via draggable attribute + dataTransfer.
<Workspace />
<CrossWindowDragMonitor />
```

`react-dnd-html5-backend` registers **global document-level event listeners** for `dragstart`, `dragover`, `dragend`, `drop`, etc. These listeners use `addEventListener` with specific options and call `preventDefault()` / `stopPropagation()` in various cases. Removing this layer changes how pointer and drag events propagate through the DOM.

### 2. `<Suspense>` wrapper removed from TileLayout

React version wrapped the tile layout in `<Suspense>`, SolidJS does not. This changes the initial render timing — React defers rendering until all lazy children resolve, potentially allowing native window chrome to handle early pointer events. Without Suspense, SolidJS renders immediately, and the full-bleed content captures all events from the start.

### 3. DisplayNode now has `draggable={true}` on the tile div itself

React version used `react-dnd`'s `useDrag` hook which attaches drag behavior to a separate drag handle ref. SolidJS puts `draggable={true}` directly on the tile node div:

```tsx
<div
    ref={tileNodeRef}
    id={props.node.id}
    draggable={!isEphemeral() && !isMagnified()}
    onDragStart={onDragStart}
    onDragEnd={onDragEnd}
    ...
>
```

When every display node div is natively `draggable`, the browser may handle pointer events at window edges differently — especially on macOS WebKit where native drag initiation competes with NSWindow resize hit-testing.

## Root Cause Analysis

On macOS with `decorations: false` + `transparent: true`, window edge resize relies on NSWindow providing resize hit-test areas that the WebView doesn't intercept. This is fragile — it depends on how the WebView handles events near the edges.

**Why it worked with React:** `react-dnd-html5-backend` managed drag state through its own event layer, and individual tile nodes were NOT natively `draggable`. The HTML5Backend only made elements draggable when a drag actually started (via `connectDragSource`). At rest, no elements had `draggable=true`, so pointer events at window edges could fall through to the native NSWindow resize handler.

**Why it broke with SolidJS:** Every display node div now has `draggable={true}` at all times. On macOS WebKit, a `draggable` element near the window edge intercepts the pointer event before it reaches the NSWindow resize hit-test area. The browser's drag initiation logic takes priority over the window manager's resize logic.

## Solution

### Option A: Only set `draggable` during active drag intent (recommended)

Match React-DnD's behavior — don't set `draggable={true}` on tile nodes by default. Only make them draggable when the user starts a drag gesture (e.g., on long-press of the drag handle, or when pointer-down on the header bar):

```tsx
// In DisplayNode:
const [isDragReady, setIsDragReady] = createSignal(false);

<div
    ref={tileNodeRef}
    draggable={isDragReady()}
    ...
>
    <div
        class="drag-handle"
        onPointerDown={() => setIsDragReady(true)}
        onPointerUp={() => setIsDragReady(false)}
    />
    ...
</div>
```

This restores the React-era behavior where tile nodes are not natively draggable at rest, allowing macOS NSWindow edge resize to work.

### Option B: CSS `pointer-events: none` border zone

Add an invisible border zone (4-6px) around the window content that has `pointer-events: none`, allowing native resize events to pass through to the NSWindow:

```scss
// In app.scss
#main {
    // Inset content slightly so native resize areas at edges are reachable
    padding: 4px;
}
```

Simple but loses 4px of usable space on all edges.

### Option C: Native NSWindow style mask fix (fallback)

If the above don't work, explicitly set `NSWindowStyleMaskResizable` on the NSWindow after creation in Rust code. This is a deeper fix but may not help if the WebView is still consuming the events.

## Implementation Plan

1. **Try Option A first** — conditionally set `draggable` only on drag-handle interaction, not on the entire tile node. This is the most targeted fix and matches what react-dnd did.
2. **Verify on macOS** — resize cursor appears at window edges, drag to resize works.
3. **Verify drag-and-drop still works** — pane reorder, cross-window drag, file drop all still function.

## Files to Change

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.tsx` | Conditionally set `draggable` on DisplayNode — only during active drag intent |

## Notes

- Windows is unaffected because Win32's `WM_NCHITTEST` provides resize zones below the WebView layer regardless of `draggable` attributes.
- This is a WebKit-specific behavior on macOS — `draggable` elements near window edges compete with native resize hit-testing for frameless windows.
