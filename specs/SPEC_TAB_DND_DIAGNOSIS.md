# Spec: Tab DnD — Root Cause Analysis & Architecture

**Date:** 2026-03-20
**Status:** Diagnosis — not working on Windows

---

## Symptom

Tab drag-and-drop reordering does not work. Either:
- A) Drag never starts (no opacity change, no insertion indicator)
- B) Drag starts visually but drop does not reorder

Distinguishing A vs B requires running in `task dev` and watching for `[fe] tab-drag started` in
the terminal output. Without that log, drag never initiated.

---

## Root Cause Analysis

### Bug 1 — `tab-drop-wrapper` missing `data-tauri-drag-region="false"` (CRITICAL)

**File:** `frontend/app/tab/tabbar.tsx:192-217`

The `.tab-bar` div has `data-tauri-drag-region="true"` (spread from `useWindowDrag` on Windows).
On Windows, Tauri/WebView2 handles this attribute at the OS level — it intercepts `pointerdown`
events **before they reach the JavaScript event loop** for any element inside the drag region that
does not explicitly opt out.

The draggable element registered with pragmatic-dnd is `tab-drop-wrapper`:

```tsx
// tabbar.tsx — DroppableTab return
<div ref={tabWrapRef!} class="tab-drop-wrapper">   // ← NO data-tauri-drag-region="false"
    <Tab ... data-tauri-drag-region="false" />     // ← inner .tab has it, but too late
</div>
```

Pragmatic-dnd attaches a `pointerdown` listener to `tabWrapRef` to start drags. On Windows,
Tauri intercepts `pointerdown` on `.tab-drop-wrapper` at the OS level. The inner `.tab` div has
`data-tauri-drag-region="false"`, which means clicks directly on the tab label/buttons work.
But pragmatic-dnd needs the **wrapper** to receive the event — and the wrapper is unprotected.

**Result:** `onDragStart` never fires → `globalDragTabId` stays null → no visual feedback → no
reorder on drop.

**Fix:** Add `data-tauri-drag-region="false"` to the `tab-drop-wrapper` div.

---

### Bug 2 — Frontend sends combined indices; backend uses section-specific indices (LOGIC BUG)

**Files:** `frontend/app/tab/tabbar.tsx:161`, `agentmuxsrv-rs/src/backend/wcore.rs:483-493`

The frontend builds a combined `allTabIds()` array: `[...pinnedTabIds(), ...regularTabIds()]`
and uses that to compute `tabIndex`:

```typescript
// pinned loop
tabIndex={idx}                           // 0, 1, 2...

// regular loop
tabIndex={idx()}                         // pinnedCount + 0, pinnedCount + 1, ...
```

The backend `reorder_tab` operates on **section-specific** arrays:

```rust
if let Some(pos) = ws.tabids.iter().position(|id| id == tab_id) {
    ws.tabids.remove(pos);
    let insert_at = new_index.min(ws.tabids.len());  // new_index into tabids[], not allTabs[]
    ws.tabids.insert(insert_at, tab_id.to_string());
```

**Example scenario** — 2 pinned tabs, 3 regular tabs [A, B, C]:
- Drag B (combined index 3, section index 1) to right of C (combined index 4, section index 2)
- Frontend: `sourceIndex=3`, `targetIndex=4`, `side="right"` → `rawIndex=5` → `newIndex=4`
- Backend: removes B from tabids (section), `insert_at = min(4, 2) = 2` → clamps to end
- Result: lucky — works. But:

**Broken scenario** — drag B (section index 1) to left of A (combined index 2, section index 0):
- Frontend: `sourceIndex=3`, `targetIndex=2`, `side="left"` → `rawIndex=2` → `newIndex=2`
- Backend: removes B (was at section pos 1), `insert_at = min(2, 2) = 2` → appends to end ❌
- Expected: B should move to front of regular section (index 0)

