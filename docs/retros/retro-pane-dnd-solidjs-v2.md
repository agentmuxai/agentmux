# Retro: Pane Drag-and-Drop Regression (SolidJS Migration) — v2

**Date:** 2026-03-19
**Status:** In Progress
**Branch:** `agenta/fix-pane-dnd-v2`

---

## Background

Pane drag-and-drop (rearranging panes by dragging their title bar headers) has been broken since the SolidJS migration. Multiple fix attempts have been made across several commits and sessions.

---

## Timeline of Fix Attempts

### Attempt 1: `cc1806d` — Gate onDragStart to header only
**Approach:** SolidJS migration had put `draggable=true` on the entire tile-node div. Fixed by gating `onDragStart` to only fire when drag originates from the header (`dragHandleRef`).
**Result:** Partial fix. Worked on some machines but not reliably.

### Attempt 2: `27c9e72` — createEffect on dragHandleRef
**Approach:** Register drag directly on `dragHandleRef` (header element) via `createEffect`, using `addEventListener` imperatively. Removed `draggable`/`onDragStart`/`onDragEnd` from tile-node JSX.
**Result:** Did not work. The `createEffect` ran before `BlockFull` mounted (gated behind `Show when={ready()}` with 50ms delay), so `dragHandleRef.current` was null.

### Attempt 3: `514a2af` — Track nodeModel.ready() in createEffect
**Approach:** Added `nodeModel.ready()` as a reactive dependency in the `createEffect` so it re-runs after the 50ms delay when `BlockFull` renders and the header mounts.
**Result:** Did not work reliably. The `ready()` signal from the layout model (50ms timer) fires before the block's own `ready` state (depends on backend data load via network). The `dragHandleRef.current` is still null when the effect re-runs because the `Show when={localReady()}` gate in block.tsx hasn't opened yet. **Root cause: two different "ready" signals with different timing.**

### Attempt 4 (current session): Option 2 — Blockframe approach
**Approach:** Move drag logic entirely to blockframe.tsx via NodeModel callbacks.
- Remove `dragHandleRef` from NodeModel interface, replace with `dragHandlers: { onDragStart, onDragEnd }`
- DisplayNode sets `nodeModel.dragHandlers = { ... }` synchronously in its body
- blockframe.tsx reads `nodeModel.dragHandlers` and applies as JSX props on the header div
- Eliminates ref timing issue because blockframe IS where the header lives

**Changes made:**
1. `types.ts` — Replaced `dragHandleRef` with `dragHandlers` in NodeModel interface
2. `layoutNodeModels.ts` — Removed `dragHandleRef` from nodeModel creation
3. `TileLayout.tsx` — Replaced old DnD functions + createEffect with direct `nodeModel.dragHandlers` assignment
4. `blockframe.tsx` — Header div reads `dragHandlers` for `draggable`/event handlers

**Sub-attempts within this session:**

| # | Change | Result |
|---|--------|--------|
| 4a | `onDragStart`/`onDragEnd` as JSX props | Not working — no drag initiated |
| 4b | Missing `createEffect` import | Runtime crash: `createEffect is not defined` |
| 4c | Missing `onCleanup` import | Runtime crash: `onCleanup is not defined` |
| 4d | `attr:draggable` instead of `draggable` | Not working |
| 4e | `on:dragstart`/`on:dragend` instead of `onDragStart`/`onDragEnd` | Not working |

---

## Key Diagnostic Finding

Debug logging confirmed:
```
[DnD-debug] dragHandlers: {} preview: false nodeId: 01add974-...
```

- `dragHandlers` is **NOT null** — the object with `onDragStart`/`onDragEnd` functions is set correctly
- The `{}` display is because `console.log` serializes objects with function values as `{}`
- The handlers ARE populated by the time blockframe reads them
- Both `draggable` attribute and event handlers are present in the JSX

**Yet dragging does not initiate.** Something else is preventing it.

---

## Working Reference: Tab Drag-and-Drop

Tab drag works perfectly with this pattern in `tab.tsx:267`:
```tsx
<div
    draggable={true}
    onDragStart={props.onDragStart}
    data-tauri-drag-region="false"
>
```

Key differences from pane drag:
1. `draggable={true}` — static boolean, not a computed expression
2. `onDragStart` — passed as a **prop** (function reference), not read from a mutable object
3. `data-tauri-drag-region="false"` — **explicitly opts out of Tauri window drag**

---

## Root Cause Theories

### Theory 1: `data-tauri-drag-region` Conflict (HIGH probability)
The pane header div does NOT have `data-tauri-drag-region="false"`. Tauri's window drag system may be intercepting the mousedown/dragstart on the header, preventing HTML5 DnD from initiating.

**Evidence:**
- Tab drag works and explicitly sets `data-tauri-drag-region="false"`
- Window drag uses `data-tauri-drag-region` attributes
- Previous fix `cc1806d` mentions macOS `setMovableByWindowBackground` intercepting drags
- The `useWindowDrag.ts` hook manages drag region attributes

**Test:** Add `data-tauri-drag-region="false"` to the block-frame-default-header div.

### Theory 2: SolidJS Computed `draggable` Expression (MEDIUM probability)
Tab drag uses `draggable={true}` (static). Pane drag uses:
```tsx
draggable={dragHandlers != null && !props.nodeModel.isEphemeral() && !props.nodeModel.isMagnified()}
```

In SolidJS, this creates a reactive expression. If SolidJS evaluates it once and the result is a function (accessor) rather than a boolean value, the HTML attribute may be set to a truthy non-"true" value, which browsers may not recognize as enabling drag.

**Test:** Change to `draggable={true}` (hardcoded) and see if drag initiates.

