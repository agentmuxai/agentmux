# SPEC — Pane Minimize Refinements

**Date:** 2026-06-24
**Status:** Proposed
**Builds on:** SPEC_PANE_MINIMIZE_AND_TOOLCALL_FAILCOLLAPSE_2026_06_21.md

---

## Overview

Two refinements to the pane minimize button introduced in the prior spec:

1. **Caret direction** — the minimize button's chevron should flip to point upward when the pane is collapsed, signaling "click to expand."
2. **Solo-column slip behavior** — a pane that occupies its own column slot (no vertical sibling) should collapse vertically and slip its header into the adjacent column rather than shrinking horizontally into a thin strip.

---

## Background: current implementation

The minimize button lives in `frontend/app/block/blockframe.tsx` (`OptMinimizeButton`, line 179). It is visible when `numLeafs() > 1` (more than one pane in the workspace) and the pane is not magnified or ephemeral.

Collapse is implemented in `frontend/layout/lib/layoutMinimize.ts` (`minimizeNodeToggle`). It:

1. Finds the node's **parent** in the layout tree.
2. Picks an **adjacent sibling** (right-neighbor preferred, else left).
3. Stores the original `node.size` in `node.minimizedSize` and sets `node.size = headerSizeUnits` (33px + gap in flex-units).
4. Gives the freed units to the sibling.

The critical flaw: `node.size` is the size **along the parent's flex direction**. When `parentNode.flexDirection === Row`, `node.size` is **width** — so minimize makes the pane horizontally narrow (thin strip) instead of vertically short (header bar). This is the "thin pane" bug.

### Layout tree structure for the solo-column case

```
rootNode  (FlexDirection.Row)
├── soloPane  (leaf, size = W₁)   ← minimize button visible because total numLeafs > 1
└── adjacentColumn  (FlexDirection.Column, size = W₂)
    ├── paneB  (leaf)
    └── paneC  (leaf)
```

With current code, minimizing soloPane sets `soloPane.size = headerSizeUnits` where `pixelToSizeRatio` is from the **Row** parent — so soloPane becomes ~38px **wide**, not ~38px tall.

---

## Refinement 1 — Caret direction

### Desired behavior

| Pane state | Chevron | Tooltip |
|---|---|---|
| Expanded (default) | `chevron-down` ▾ | "Minimize" |
| Collapsed | `chevron-up` ▴ | "Restore" |

### Current code status

`OptMinimizeButton` (blockframe.tsx:182) already reads:

```typescript
icon: props.minimized ? "chevron-up" : "chevron-down",
title: props.minimized ? "Restore" : "Minimize",
```

`props.minimized` is passed as a reactive getter from `EndIcons`:

```typescript
const minimized = () => props.nodeModel.isMinimized();
// ...
<OptMinimizeButton minimized={minimized()} ... />
```

`isMinimized()` is a `createMemo` that reads `model.minimizedNodeIds` (a `SignalAtom`). The signal is updated synchronously in `minimizeNodeToggle`. The reactive chain is: signal → memo → prop getter → inner memo → icon string.

**Conclusion:** The caret toggle is already implemented correctly in code on main. If the icon is observed to not flip, verify that `model.minimizedNodeIds._set(...)` is firing (add a console.log in `minimizeNodeToggle` lines 64–72) and that `rebuildMinimizedSet` is called on tree load.

No code change required for Refinement 1 unless a reactivity gap is found.

---

## Refinement 2 — Solo-column slip behavior

### Desired behavior

When a pane's immediate parent in the layout tree is a **Row** node (meaning the pane occupies a horizontal column slot with no vertical neighbors), clicking minimize should:

1. **Collapse vertically** to header-bar height (33px).
2. **Slip** the pane's header to the **top** of the nearest adjacent column.
3. **Release** its column-slot width to the adjacent column, which expands to fill the gap.

On **restore**:
1. Remove the pane from the adjacent column.
2. Re-insert it in the root Row at its original position with its original width.
3. Shrink the adjacent column back to its pre-collapse width.

The pane header appears pinned above the adjacent column's top pane during the minimized state. It takes the full width of that column and only the header height.

### New type field

Extend `LayoutNode` in `frontend/layout/lib/types.ts`:

```typescript
export interface LayoutNode {
    id: string;
    data?: TabLayoutData;
    children?: LayoutNode[];
    flexDirection: FlexDirection;
    size: number;
    /** Present when node is minimized in-column (vertical collapse). */
    minimizedSize?: number;
    /**
     * Present when node was minimized from a solo Row slot and slipped into
     * an adjacent column. Stores restore context so the operation is fully
     * reversible. Never coexists with `minimizedSize`.
     */
    slipMinimize?: {
        /** Node ID of the Column the pane slipped into. */
        targetColumnId: string;
        /** Original Row-slot size (width) before slip. */
        originalRowSize: number;
        /** Original index in the Row's children array. */
        originalRowIndex: number;
    };
}
```

### Detection — when to slip vs when to collapse normally

In `minimizeNodeToggle` (`layoutMinimize.ts`), after finding `parentNode`:

```typescript
const isSoloRowSlot = parentNode.flexDirection === FlexDirection.Row;
```

If `isSoloRowSlot`, run the slip path. Otherwise, run the existing column-collapse path unchanged.

> **Note:** A pane in a Row with a Row sibling (two panes side-by-side sharing width) also hits this branch. That is intentional — any pane whose parent is a Row should slip rather than thin. The "solo" descriptor in the spec title refers to the common UX case; the code predicate is simply parent-is-Row.

### Slip-minimize algorithm