The `computeInsertIndex` function is mathematically correct for a single combined array, but the
backend ignores the pinned offset entirely. The index math is applied to the wrong domain.

**Fix:** Frontend must pass section-relative indices. Strip `pinnedTabIds().length` from regular
tab indices before calling `ReorderTab`.

---

### Bug 3 — `tabIndex` in `getInitialData` may be stale (MINOR)

**File:** `frontend/app/tab/tabbar.tsx:100,107`

```typescript
const cleanupDraggable = draggable({
    ...
    getInitialData: () => ({
        tabIndex: props.tabIndex,   // ← captured at drag-start time
```

`props.tabIndex` is reactive in SolidJS, so reading it in the callback is correct. However, the
`For` loop in the pinned section captures `idx` as a non-reactive value:

```typescript
// pinned loop — idx is a plain number, not reactive:
{(tabId, i) => {
    const idx = i();              // ← evaluated once at render, NOT reactive
    return <DroppableTab tabIndex={idx} .../>
```

If a tab is added/removed from pinned while a drag is in flight, the rendered DroppableTab has
a stale `tabIndex` prop. This is low-risk in practice (rare mid-drag mutations), but should
be made reactive.

---

### Bug 4 — `nearestHint` is module-level; breaks with multiple windows (FUTURE RISK)

**File:** `frontend/app/tab/tabbar.tsx:39`

```typescript
const [nearestHint, setNearestHint] = createSignal<...>(null);
const tabWrapperRefs = new Map<string, HTMLDivElement>();
let globalDragTabId: string | null = null;
```

These are module-level singletons. When AgentMux supports multiple windows, each window has its
own `TabBar` mounted in a separate webview — this is fine. But if multiple `TabBar` instances
ever exist in the same webview (different workspaces rendered simultaneously), they'd corrupt
each other's drag state.

Currently not a bug, but the shared state should be encapsulated in a class or context.

---

### Bug 5 — `monitorForElements` registered in TabBar `onMount`, but it references `allTabIds()` closure

**File:** `frontend/app/tab/tabbar.tsx:305-341`

The `onDrop` callback in `monitorForElements` calls `allTabIds()` to resolve `targetIdx` and
`sourceIdx`. `allTabIds` is a reactive derived signal — reading it inside a non-reactive callback
(pragmatic-dnd event) is fine, it will return the current value. This is not a bug, but it's
subtle and worth documenting.

---

### Bug 6 — `NewTabDropZone` uses native DnD (`onDragOver`) not pragmatic-dnd

**File:** `frontend/app/tab/tabbar.tsx:223-239`

The `NewTabDropZone` component listens to `onDragOver` (native HTML5 DnD), while all tabs use
pragmatic-dnd. These two systems don't interoperate — a tab being dragged via pragmatic-dnd will
not trigger `onDragOver` on the drop zone.

This component currently does nothing on drop anyway (no `onDrop` handler). It's dead code for
tab reordering, but it will cause confusion when we add "drag tab to create new tab" behavior.

---

## Fix Plan (Ordered by Impact)

### Fix 1 — Add `data-tauri-drag-region="false"` to `tab-drop-wrapper` (30 min)

```tsx
// tabbar.tsx — DroppableTab return
<div
    ref={tabWrapRef!}
    data-tauri-drag-region="false"      // ← ADD THIS
    class={clsx("tab-drop-wrapper", { ... })}
>
```

This alone may be sufficient to unblock drag-start on Windows. **Test first** before applying
any other fix.

### Fix 2 — Pass section-relative indices to `ReorderTab` (1 hr)

Two options:

**Option A — Strip pinned offset in frontend (simpler)**

