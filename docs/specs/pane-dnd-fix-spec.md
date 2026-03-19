# Spec: Fix Pane Drag-and-Drop — Alternative Approaches

**Date:** 2026-03-19
**Status:** Proposal — **REVISED after research**
**Problem:** HTML5 native DnD is fundamentally broken in Tauri v2 + WebView2 on Windows.

---

## Root Cause: WebView2 + HTML5 DnD Is A Known Platform Bug

**This is NOT a SolidJS reactivity issue.** The HTML5 DnD API itself is broken in WebView2:

1. **Tauri's `dragDropEnabled` (default: true) intercepts ALL drag events** for its own file-drop system, preventing HTML5 DnD from working. ([tauri-apps/tauri#4168](https://github.com/tauri-apps/tauri/issues/4168))
2. **Even with `dragDropEnabled: false`, WebView2 freezes the canvas during HTML5 drag** — dragstart fires but the webview doesn't repaint until the drag ends. ([tauri-apps/tauri#9445](https://github.com/tauri-apps/tauri/issues/9445))
3. **WebView2's `WINDOW_TO_VISUAL` hosting mode breaks DnD entirely** ([MicrosoftEdge/WebView2Feedback#5486](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5486))

**Any solution using the HTML5 DnD API (draggable, dragstart, dragover, drop, dataTransfer) will NOT work in Tauri WebView2 on Windows.** This rules out react-dnd, Pragmatic DnD, and all our manual HTML5 DnD attempts.

**Tab drag works** only because it's a simpler interaction that doesn't need overlay repositioning during the drag — the tabbar is always visible.

Additionally, our SolidJS reactive updates compound the issue:

1. `dragstart` fires → our handler sets `activeDrag` signal → SolidJS immediately flushes DOM changes (overlay repositioning, CSS class changes) → WebView2 sees the drag source element mutated mid-event → cancels the drag
2. Deferring state updates with `setTimeout` allows the drag to sustain, BUT the overlay drop zones are off-screen (positioned at `top: 10000px`) until `activeDrag` is set, creating a catch-22
3. The window/document-level `dragover` handler with `e.preventDefault()` doesn't change the cursor from 🚫 to move — suggesting WebView2's drag subsystem isn't fully engaging

**Tab drag works** because tabs use a simpler pattern: `draggable={true}` + `onDragStart={fn}` with no reactive state changes during dragstart, and the drop target (tabbar) is always visible (not gated behind an `activeDrag` signal).

---

## Option A: Pragmatic Drag and Drop (Recommended)

**Library:** `@atlaskit/pragmatic-drag-and-drop`
**Size:** ~4.7KB min+gzip (core only)
**Maintained by:** Atlassian (active, used in Jira/Confluence)
**Framework:** Vanilla JS — works with SolidJS directly, no bridge needed
**DnD approach:** Uses browser's built-in DnD but with proper event management

### Why This Option