```typescript
if (isSoloRowSlot) {
    // 1. Identify the adjacent column that will absorb the slip.
    //    Pick the same neighbor as the normal path (right preferred).
    const sibling = siblings[nodeIdx + 1] ?? siblings[nodeIdx - 1];
    if (!sibling) return;

    // 2. Record restore context.
    node.slipMinimize = {
        targetColumnId: sibling.id,
        originalRowSize: node.size,
        originalRowIndex: nodeIdx,
    };

    // 3. Give width to sibling; remove node from Row.
    sibling.size += node.size;
    parentNode.children!.splice(nodeIdx, 1);

    // 4. Insert node at top of sibling column.
    //    Get column's pixelToSizeRatio to convert header height to column units.
    const colProps = addlProps[sibling.id];
    const colRatio = colProps?.pixelToSizeRatio ?? pixelToSizeRatio;
    const headerInColUnits = (HeaderHeightPx + gapSizePx) * colRatio;

    if (!sibling.children) {
        // Column has no children yet — shouldn't happen in practice.
        sibling.data = undefined;
        sibling.children = [];
    }
    // Steal header-height from the first child so total column size is preserved.
    const firstChild = sibling.children[0];
    if (firstChild) firstChild.size = Math.max(firstChild.size - headerInColUnits, headerInColUnits);
    node.size = headerInColUnits;
    sibling.children.unshift(node);
}
```

### Slip-restore algorithm

On the second toggle, detect restore via `node.slipMinimize !== undefined`:

```typescript
if (node.slipMinimize !== undefined) {
    const { targetColumnId, originalRowSize, originalRowIndex } = node.slipMinimize;

    // 1. Find the target column by id.
    const targetCol = findNode(model.treeState.rootNode, targetColumnId);
    if (!targetCol?.children) return;

    // 2. Remove node from column; return its size to the next sibling.
    const slipIdx = targetCol.children.findIndex((c) => c.id === nodeId);
    if (slipIdx !== -1) {
        const reclaimUnits = node.size;
        targetCol.children.splice(slipIdx, 1);
        const nextChild = targetCol.children[slipIdx] ?? targetCol.children[slipIdx - 1];
        if (nextChild) nextChild.size += reclaimUnits;
    }

    // 3. Shrink sibling (the target column) back by originalRowSize.
    targetCol.size -= originalRowSize;
    if (targetCol.size < 0) targetCol.size = 0; // safety clamp

    // 4. Re-insert node in the Row at its original index.
    node.size = originalRowSize;
    node.slipMinimize = undefined;
    const rowNode = findParent(model.treeState.rootNode, targetColumnId);
    if (!rowNode?.children) return;
    const clampedIdx = Math.min(originalRowIndex, rowNode.children.length);
    rowNode.children.splice(clampedIdx, 0, node);
}
```

### `minimizedNodeIds` set — both paths

After either branch (normal or slip), the `minimizedNodeIds` set must reflect the node's new minimized state. The existing set-update block at lines 64–72 already reads `node.minimizedSize !== undefined`, so extend it:

```typescript
model.minimizedNodeIds._set((prev) => {
    const next = new Set(prev);
    const isMinimized = node.minimizedSize !== undefined || node.slipMinimize !== undefined;
    if (isMinimized) {
        next.add(nodeId);
    } else {
        next.delete(nodeId);
    }
    return next;
});
```

### `rebuildMinimizedSet` — include slip nodes

In `rebuildMinimizedSet` (layoutMinimize.ts:83), extend the walk to include slip nodes:

```typescript
if (node.minimizedSize !== undefined || node.slipMinimize !== undefined) ids.add(node.id);
```

### Edge cases

| Scenario | Handling |
|---|---|
| Adjacent column has no children | Treat column as sole occupant; skip first-child resize. Node becomes the column's only child with full size. |
| Restore when target column no longer exists (deleted) | Detect `targetCol === undefined`; fall back to reinserting at a heuristic Row position (after last child). Log a warning. |
| Multiple slip-minimized panes into the same column | Each inserts at index 0 sequentially — last one wins top position. Order is preserved per insertion. |
| Restoring when target column's children have been resized | Restore only reclaims `node.size` (header-height in column units) from the next sibling — it does not try to fully restore the column's original internal distribution. |
| Adjacent sibling is itself a leaf (not a column) | The same slip algorithm applies: the sibling is promoted to hold both the slipped header and its own content. An intermediate Column node must be inserted via `addIntermediateNode` before the `children.unshift` call. |

#### Adjacent sibling is a leaf — intermediate column insertion

When `!sibling.children` (the sibling is a leaf, not a branch), use `addIntermediateNode` to wrap it before inserting:

```typescript
if (!sibling.children) {
    // sibling is a leaf; wrap it in a Column so we can prepend the slipped header.
    addIntermediateNode(sibling); // mutates sibling: data → child, adds Column wrapper
}
```

After this, `sibling.children` is defined and the `unshift` proceeds normally.

---

## Files touched

| File | Change |
|---|---|
| `frontend/layout/lib/types.ts` | Add `slipMinimize` field to `LayoutNode` |
| `frontend/layout/lib/layoutMinimize.ts` | Add `isSoloRowSlot` branch in `minimizeNodeToggle`; extend `minimizedNodeIds` update; extend `rebuildMinimizedSet` |
| `frontend/layout/lib/layoutNode.ts` | No change — `addIntermediateNode`, `removeChild`, `findNode`, `findParent` are reused as-is |
| `frontend/app/block/blockframe.tsx` | No change needed — caret toggle already implemented |

---

## Out of scope

- Animated transition (the header sliding from old position to top of adjacent column).
- Keyboard shortcut for minimize.
- Minimized state in tab strip thumbnails.
- Slip behavior for panes in a floating (torn-off) window — floating windows have a single column, so `numLeafs() > 1` is typically false there already.
- Preserving the internal size distribution of the adjacent column on restore.
