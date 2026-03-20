# Spec: Tab Drag-and-Drop Reordering

**Date:** 2026-03-20
**Goal:** Drag tabs horizontally along the tab bar to reorder them within the same window.

---

## Current State

The infrastructure is 90% there but visual feedback is disabled:

| Component | Status | File |
|-----------|--------|------|
| `draggable={true}` on Tab | Working | `tab.tsx:267` |
| `onDragStart` with payload | Working | `tabbar.tsx:41-58` |
| `onDragOver` on drop wrapper | Working (no visual) | `tabbar.tsx:60-65` |
| `onDrop` with midpoint calc + ReorderTab | Working | `tabbar.tsx:67-100` |
| `WorkspaceService.ReorderTab()` backend | Working | `services.ts:200` |
| `isDragging` visual state | **Disabled** (hardcoded `false`) | `tabbar.tsx:114` |
| Insertion line indicators | **Never applied** | `tabbar.scss:100-125` |
| `data-tauri-drag-region="false"` on tabs | Working | `tab.tsx:272` |

### What's Broken

1. **No visual feedback on the dragged tab** — `isDragging={false}` at `tabbar.tsx:114` means the tab never gets the `.tab-dragging` class (opacity 0.4)

2. **No insertion line during drag-over** — `handleDragOver` only calls `e.preventDefault()` but never adds `.tab-insert-left` or `.tab-insert-right` to the wrapper

3. **No cleanup on drag leave/end** — if user drags out of the tab bar, stale insertion indicators would remain (if they were applied)

4. **Possible conflict with `data-tauri-drag-region`** — the tab bar has `{...dragProps}` which sets `data-tauri-drag-region="true"` on Windows/macOS. Individual tabs have `data-tauri-drag-region="false"` which should override. Verify this actually works — if the tab bar's drag region swallows mousedown before the tab's HTML5 DnD fires, tab dragging won't start.

---

## Implementation

### 1. Add drag state signals to DroppableTab

```typescript
function DroppableTab(props: { ... }): JSX.Element {
    let tabWrapRef!: HTMLDivElement;
    const [isDragging, setIsDragging] = createSignal(false);
    const [insertSide, setInsertSide] = createSignal<"left" | "right" | null>(null);
```

### 2. Wire up isDragging

In `handleDragStart`:
```typescript
const handleDragStart = (e: DragEvent) => {
    // ... existing code ...
    setIsDragging(true);
};
```

Add `handleDragEnd` to clear state:
```typescript
const handleDragEnd = () => {
    setIsDragging(false);
};
```

Pass to Tab:
```typescript
<Tab
    isDragging={isDragging()}
    ...
/>
```

Add to wrapper:
```typescript
<div
    ref={tabWrapRef!}
    class={clsx("tab-drop-wrapper", {
        "tab-dragging": isDragging(),
    })}
    onDragEnd={handleDragEnd}
    ...
>
```

### 3. Add insertion line indicators

Update `handleDragOver` to compute side and set class:
```typescript
const handleDragOver = (e: DragEvent) => {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";

    // Don't show indicator on the tab being dragged
    const raw = e.dataTransfer?.types.includes("application/x-tab-reorder");
    if (!raw) return;

    const rect = tabWrapRef.getBoundingClientRect();
    const midX = rect.left + (rect.right - rect.left) / 2;
    setInsertSide(e.clientX < midX ? "left" : "right");
};
```

Add `handleDragLeave`:
```typescript
const handleDragLeave = () => {
    setInsertSide(null);
};
```

Clear in `handleDrop`:
```typescript
const handleDrop = (e: DragEvent) => {
    setInsertSide(null);
    // ... existing drop logic ...
};
```

Apply classes:
```typescript
<div
    ref={tabWrapRef!}
    class={clsx("tab-drop-wrapper", {
        "tab-dragging": isDragging(),
        "tab-insert-left": insertSide() === "left",
        "tab-insert-right": insertSide() === "right",
    })}
    onDragOver={handleDragOver}
    onDragLeave={handleDragLeave}
    onDrop={handleDrop}
    onDragEnd={handleDragEnd}
>
```

### 4. Verify Tauri drag region doesn't conflict

The tab bar spreads `{...dragProps}` which on Windows/macOS sets `data-tauri-drag-region="true"`. Each tab has `data-tauri-drag-region="false"`. According to Tauri docs, child elements with `false` should block the parent's drag region.

**Test:** can you start a drag on a tab on Windows? If not, the fix is to move the drag region to a wrapper div that doesn't contain the tabs (e.g., only the empty space and the `+` button have drag region).

### 5. Handle edge case: dragging over self

When dragging tab A over tab A, don't show insertion indicators:
```typescript
const handleDragOver = (e: DragEvent) => {
    // ... midpoint calc ...
    // Check if this is the dragged tab (via a module-level signal or data attribute)
    if (isDragging()) {
        setInsertSide(null);
        return;
    }
    setInsertSide(e.clientX < midX ? "left" : "right");
};
```

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/tab/tabbar.tsx` | Add signals, wire visual feedback, add dragEnd/dragLeave handlers |
| `frontend/app/tab/tabbar.scss` | No changes needed — CSS already complete |
| `frontend/app/tab/tab.tsx` | No changes needed — already accepts isDragging prop |

---

## Testing

1. Drag a tab left — green insertion line appears on left side of target tab
2. Drag a tab right — green insertion line appears on right side
3. Drop — tab moves to new position, insertion line disappears
4. Dragged tab has reduced opacity (0.4) while dragging
5. Cancel drag (press Esc or release outside tab bar) — everything resets
6. Single tab — drag is prevented (existing code at line 42)
7. Pinned tabs stay in pinned section, regular tabs in regular section
