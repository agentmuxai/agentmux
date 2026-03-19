# Pane DnD Fix: Pragmatic DnD vs React-DnD Bridge

**Date:** 2026-03-19
**Context:** HTML5 DnD worked in the old React codebase via `react-dnd` + `react-dnd-html5-backend`. After the SolidJS migration, pane DnD broke. Multiple attempts to fix it with native HTML5 DnD attributes in SolidJS have failed because SolidJS's synchronous reactive updates mutate the DOM during `dragstart`, causing WebView2 to cancel the drag.

**Key fact:** react-dnd with HTML5Backend DID work in this exact Tauri WebView2 environment. The problem is not "HTML5 DnD is broken in WebView2" — it's "raw HTML5 DnD without React's batched updates breaks in WebView2 because SolidJS mutates the DOM synchronously during drag events."

---

## How the Old React Code Worked

From `git show 0adce7c -- frontend/layout/lib/TileLayout.tsx`:

### Drag Source (DisplayNode)
```tsx
import { useDrag, useDragLayer, useDrop } from "react-dnd";

// In DisplayNode component:
const [{ isDragging }, drag, dragPreview] = useDrag(() => ({
    type: tileItemType,
    canDrag: () => !(isEphemeral || isMagnified),
    item: () => node,
    collect: (monitor) => ({
        isDragging: monitor.isDragging(),
    }),
}), [node, addlProps, isEphemeral, isMagnified]);

// Register drag handle
useEffect(() => {
    drag(nodeModel.dragHandleRef);
}, [drag, nodeModel.dragHandleRef.current]);
```

### Drop Target (OverlayNode)
```tsx
const [, drop] = useDrop(() => ({
    accept: tileItemType,
    canDrop: (_, monitor) => {
        const dragItem = monitor.getItem<LayoutNode>();
        return monitor.isOver({ shallow: true }) && dragItem.id !== node.id;
    },
    drop: (_, monitor) => {
        if (!monitor.didDrop()) layoutModel.onDrop();
    },
    hover: throttle(50, (_, monitor) => {
        if (monitor.isOver({ shallow: true }) && monitor.canDrop()) {
            const dragItem = monitor.getItem<LayoutNode>();
            const offset = monitor.getClientOffset();
            // ... compute drop direction, dispatch ComputeMove
        }
    }),
}), [node, additionalProps]);
```

### Drag Layer (TileLayoutComponent)
```tsx
const { activeDrag, dragClientOffset } = useDragLayer((monitor) => ({
    activeDrag: monitor.isDragging(),
    dragClientOffset: monitor.getClientOffset(),
    dragItemType: monitor.getItemType(),
}));

useEffect(() => {
    setActiveDrag(activeDrag && dragItemType == tileItemType);
}, [activeDrag, dragItemType]);
```

### Why React-DnD Worked
1. **React batches state updates** — `isDragging`, `activeDrag` signals don't flush to DOM until after the event handler returns
2. **react-dnd manages the HTML5 DnD lifecycle internally** — it calls `e.preventDefault()` at the right times, sets `dataTransfer` correctly, handles the dragover/drop coordination
3. **`useDragLayer` provides `isDragging`** — no manual signal management needed, React re-renders on next frame after drag starts (not during dragstart)

---

## Option 1: Pragmatic Drag and Drop

**Package:** `@atlaskit/pragmatic-drag-and-drop` (4.7KB core)
**Maintained by:** Atlassian (used in Jira, Confluence, Trello)
**DnD mechanism:** Wraps the HTML5 DnD API — but manages the lifecycle properly

### API Pattern

```tsx
import { draggable, dropTargetForElements, monitorForElements }
    from '@atlaskit/pragmatic-drag-and-drop/element/adapter';

// DRAG SOURCE — in blockframe.tsx onMount:
const cleanup = draggable({
    element: headerRef,                    // the DOM element
    getInitialData: () => ({               // data attached to drag
        nodeId: props.nodeModel.nodeId,
        type: 'tile-node',
    }),
    onGenerateDragPreview: ({ nativeSetDragImage }) => {
        // Custom drag ghost image
        nativeSetDragImage(previewImg, offsetX, offsetY);
    },
    onDragStart: () => {
        // SAFE to set reactive state here — fires AFTER browser commits drag
        layoutModel.activeDrag._set(true);
        setIsDragging(true);
    },
    onDrop: () => {
        layoutModel.activeDrag._set(false);
        setIsDragging(false);
    },
});
onCleanup(cleanup);

// DROP TARGET — in OverlayNode onMount:
const cleanup = dropTargetForElements({
    element: overlayRef,
    canDrop: ({ source }) => source.data.type === 'tile-node',
    getData: () => ({ nodeId: props.node.id }),
    onDragEnter: ({ source }) => {
        // Compute drop direction
    },
    onDrag: ({ source, self }) => {
        // Throttled (~60fps via rAF) — update drop preview
    },
    onDragLeave: () => {
        layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
    },
    onDrop: ({ source }) => {
        layoutModel.onDrop();
    },
});
onCleanup(cleanup);

// MONITOR — in TileLayoutComponent onMount (replaces useDragLayer):
const cleanup = monitorForElements({
    onDragStart: ({ source }) => {
        if (source.data.type === 'tile-node') {
            globalDragNodeId = source.data.nodeId;
        }
    },
    onDrop: () => {
        globalDragNodeId = null;
    },
});
onCleanup(cleanup);
```

