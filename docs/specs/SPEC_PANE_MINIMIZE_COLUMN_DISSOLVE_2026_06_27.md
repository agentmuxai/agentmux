# SPEC — Pane Minimize: Column Dissolve on Full-Column Collapse

**Date:** 2026-06-27
**Status:** Proposed
**Builds on:** SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md

---

## Problem

When all panes in a column are individually minimized, the column does not dissolve —
it stays in the layout as a narrow strip of stacked header bars. The user's intent when
collapsing every pane in a column is to reclaim that column's width for adjacent content.

### Current behavior

```
root (Row)
├── colA (Column, width=400px)        ← colA stays, just 2 header bars tall
│   ├── pane1 (minimized, 33px tall)
│   └── pane2 (minimized, 33px tall)
└── colB (Column, width=600px)
    └── pane3 (normal)
```

### Desired behavior

When pane2 is minimized (making colA fully collapsed), colA dissolves:
its headers slip to the top of colB, and colA's 400 px of width is given to colB.

```
root (Row, _slipAnchor)
└── colB (Column, width=1000px)
    ├── pane1 (minimized header, 33px)
    ├── pane2 (minimized header, 33px)
    └── pane3 (normal, fills remainder)
```

### Cascading

This keeps going: if the user then minimizes pane3, colB becomes fully collapsed and
(if there is a further adjacent column) dissolves in turn. When there is only one
column left, all panes inside it are in the normal minimized state — no further
dissolve is possible (no adjacent column to accept them).

---

## Definitions

| Term | Meaning |
|---|---|
| **Full collapse** | Every leaf child of a Column node has `minimizedSize !== undefined` |
| **Column dissolve** | The full-collapse action that moves a Column's minimized headers into an adjacent column and removes the Column from its Row |
| **columnDissolve** | New field on `LayoutNode` that stores restore context for a dissolved column |

---

## Data model change

Add to `LayoutNode` in `frontend/layout/lib/types.ts`:

```typescript
/**
 * Present when this Column node was dissolved because all its children became
 * minimized. The column was removed from the root Row and re-inserted as a
 * child of `targetColumnId`. Stores restore context.
 *
 * Invariants:
 *   - Never coexists with `minimizedSize` or `slipMinimize` (those are leaf-only).
 *   - Only set on branch (Column) nodes.
 *   - The column's children retain their individual `minimizedSize` values.
 */
columnDissolve?: {
    /** ID of the Column this branch was inserted into. */
    targetColumnId: string;
    /** Original width (Row-slot size) before dissolve. */
    originalRowSize: number;
    /** Original index in the root Row's children array. */
    originalRowIndex: number;
};
```

The existing `_slipAnchor` field on the root Row continues to serve the same role:
preventing `balanceNode` from hoisting when the Row has exactly one child.

---

## Trigger condition

Inside `minimizeNodeToggle`, after the normal Column-parent collapse is applied to
the node, check:

```typescript
const parentNode = findParent(model.treeState.rootNode, nodeId);
if (parentNode?.flexDirection === FlexDirection.Column) {
    // Just collapsed nodeId. Check if the column is now fully minimized.
    const allMinimized = parentNode.children!.every(
        (c) => c.minimizedSize !== undefined || c.slipMinimize !== undefined
    );
    if (allMinimized) {
        _dissolveColumn(model, parentNode, addlProps, gapSizePx);
    }
}
```

The dissolve check runs **after** the normal collapse so that `node.minimizedSize` is
already set when we evaluate `allMinimized`.

---

## Dissolve algorithm (`_dissolveColumn`)