### Theory 3: CSS `user-select: none` or `pointer-events: none` (MEDIUM probability)
A parent element's CSS may be preventing drag initiation. The `disablePointerEvents` signal on NodeModel is used to set `pointer-events: none` during active drags on OTHER panes, but if it's stuck in the wrong state, it could block everything.

**Test:** Inspect the pane header in DevTools, check computed `pointer-events` and `user-select`.

### Theory 4: Drag Event Listeners on Parent Intercepting (LOW probability)
The tile-node div or display-container may have drag-related event listeners that call `e.preventDefault()` or `e.stopPropagation()` before the header's handlers fire.

**Test:** Add `console.log` in the `on:dragstart` handler to see if it even fires.

### Theory 5: `onDragStart` vs `on:dragstart` Event Binding (LOW probability)
SolidJS's delegated events list may or may not include drag events. The tab component uses `onDragStart` (camelCase) and it works. We tried both `onDragStart` and `on:dragstart`.

**But:** The tab's `onDragStart` is a direct function prop. Our pane's handler wraps through `(e) => dragHandlers?.onDragStart(e)`. The wrapper should be equivalent, but the optional chaining `?.` could be suspect if `dragHandlers` is a Proxy or getter.

**Test:** Replace `(e) => dragHandlers?.onDragStart(e)` with a direct inline function that logs and calls the handler.

---

## Recommended Path Forward

### Step 1: Quick Wins (try first)
1. **Add `data-tauri-drag-region="false"`** to the header div — this is the most likely fix based on the tab drag precedent
2. **Hardcode `draggable={true}`** temporarily to eliminate computed expression issues
3. **Add logging inside the onDragStart handler** to confirm whether the event fires at all

### Step 2: If Quick Wins Fail
4. **Revert to createEffect approach** but fix the timing issue properly:
   - Instead of tracking `nodeModel.ready()`, use `onMount` inside `BlockFrame_Header` to register drag listeners
   - The header element IS available in its own `onMount` — no timing issue
   - This is the simplest correct approach since it doesn't fight SolidJS reactivity

### Step 3: Nuclear Option
5. **Adopt the tab drag pattern exactly:**
   - Pass `onDragStart`/`onDragEnd` as explicit props through the component tree
   - Use `draggable={true}` (static)
   - Use `data-tauri-drag-region="false"`
   - Use `onDragStart` (camelCase, not `on:dragstart`)
   - This is proven to work in the same codebase

---

## Files Modified (Current State)

| File | Changes |
|------|---------|
| `frontend/layout/lib/types.ts` | `dragHandleRef` → `dragHandlers` in NodeModel |
| `frontend/layout/lib/layoutNodeModels.ts` | Removed `dragHandleRef` from nodeModel creation |
| `frontend/layout/lib/TileLayout.tsx` | Replaced createEffect with `nodeModel.dragHandlers = {...}` |
| `frontend/app/block/blockframe.tsx` | Header div has drag props from `nodeModel.dragHandlers` |

---

## Research: @neodrag/solid

**Verdict: Wrong tool for this job.**

`@neodrag/solid` is a **pointer-event-based positional drag** library — it moves elements around freely on screen (like dragging a dialog/window). It does NOT implement HTML5 drag-and-drop.

Our pane DnD system requires HTML5 DnD specifically because:
- Uses `dataTransfer.setData()` to pass node IDs between drag source and drop target
- Uses `onDragOver`/`onDrop` on overlay elements for drop zone detection
- Uses `setDragImage()` for custom drag preview
- Supports cross-window drag via `CrossWindowDragMonitor.tsx` (Tauri events)
- Uses `dropEffect`/`effectAllowed` for cursor feedback

`@neodrag/solid` provides none of these — it's `transform: translate()` based movement with pointer events. Using it would require rewriting the entire drop zone system, overlay rendering, and cross-window support.

**Alternative considered: `@thisbeyond/solid-dnd`**
This IS a proper DnD toolkit for SolidJS with sortable lists, droppable zones, etc. However:
- It's a heavyweight abstraction over our already-working drop zone system
- Our `onDragOver`/`onDrop` handlers on overlay nodes already work (the DROP side isn't broken)
- Only the DRAG INITIATION on pane headers is broken
- Adding a new dependency to fix one attribute/event issue is overkill

---

## Lessons Learned

1. **Two different "ready" signals** caused confusion: layout's `nodeModel.ready` (50ms timer) vs block's local `ready` (backend data + viewModel loaded)
2. **SolidJS ref timing** with `createEffect` is fundamentally different from React's `useEffect` — effects run synchronously during the reactive graph update, not after DOM commit
3. **console.log({functions})** shows `{}` — always use explicit checks like `typeof obj.method === 'function'` for debugging
4. **Working code in the same codebase** (tab drag) is the best reference — match its pattern exactly before trying novel approaches
5. **Tauri window drag regions** can silently intercept HTML5 drag events — always check `data-tauri-drag-region` attributes
6. **Don't reach for libraries** when the problem is one missing attribute — `@neodrag/solid` and `solid-dnd` are wrong tools for a simple HTML5 DnD initiation bug

---

## Next Steps (Prioritized)

**Do these in order. Stop as soon as drag works.**

1. Add `data-tauri-drag-region="false"` to the header div in blockframe.tsx
2. Hardcode `draggable={true}` (remove computed expression)
3. Add `console.log("DRAGSTART FIRED")` inside the handler to confirm event fires
4. If event doesn't fire: use `onMount` in `BlockFrame_Header` with `addEventListener("dragstart", ...)` directly on the header ref
5. If event fires but DnD doesn't work: check CSS `pointer-events` / `user-select` on parents
6. If all else fails: match tab.tsx pattern exactly — `draggable={true}` + `onDragStart={fn}` + `data-tauri-drag-region="false"`