### Why Pragmatic DnD Should Work

1. **Callbacks fire at the RIGHT time** — `onDragStart` fires after browser commits the drag, not during `dragstart` event. DOM mutations are safe.
2. **Manages `e.preventDefault()`** — calls it on dragover internally, so drop zones accept drops
3. **`onDrag` is rAF-throttled** — ~60fps updates for drop position tracking
4. **Framework agnostic** — vanilla JS, attaches to DOM elements via `onMount`, no React needed
5. **Handles drag preview** — `onGenerateDragPreview` provides `nativeSetDragImage`
6. **Tiny** — 4.7KB core, no optional Atlassian UI deps needed

### Integration Plan

| File | Changes |
|------|---------|
| `package.json` | `npm install @atlaskit/pragmatic-drag-and-drop` |
| `blockframe.tsx` | `onMount` → `draggable({ element: headerRef, ... })` |
| `TileLayout.tsx` (OverlayNode) | `onMount` → `dropTargetForElements({ element: overlayRef, ... })` |
| `TileLayout.tsx` (Component) | `onMount` → `monitorForElements({ ... })` — replaces `useDragLayer` |
| `TileLayout.tsx` (DisplayNode) | Remove `nodeModel.dragHandlers` assignment, remove old HTML5 DnD code |
| `types.ts` | Remove `dragHandlers` from NodeModel (replace with pragmatic-dnd in onMount) |
| `layoutNodeModels.ts` | No changes needed |
| `CrossWindowDragMonitor.tsx` | Use `monitorForElements` `onDrop` instead of document `dragend` listener |

### Estimated effort: ~2-3 hours

### Risks
- Pragmatic-dnd uses HTML5 DnD under the hood. If the issue is truly WebView2 (not SolidJS reactivity), this won't help. But since react-dnd (also HTML5 DnD) worked before, this is unlikely.
- Cross-window drag needs testing — pragmatic-dnd may or may not support it natively

---

## Option 2: React-DnD Bridge

**Packages:** `react` (44KB), `react-dom` (130KB), `react-dnd` (14KB), `react-dnd-html5-backend` (10KB)
**Total additional:** ~50KB min+gzip (React runtime is the bulk)
**Approach:** Mount a thin React component tree inside Solid-managed container divs

### Architecture

```
SolidJS App
├── TileLayout (Solid)
│   ├── DisplayNode (Solid)
│   │   └── BlockFrame_Header (Solid)
│   │       └── [React DnD DragHandle mounted here via createRoot]
│   └── OverlayNode (Solid)
│       └── [React DnD DropTarget mounted here via createRoot]
└── [React DndProvider wrapping all drag/drop React islands]
```

### Implementation

**react-dnd-bridge.tsx** (React side):
```tsx
import React, { useEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { DndProvider, useDrag, useDrop, useDragLayer } from 'react-dnd';
import { HTML5Backend } from 'react-dnd-html5-backend';

const tileItemType = 'TILE_NODE';

// Shared DnD context — single React tree wraps all islands
let sharedRoot: ReturnType<typeof createRoot> | null = null;
let sharedContainer: HTMLDivElement | null = null;

export function initDndProvider() {
    sharedContainer = document.createElement('div');
    sharedContainer.id = 'react-dnd-provider';
    sharedContainer.style.display = 'contents'; // no layout impact
    document.body.appendChild(sharedContainer);
    sharedRoot = createRoot(sharedContainer);
    sharedRoot.render(
        <DndProvider backend={HTML5Backend}>
            <DndIslandManager />
        </DndProvider>
    );
}

// Problem: react-dnd requires all useDrag/useDrop to be under the same DndProvider.
// With Solid rendering the actual UI, we'd need to portal React drag handles INTO
// Solid-rendered DOM, which requires a custom bridge for each drag source/drop target.
```

### The DndProvider Problem