- Framework-agnostic vanilla JS — call `draggable()` and `dropTargetForElements()` on DOM elements
- Already has a [SolidJS + Tailwind + Vite example](https://github.com/atlassian/pragmatic-drag-and-drop/discussions/71)
- Handles the dragstart/dragover/drop lifecycle correctly across browsers
- Supports custom drop zones (not just sortable lists)
- Tiny bundle — just the core package, no optional Atlassian UI deps
- No React dependency

### Integration Plan

```
npm install @atlaskit/pragmatic-drag-and-drop
```

**Step 1: Make pane headers draggable**

In `blockframe.tsx` — use `onMount` to register the header as a draggable:

```tsx
import { draggable } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';

function BlockFrame_Header(props) {
    let headerRef: HTMLDivElement;

    onMount(() => {
        if (props.preview) return;
        const cleanup = draggable({
            element: headerRef,
            getInitialData: () => ({ nodeId: props.nodeModel.nodeId }),
            onDragStart: () => {
                props.nodeModel.dragHandlers?.onDragStart();
            },
            onDrop: () => {
                props.nodeModel.dragHandlers?.onDragEnd();
            },
        });
        onCleanup(cleanup);
    });

    return <div ref={headerRef} class="block-frame-default-header" ...>
}
```

**Step 2: Make overlay nodes drop targets**

In `TileLayout.tsx` — register overlay nodes as drop targets:

```tsx
import { dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';

const OverlayNode = (props) => {
    let overlayRef: HTMLDivElement;

    onMount(() => {
        const cleanup = dropTargetForElements({
            element: overlayRef,
            getData: () => ({ nodeId: props.node.id }),
            onDragEnter: ({ source }) => {
                // Compute drop direction, show preview
                const dragNodeId = source.data.nodeId;
                // ... existing ComputeMove logic
            },
            onDragLeave: () => {
                // Clear pending action
            },
            onDrop: ({ source }) => {
                // Execute the move
                props.layoutModel.onDrop();
            },
        });
        onCleanup(cleanup);
    });

    return <div ref={overlayRef} class="overlay-node" .../>
};
```

**Step 3: Activate overlays**

The key insight: pragmatic-dnd fires `onDragStart` AFTER the browser has fully committed the drag. So we can safely set `activeDrag` in the `onDragStart` callback without killing the drag:

```tsx
onDragStart: () => {
    layoutModel.activeDrag._set(true);  // Safe — drag already committed
    setIsDragging(true);
},
```

**Step 4: Monitor for cross-window**

```tsx
import { monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';

// In CrossWindowDragMonitor or TileLayoutComponent:
onMount(() => {
    const cleanup = monitorForElements({
        onDragStart: ({ source }) => { /* track */ },
        onDrop: ({ source, location }) => { /* handle */ },
    });
    onCleanup(cleanup);
});
```

### Files to Modify

| File | Changes |
|------|---------|
| `package.json` | Add `@atlaskit/pragmatic-drag-and-drop` |
| `blockframe.tsx` | Register header as `draggable()` via `onMount` |
| `TileLayout.tsx` | Register overlay nodes as `dropTargetForElements()` |
| `TileLayout.tsx` | Remove old HTML5 DnD handlers (onDragStart/onDragOver/onDrop from JSX) |
| `types.ts` | Update `dragHandlers` type to match pragmatic-dnd callbacks |
| `layoutNodeModels.ts` | No change needed |
| `CrossWindowDragMonitor.tsx` | Use `monitorForElements` instead of document dragend listener |

### Risks

- Cross-window drag may need additional work (pragmatic-dnd is single-window by default)
- Custom drag preview images may need `setCustomNativeDragPreview` from pragmatic-dnd
- Need to verify pragmatic-dnd works in Tauri WebView2 (likely yes — it wraps HTML5 DnD properly)

---

## Option B: React Shim + react-dnd

**Libraries:** `react`, `react-dom`, `react-dnd`, `react-dnd-html5-backend`
**Total size:** ~45KB+ min+gzip (React 18 + react-dnd)
**Approach:** Mount a thin React wrapper inside a Solid-managed container

### Why Consider This

- react-dnd was the original DnD solution before the SolidJS migration
- Known working — the exact same DnD logic worked for years in the React version
- React's batched updates don't cause the same mid-event DOM mutation issue

### Integration Pattern

```tsx
// react-dnd-bridge.tsx — React side
import React from 'react';
import { createRoot } from 'react-dom/client';
import { DndProvider, useDrag } from 'react-dnd';
import { HTML5Backend } from 'react-dnd-html5-backend';

interface DragHandleProps {
    nodeId: string;
    onDragStart: () => void;
    onDragEnd: () => void;
    children: React.ReactNode;
}

function DragHandle({ nodeId, onDragStart, onDragEnd, children }: DragHandleProps) {
    const [, dragRef] = useDrag({
        type: 'TILE_NODE',
        item: () => { onDragStart(); return { nodeId }; },
        end: () => onDragEnd(),
    });
    return <div ref={dragRef}>{children}</div>;
}

// Mount function called from SolidJS
export function mountDragHandle(container: HTMLElement, props: Omit<DragHandleProps, 'children'>) {
    const root = createRoot(container);
    root.render(
        <DndProvider backend={HTML5Backend}>
            <DragHandle {...props}>
                {/* Solid renders the actual header content elsewhere */}
            </DragHandle>
        </DndProvider>
    );
    return () => root.unmount();
}
```

```tsx
// In blockframe.tsx — SolidJS side
import { mountDragHandle } from './react-dnd-bridge';

function BlockFrame_Header(props) {
    let wrapperRef: HTMLDivElement;

    onMount(() => {
        if (props.preview) return;
        const cleanup = mountDragHandle(wrapperRef, {
            nodeId: props.nodeModel.nodeId,
            onDragStart: () => props.nodeModel.dragHandlers?.onDragStart(),
            onDragEnd: () => props.nodeModel.dragHandlers?.onDragEnd(),
        });
        onCleanup(cleanup);
    });

    return <div ref={wrapperRef} class="block-frame-default-header" ...>
}
```

### Problems With This Approach

1. **+45KB bundle** for React runtime just for DnD
2. **Two virtual DOMs** running simultaneously — React for drag handles, Solid for everything else
3. **Content rendering** — the header content (icons, title, buttons) is rendered by Solid but needs to be inside the React drag handle wrapper. Requires portal-like tricks.
4. **react-dnd HTML5Backend** uses the same HTML5 DnD API under the hood — if WebView2 is the problem, react-dnd will have the same issue. React's batched updates may help, but the core browser API issue remains.
5. **Maintenance burden** — two frameworks to keep updated, version conflicts, build complexity

---

## Option C: Custom Pointer Events (No Library)

**Size:** 0 bytes additional
**Approach:** Replace HTML5 DnD entirely with pointer events (pointerdown/pointermove/pointerup)

### Why Consider This

- Pointer events work reliably everywhere — no browser DnD API quirks
- No external dependencies
- Full control over the drag visual (custom overlay element follows cursor)
- No ghost image issues (we render our own drag preview)

### Implementation Sketch

```tsx
// In blockframe.tsx header:
onPointerDown={(e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX, startY = e.clientY;
    let dragging = false;

    const onMove = (me: PointerEvent) => {
        if (!dragging && Math.hypot(me.clientX - startX, me.clientY - startY) > 5) {
            dragging = true;
            layoutModel.activeDrag._set(true);
            // Show drag preview overlay at cursor position
        }
        if (dragging) {
            // Update preview position, compute drop target
            updateDragPreview(me.clientX, me.clientY);
            computeDropTarget(me.clientX, me.clientY);
        }
    };

    const onUp = (ue: PointerEvent) => {
        document.removeEventListener('pointermove', onMove);
        document.removeEventListener('pointerup', onUp);
        if (dragging) {
            executeDrop();
            layoutModel.activeDrag._set(false);
        }
    };

    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
}}
```

### Tradeoffs

- **Pro:** No HTML5 DnD API at all — sidesteps WebView2 issues entirely
- **Pro:** Works identically on all platforms
- **Pro:** No external dependencies
- **Con:** Must implement our own drag preview overlay (follow cursor with a div)
- **Con:** Must implement our own hit testing for drop zones
- **Con:** Cross-window drag requires Tauri IPC (pointer events don't cross windows)
- **Con:** ~200-300 lines of new code to replace existing DnD infrastructure

---

## Recommendation: @thisbeyond/solid-dnd (Option D)

**After research, the best option is `@thisbeyond/solid-dnd`** — a SolidJS-native DnD library that uses **pointer events**, not the HTML5 DnD API.

| Criteria | solid-dnd | Pragmatic DnD | react-dnd | Custom pointer |
|----------|-----------|---------------|-----------|----------------|
| Event system | **Pointer events** | HTML5 DnD | HTML5 DnD | Pointer events |
| WebView2 safe | **YES** | NO | NO | YES |
| SolidJS native | **YES** | Manual wiring | NO (React) | Manual |
| Tile rearrangement | **YES** | YES | YES | DIY |
| Bundle size | ~8-10KB | 4.7KB | 45KB+ | 0KB |
| Maintenance | Stable v0.7.5 | Active | Dead (4yr) | N/A |

### Why solid-dnd

1. **Pointer events = WebView2 safe** — never touches HTML5 DnD API
2. **SolidJS native** — `createDraggable`, `createDroppable`, `DragOverlay` primitives
3. **Supports custom drop zones** — not just sortable lists
4. **No framework bridge** — no React, no interop overhead
5. **Stable** — v0.7.5, used in production SolidJS apps

### Why NOT Pragmatic DnD or react-dnd

Both use the HTML5 DnD API under the hood. They will hit the exact same WebView2 canvas-freeze and drag-cancel bugs we've been fighting.

### Fallback: Custom Pointer Events (Option C)

If solid-dnd has edge cases, implement pointer events manually (~200-300 lines). The drop zone infrastructure (overlay nodes, ComputeMove, etc.) already exists — we just need to replace the event source.

### Future: @dnd-kit/dom + SolidJS adapter

dnd-kit v2 uses pointer events and has a community SolidJS adapter (`dnd-kit-solid`). Currently pre-1.0 and not production-ready, but best long-term bet.

---

## Next Steps

1. `npm install @atlaskit/pragmatic-drag-and-drop`
2. Prototype: register one header as `draggable()` and one overlay as `dropTargetForElements()`
3. Verify the drag ghost appears and drops work in WebView2
4. If yes: migrate all DnD code to pragmatic-dnd
5. If no: implement Option C (pointer events)