```
function _dissolveColumn(model, colNode, addlProps, gapSizePx):

1.  Find colNode's parent (must be a Row — the root Row or an intermediate Row).
    If no Row parent, bail (column is the root — no dissolve possible).

2.  Find the adjacent sibling in the Row (right-neighbor preferred, else left).
    If none exists, bail (only child of Row — nothing to dissolve into).

3.  Compute total header height for colNode in the TARGET column's flex-unit space:
        headerCount  = colNode.children.length
        totalPx      = headerCount × (HeaderHeightPx + gapSizePx)
        colRatio     = target's pixelToSizeRatio (from addlProps)
        headerUnits  = totalPx × colRatio
    If colRatio is unavailable, bail (same guard as _slipMinimize).

4.  Ensure target has children (convert leaf to Column via addIntermediateNode if
    needed — same as _slipMinimize's targetWasLeaf path).

5.  Steal space from target's first child so the target's total size is unchanged:
        firstChild.size = max(firstChild.size - headerUnits, headerInColUnits_min)
    where headerInColUnits_min = (HeaderHeightPx + gapSizePx) × colRatio.

6.  Record restore context on colNode:
        colNode.columnDissolve = {
            targetColumnId: target.id,
            originalRowSize: colNode.size,
            originalRowIndex: nodeIdx,
        }

7.  Give colNode's Row-slot width to the sibling:
        target.size += colNode.size

8.  Remove colNode from the Row:
        rowNode.children.splice(nodeIdx, 1)

9.  Set _slipAnchor on rowNode (prevents balanceNode hoist):
        rowNode._slipAnchor = true

10. Insert colNode at the TOP of target's children:
        colNode.size = headerUnits
        target.children.unshift(colNode)

11. Call _finishToggle for each child of colNode:
    No — individual children are already in minimizedNodeIds.
    No additional _finishToggle call needed; the dissolved column is not itself
    a "minimized node" in the sense of minimizedNodeIds (it's a branch, not a leaf).
    The column's collapse is observable through columnDissolve being set.
```

---

## Restore algorithm

### Trigger

Restoring any individual pane that lives inside a dissolved column should first
undissolve the column, then apply the normal pane restore.

In `minimizeNodeToggle`, at the top of the function, before any other path:

```typescript
// Check if node is inside a dissolved column.
const parent = findParent(model.treeState.rootNode, nodeId);
if (parent?.columnDissolve !== undefined) {
    _undissolveColumn(model, parent, addlProps);
    // Fall through to normal restore for nodeId.
}
```

### Undissolve algorithm (`_undissolveColumn`)

```
function _undissolveColumn(model, colNode, addlProps):

1.  Read colNode.columnDissolve:
        { targetColumnId, originalRowSize, originalRowIndex }

2.  Find targetCol by id. If not found, log warning and clear columnDissolve; return.

3.  Remove colNode from targetCol.children:
        idx = targetCol.children.findIndex(c => c.id === colNode.id)
        reclaimUnits = colNode.size
        targetCol.children.splice(idx, 1)
        neighbor = targetCol.children[idx] ?? targetCol.children[idx-1]
        if (neighbor) neighbor.size += reclaimUnits

4.  Shrink targetCol back by originalRowSize:
        targetCol.size = max(targetCol.size - originalRowSize, 1)

5.  Find the Row that owns targetCol (same as findParent(root, targetColumnId)).
    If found and it has children:
        colNode.size = originalRowSize
        colNode.columnDissolve = undefined
        rowNode._slipAnchor = undefined
        clampedIdx = min(originalRowIndex, rowNode.children.length)
        rowNode.children.splice(clampedIdx, 0, colNode)

6.  After undissolve, colNode's children all retain their minimizedSize — the column
    is back in the layout but still fully collapsed (all panes are header-height bars).
    The user can then individually restore each pane.
```

---

## `rebuildMinimizedSet` — include dissolved columns