react-dnd requires ALL drag sources and drop targets to share a single `<DndProvider>`. This means:

1. **Single React tree** — can't have independent React islands for each pane header
2. **React must render the drag handles** — but Solid renders the headers, creating a DOM ownership conflict
3. **State synchronization** — react-dnd's `isDragging`/`monitor` state lives in React context, but our layout state lives in SolidJS signals

### Workaround: Invisible React Overlay

Instead of mounting React inside Solid components, mount a single invisible React tree that overlays the entire tile layout. React renders invisible drag handles positioned exactly over each pane header, and invisible drop targets over each overlay node.

```tsx
// React component that mirrors Solid's layout
function DndOverlay({ nodes, onDrop, onDragStart, onDragEnd }) {
    return (
        <DndProvider backend={HTML5Backend}>
            {nodes.map(node => (
                <DragHandle key={node.id} node={node} ... />
                <DropTarget key={node.id} node={node} ... />
            ))}
        </DndProvider>
    );
}
```

**Problems with this approach:**
- Must keep React's invisible overlay perfectly synchronized with Solid's layout
- Position changes from resize/animation must be mirrored
- Two independent render trees managing overlapping DOM = race conditions
- Debugging nightmare

### Integration Complexity

| File | Changes |
|------|---------|
| `package.json` | `npm install react react-dom react-dnd react-dnd-html5-backend` |
| NEW `react-dnd-bridge.tsx` | React DnD provider + drag/drop wrappers |
| NEW `react-dnd-types.ts` | Shared types between React and Solid |
| `blockframe.tsx` | Mount React drag handle inside header via `onMount` + `createRoot` |
| `TileLayout.tsx` | Mount React drop targets inside overlay nodes |
| `wave.ts` or `index.tsx` | Initialize DndProvider at app startup |
| `vite.config.ts` | Add React to build config, JSX pragma config for dual React+Solid |
| `tsconfig.json` | Handle dual JSX transform (React for bridge files, Solid for everything else) |

### Estimated effort: ~6-10 hours

### Risks
- **Dual JSX problem** — Vite needs to compile some files with React JSX and others with Solid JSX. Requires careful config (file extensions, pragma comments, or separate tsconfig).
- **Bundle size** — +50KB min+gzip for React runtime
- **Maintenance** — two frameworks, two mental models, version conflicts
- **Synchronization** — keeping React overlay positions in sync with Solid layout
- **Proven BUT complex** — the DnD logic is proven, but the bridge architecture is novel and fragile

---

## Comparison

| Criteria | Pragmatic DnD | React-DnD Bridge |
|----------|--------------|-------------------|
| Bundle size | +4.7KB | +50KB |
| Framework | Vanilla JS | React (full runtime) |
| Integration effort | ~2-3 hours | ~6-10 hours |
| Dual JSX config | No | Yes (complex) |
| Maintenance burden | Low | High |
| API similarity to old code | Different (imperative) | Same (useDrag/useDrop) |
| Proven in WebView2 | Not yet tested | Yes (old code worked) |
| Cross-window drag | Needs investigation | Known working (old code) |
| Risk level | Low-Medium | Medium-High |

---

## Recommendation

**Try Pragmatic DnD first (Option 1).** Reasons:

1. **10x simpler integration** — vanilla JS, no dual-framework complexity
2. **10x smaller bundle** — 4.7KB vs 50KB
3. **Solves the root cause** — callbacks fire after browser commits drag, so SolidJS reactive updates don't interfere
4. **Clean migration path** — each drag source/drop target is independent, can migrate incrementally
5. **Same underlying API** — pragmatic-dnd wraps HTML5 DnD just like react-dnd, so if react-dnd worked, pragmatic-dnd should too

**Fall back to React-DnD Bridge (Option 2) ONLY if:**
- Pragmatic-dnd fails in WebView2 (unlikely given react-dnd worked)
- Cross-window drag can't be implemented with pragmatic-dnd
- Some other showstopper discovered during prototyping

**Quick validation test (30 min):**
1. `npm install @atlaskit/pragmatic-drag-and-drop`
2. In one pane header: `onMount(() => draggable({ element: headerRef, onDragStart: () => console.log("DRAG STARTED") }))`
3. If we see "DRAG STARTED" in logs and the ghost image appears → pragmatic-dnd works → proceed with full migration
4. If drag still dies → HTML5 DnD is truly broken in our WebView2 config → switch to React bridge or pointer events

---

## Next Steps

1. Install pragmatic-dnd
2. Quick validation test on one header
3. If works: full migration (~2-3 hours)
4. If fails: evaluate React bridge vs custom pointer events