```typescript
// In DroppableTab onDrop:
const sourceTabId = source.data.tabId as string;
const isPinnedSource = source.data.isPinned as boolean;
const isPinnedTarget = props.isPinned;

// Only reorder within the same section
if (isPinnedSource !== isPinnedTarget) return; // cross-section drag — ignore for now

const sectionSourceIndex = isPinnedSource
    ? source.data.tabIndex as number
    : (source.data.tabIndex as number) - pinnedOffset;   // strip pinned count

const sectionTargetIndex = isPinnedTarget
    ? props.tabIndex
    : props.tabIndex - pinnedOffset;

const newIndex = computeInsertIndex(sectionSourceIndex, sectionTargetIndex, side);
await WorkspaceService.ReorderTab(props.workspaceId, draggedTabId, newIndex);
```

The `pinnedOffset` must be passed to `DroppableTab` as a prop (it's available in `TabBar`).

**Option B — Change backend to accept combined indices (bigger change)**

Refactor `reorder_tab` in `wcore.rs` to:
1. Build combined `[pinnedtabids..., tabids...]`
2. Remove source from combined
3. Insert at `new_index` in combined
4. Split back into pinned/regular based on original pinned count

This is cleaner but requires a backend change and tests. Probably the right long-term answer.

**Recommendation:** Ship Option A now, migrate to Option B when implementing cross-section drag.

### Fix 3 — Make pinned `tabIndex` reactive (30 min)

```typescript
// Change in pinned For loop:
{(tabId, i) => {
    // idx was: const idx = i();  ← stale value
    const isActive = () => tabId === activeTabId();
    const isBeforeActive = () => i() === activeIndex() - 1;  // already reactive
    return (
        <DroppableTab
            tabIndex={i()}      // ← read i() reactively each render, not once
```

---

## Architecture: Should We Split tabbar.tsx?

Yes. The file currently has four distinct concerns in one 410-line file:

```
tabbar.tsx
├── DnD state (module-level)        → tabbar-dnd-state.ts
│   ├── globalDragTabId
│   ├── nearestHint signal
│   ├── tabWrapperRefs map
│   ├── computeNearestTab()
│   └── computeInsertIndex()
├── DroppableTab component          → droppable-tab.tsx
│   ├── draggable() registration
│   ├── dropTargetForElements() registration
│   └── insert indicator rendering
├── NewTabDropZone component        → new-tab-drop-zone.tsx
└── TabBar component                → tabbar.tsx (keeps this)
    ├── monitorForElements()
    ├── handleSelect/Close/PinChange
    └── JSX layout
```

**Benefits of splitting:**
- Each file is focused and testable
- `tabbar-dnd-state.ts` can be a class (fixes Bug 4 — encapsulates shared state)
- `DroppableTab` can be developed/debugged independently
- Sets up clean extension points for cross-window drag (Bug 4 architecture)

**When to split:** After Bug 1 and Bug 2 are confirmed fixed. Splitting while debugging adds noise.

---

## Immediate Action Plan

1. **Apply Fix 1** (add `data-tauri-drag-region="false"` to wrapper)
2. **Run dev mode** (`task dev`) and drag a tab — confirm `[fe] tab-drag started` appears in terminal
3. **If drag starts:** test reorder. If wrong positions, apply Fix 2
4. **If drag still doesn't start:** the blocker is deeper (Tauri version, WebView2 behavior)
   - Try moving `draggable()` to the inner `.tab` element instead of the wrapper
   - Or use a `dragHandle` pointing to the inner tab
5. **Once correct behavior confirmed:** apply Fix 3 and split the file

---

## How to See Logs During Testing

Drag logs only appear in `task dev` terminal output (NOT in the portable build, NOT in DevTools):

```
INFO agentmux_lib::commands::backend: [fe] tab-drag started tabId=xxx module=dnd
INFO agentmux_lib::commands::backend: [fe] tab-reorder drop tabId=xxx newIndex=N module=dnd
```

Run `task dev` and drag tabs. If "tab-drag started" never appears → Bug 1 (drag not initiating).
If it appears but "tab-reorder drop" shows a wrong `newIndex` → Bug 2 (index math).