`rebuildMinimizedSet` currently walks nodes and adds IDs where `minimizedSize !== undefined || slipMinimize !== undefined`. No change needed for the column case: the branch node itself is not added to `minimizedNodeIds` (it's not a leaf). Individual leaf children still have their `minimizedSize` and will be included.

However, `rebuildMinimizedSet` must also restore `_slipAnchor` on the parent Row
when it encounters a branch node with `columnDissolve` set:

```typescript
function walk(node: LayoutNode) {
    if (node.minimizedSize !== undefined || node.slipMinimize !== undefined) ids.add(node.id);
    if (node.columnDissolve !== undefined) {
        // The Row that owns the target column must have _slipAnchor re-applied.
        const targetCol = findNode(root, node.columnDissolve.targetColumnId);
        const targetRow = findParent(root, node.columnDissolve.targetColumnId);
        if (targetRow) targetRow._slipAnchor = true;
    }
    node.children?.forEach(walk);
}
```

---

## Edge cases

| Scenario | Handling |
|---|---|
| Only one Column child in the Row (no sibling to accept the dissolve) | Bail silently — no dissolve. The column stays as stacked header bars. The minimize action still completes normally for the individual pane. |
| Multiple columns dissolve into the same target | Each inserts at target's `children[0]` in sequence. Last dissolved column ends up at the visual top. Order of dissolve = insertion order. |
| Target column is itself fully minimized (all its leaves have `minimizedSize`) | Dissolve proceeds normally — the dissolved branch is still prepended to the target's children. The target column's leaves remain minimized. No cascade of the target is triggered (the target itself was not just minimized). |
| Target column has `columnDissolve` set (it's already dissolved into a further column) | This is a degenerate case — the "target" is currently a child of another column. Dissolve bails: do not dissolve into a node that is itself dissolved. Log a warning. |
| Restore when `targetColumnId` no longer exists | Clear `columnDissolve`, log warning, return. The pane will be stuck as minimized within a column that has no restore context. The user can still restore individual panes manually. |
| Column with a single pane becomes fully minimized | Same path — `allMinimized` fires after that one pane collapses. The column dissolves as normal. |
| User restores pane inside dissolved column before undissolve | `_undissolveColumn` runs first, column is re-inserted into the Row, then normal restore proceeds. The other panes in the column remain minimized. |

---

## Visual result — step by step

**Start:** 3 columns, A (2 panes), B (1 pane), C (1 pane).

```
root (Row)
├── colA (Column)  ←→ pane1, pane2
├── colB (Column)  ←→ pane3
└── colC (Column)  ←→ pane4
```

1. Minimize pane1 → normal column collapse. pane1 = header, pane2 grows.
2. Minimize pane2 → normal collapse, then `allMinimized` fires. colA dissolves into colB.
   - Layout: `root(Row)[colB(Column)[colA(headers), pane3], colC]`
   - Visually: colB shows pane1 header, pane2 header, pane3. colC unchanged.
3. Minimize pane3 → normal column collapse. pane3 = header. colB now fully minimized → dissolve into colC.
   - Layout: `root(Row)[colC(Column)[colB(Column[colA(headers), pane3-header]), pane4]]`
   - Visually: colC shows pane1 header, pane2 header, pane3 header, pane4.
4. Result: one visible column (colC) with all minimized headers stacked above pane4. ✓

**Restore pane2** (from step 3 state):
1. pane2 is inside colA, which has `columnDissolve` (nested inside colB, which is inside colC).
   - First: undissolve colB (re-inserts into root Row at its original index; colA is still inside colB as a dissolved branch).
   - Then: detect pane2's parent (colA) has `columnDissolve`.
   - Undissolve colA (re-inserts colA into the Row or wherever colB was? — actually colA is inside colB.children so it's removed from colB and re-inserted into the Row per colA's originalRowIndex).
   - Then: normal pane2 restore within colA.

---

## Interaction with balanceNode

`balanceNode` currently respects `_slipAnchor`. No additional changes needed — the
dissolve sets `_slipAnchor` on the parent Row, same as the existing slip path.

---

## Files touched

| File | Change |
|---|---|
| `frontend/layout/lib/types.ts` | Add `columnDissolve` field to `LayoutNode` |
| `frontend/layout/lib/layoutMinimize.ts` | Add `_dissolveColumn`, `_undissolveColumn`; trigger dissolve check after normal column collapse; check for dissolved parent at top of `minimizeNodeToggle`; extend `rebuildMinimizedSet` to restore `_slipAnchor` on dissolved column's owner |
| `frontend/layout/lib/layoutNode.ts` | No change — `findNode`, `findParent`, `addIntermediateNode` reused as-is |
| `frontend/app/block/blockframe.tsx` | No change — minimize button and caret behavior unchanged |

---

## Out of scope

- Animated transition (headers sliding from old column to new position).
- A "restore whole column" affordance on the dissolved column block — individual pane restore triggers undissolve implicitly.
- Dissolving a column into a floating (torn-off) window's column.
- Dissolving nested intermediate rows (non-root Row parents) — spec assumes root Row as the parent of top-level columns. The `findParent` guard catches non-root cases and bails.
