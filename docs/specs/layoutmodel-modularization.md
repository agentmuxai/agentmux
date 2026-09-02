# LayoutModel Modularization Spec

**Status:** Ready to implement
**Date:** 2026-03-04
**Owner:** AgentA
**Target file:** `frontend/layout/lib/layoutModel.ts` (1,696 lines)

---

## Problem

`LayoutModel` is a single 1,696-line god-class with 6 distinct responsibilities:

1. **Tree state management** — reducer, action dispatch, backend action processing
2. **Persistence** — debounced saves to WaveObject, initialization from WaveObject
3. **Layout geometry** — computing node rects, resize handles, CSS transforms
4. **Resize interaction** — resize context tracking, mouse move/end handlers
5. **Focus & navigation** — focus stack, directional navigation, block number switching
6. **Magnify & ephemeral nodes** — magnification toggle, ephemeral node lifecycle

All 6 concerns are interleaved within the class, making it hard to test any one in isolation, understand the flow of a single feature, or modify one concern without risk to the others.

---

## Existing Module Context

The layout system already has good separation in the *surrounding* files:

| File | Lines | Responsibility | Status |
|------|-------|----------------|--------|
| `layoutTree.ts` | 544 | Pure tree mutations (insert, delete, split, swap, resize, move) | Clean |
| `layoutNode.ts` | 303 | Node construction, find, walk, balance | Clean |
| `types.ts` | 413 | All type definitions | Clean |
| `utils.ts` | 106 | setTransform, getCenter, navigation offset | Clean |
| `TileLayout.tsx` | 537 | React rendering of the layout | Clean |
| `layoutModelHooks.ts` | 113 | React hooks wrapping LayoutModel | Clean |
| `layoutAtom.ts` | 13 | WaveObject atom factory | Clean |
| `nodeRefMap.ts` | 25 | WeakMap for DOM refs | Clean |
| **layoutModel.ts** | **1,696** | **Everything else** | **Target** |

The class is imported by: `layoutModelHooks.ts`, `TileLayout.tsx`, `layout/index.ts`, and one test file. The public API surface is manageable.

---

## Proposed Split

### New File Structure

```
frontend/layout/lib/
  layoutModel.ts           →  420 lines  (class shell + constructor + tree reducer)
  layoutPersistence.ts     →  120 lines  (init from WaveObject, persist, backend actions)
  layoutGeometry.ts        →  200 lines  (updateTree, updateTreeHelper, getBoundingRect, getLeafOrder)
  layoutResize.ts          →  120 lines  (ResizeContext, onResizeMove, onResizeEnd, onContainerResize)
  layoutFocus.ts           →  180 lines  (focus stack, directional nav, block number switching)
  layoutMagnify.ts         →  160 lines  (magnify toggle, ephemeral node lifecycle)
  layoutNodeModels.ts      →   80 lines  (getNodeModel, cleanupNodeModels, property accessors)
```

Total: ~1,280 lines (shrinks by ~400 lines of debug logging that gets consolidated).

### Design: Mixin Functions, Not Inheritance

Rather than subclasses or a full DI refactor, use **function modules that operate on `LayoutModel`**. This keeps the class as a single coherent object (preserving all existing call sites) while moving implementation into separate files.

Each extracted module exports functions that take `LayoutModel` as the first parameter (or `this`-bound methods assigned in the constructor):

```typescript
// layoutFocus.ts
export function switchNodeFocusInDirection(
    model: LayoutModel,
    direction: NavigateDirection,
    inWaveAI: boolean
): NavigationResult { ... }

export function focusNode(model: LayoutModel, nodeId: string): void { ... }
export function validateFocusedNode(model: LayoutModel, leafOrder: LeafOrderEntry[]): void { ... }
```

```typescript
// layoutModel.ts (after refactor)
import { switchNodeFocusInDirection, focusNode, ... } from "./layoutFocus";

export class LayoutModel {
    // ... atoms and properties stay here ...

    switchNodeFocusInDirection(direction: NavigateDirection, inWaveAI: boolean): NavigationResult {
        return switchNodeFocusInDirection(this, direction, inWaveAI);
    }

    focusNode(nodeId: string) {
        focusNode(this, nodeId);
    }
}
```

**Why this approach:**
- Zero changes to any file outside `layout/lib/`
- No breaking API changes — `LayoutModel` keeps all its public methods
- Each module is independently testable by constructing a minimal mock `LayoutModel`
- Incremental — can extract one module at a time, commit, verify

---

## Module Breakdown

### 1. `layoutPersistence.ts` (~120 lines)

**Extracted from:** lines 363-591

**Functions:**
- `initializeFromWaveObject(model)` — read WaveObject, set initial tree state
- `onBackendUpdate(model)` — handle WaveObject change notifications
- `processPendingBackendActions(model)` — iterate backend action queue
- `handleBackendAction(model, action)` — switch on action type, dispatch to treeReducer
- `persistToBackend(model)` — debounced write back to WaveObject

**Internal state moved:**
- `persistDebounceTimer` — stays on LayoutModel, accessed by persistence functions
- `processedActionIds` — stays on LayoutModel, accessed by persistence functions

**Dependencies:** `treeReducer` (on LayoutModel), `waveObjectAtom`, `localTreeStateAtom`

---

### 2. `layoutGeometry.ts` (~200 lines)

**Extracted from:** lines 730-963, 1684-1696

**Functions:**
- `updateTree(model, balanceTree?)` — main tree walk: compute leafs, additional props, resize handles
- `updateTreeHelper(model, node, additionalPropsMap, leafs, ...)` — per-node geometry callback
- `getBoundingRect(model)` — get normalized container dimensions
- `getLeafOrder(leafs, additionalProps)` — sort leafs by tree key (standalone, already a free function)

**Why separate:** This is pure geometry computation. It reads atoms and DOM refs, computes rects and transforms, and writes to `leafs`, `leafOrder`, and `additionalProps` atoms. It has no persistence, focus, or interaction logic.

---

### 3. `layoutResize.ts` (~120 lines)

**Extracted from:** lines 1508-1600

**Types moved:**
- `ResizeContext` interface (lines 67-76)
- `DefaultGapSizePx`, `MinNodeSizePx` constants (lines 78-79)

**Functions:**
- `onContainerResize(model)` — container resize handler
- `stopContainerResizing(model)` — debounced resize end
- `onResizeMove(model, resizeHandle, x, y)` — handle drag to compute new node sizes
- `onResizeEnd(model)` — commit pending resize action

**Internal state moved:**
- `resizeContext` — stays on LayoutModel, accessed by resize functions

---

### 4. `layoutFocus.ts` (~180 lines)

**Extracted from:** lines 968-1331

**Functions:**
- `validateFocusedNode(model, leafOrder)` — clean up focus stack after tree changes
- `switchNodeFocusInDirection(model, direction, inWaveAI)` — ray-cast directional navigation
- `switchNodeFocusByBlockNum(model, blockNum)` — jump to pane by number
- `focusNode(model, nodeId)` — set focus on a specific node
- `focusFirstNode(model)` — focus the first leaf
- `getFirstBlockId(model)` — get the first leaf's block ID

**Internal state:**
- `focusedNodeIdStack` — stays on LayoutModel, accessed by focus functions

---

### 5. `layoutMagnify.ts` (~160 lines)

**Extracted from:** lines 1332-1492

**Functions:**
- `magnifyNodeToggle(model, nodeId, setState?)` — toggle magnification
- `newEphemeralNode(model, blockId)` — create an ephemeral (floating) node
- `addEphemeralNodeToLayout(model)` — commit ephemeral node into the tree
- `updateEphemeralNodeProps(model, node, addlPropsMap, leafs, magnifiedNodeSizePct, boundingRect)` — compute ephemeral node geometry
- `closeNode(model, nodeId)` — close a node (handles ephemeral + magnified special cases)
- `closeFocusedNode(model)` — shorthand for closing the focused node

**Internal state:**
- `magnifiedNodeId`, `lastMagnifiedNodeId`, `lastEphemeralNodeId` — stay on LayoutModel

**Note:** `closeNode` is placed here (not in tree state) because it has special-case logic for ephemeral and magnified nodes. The actual tree deletion is delegated to `treeReducer`.

---

### 6. `layoutNodeModels.ts` (~80 lines)

**Extracted from:** lines 1110-1681

**Functions:**
- `getNodeModel(model, node)` — create/cache a NodeModel for a leaf
- `cleanupNodeModels(model, leafOrder)` — remove orphaned node models
- `getNodeByBlockId(model, blockId)` — find a leaf by block ID
- `getNodeAdditionalPropertiesAtom(model, nodeId)` — atom for a node's additional props
- `getNodeAdditionalPropertiesById(model, nodeId)` — direct property access
- `getNodeTransformById(model, nodeId)` — CSS transform accessor
- `getNodeRectById(model, nodeId)` — rect accessor
- Plus the convenience wrappers (`getNodeTransform`, `getNodeRect`, `getNodeAdditionalProperties`)

---

### 7. `layoutModel.ts` (what remains, ~420 lines)

- All property/atom declarations (lines 82-240)
- Constructor (lines 246-361) — atom initialization, calls `initializeFromWaveObject()`
- `registerTileLayout()` (lines 597-604)
- `treeReducer()` (lines 610-708) — the central action dispatcher
- `onTreeStateAtomUpdated()` (lines 714-719)
- `onDrop()` (lines 1497-1503)
- `getPlaceholderTransform()` (lines 1022-1103) — could also move to geometry, but it's closely tied to the pending action atom
- Delegating methods that call into the extracted modules

---

## Implementation Order

Each step is independently committable and testable:

| Step | Module | Risk | Verify |
|------|--------|------|--------|
| 1 | `layoutResize.ts` | Low — self-contained interaction handlers | Resize panes in dev |
| 2 | `layoutFocus.ts` | Low — pure navigation logic | Arrow key nav, Ctrl+1-9 |
| 3 | `layoutNodeModels.ts` | Low — accessor functions | Panes render correctly |
| 4 | `layoutMagnify.ts` | Medium — closeNode has side effects | Magnify toggle, close pane, ephemeral nodes |
| 5 | `layoutPersistence.ts` | Medium — touches backend sync | Reload tab, backend-initiated layout changes |
| 6 | `layoutGeometry.ts` | Medium — core tree walk | All layout rendering, window resize |

---

## What NOT to Change

- **No changes to `types.ts`, `layoutTree.ts`, `layoutNode.ts`, `utils.ts`** — these are already clean
- **No changes to `TileLayout.tsx` or `layoutModelHooks.ts`** — they only import `LayoutModel`, which keeps its public API
- **No new classes or inheritance** — the `LayoutModel` class stays as-is
- **No changes to imports outside `layout/lib/`** — all consumers import from `layout/index.ts` which re-exports `LayoutModel`
- **No functional changes** — pure refactor, zero behavior changes
- **Keep `DefaultAnimationTimeS` in `layoutModel.ts`** — it's only used in the constructor

---

## Visibility Changes

Some properties on `LayoutModel` that are currently `private` will need to become either `public` or use a package-internal convention (e.g., prefix with `_`) so the extracted modules can access them:

| Property | Currently | Needed By |
|----------|-----------|-----------|
| `localTreeStateAtom` | private | persistence, geometry |
| `waveObjectAtom` | private | persistence |
| `persistDebounceTimer` | private | persistence |
| `processedActionIds` | private | persistence |
| `focusedNodeIdStack` | private | focus |
| `resizeContext` | private | resize |
| `resizeHandleSizePx` | private | geometry |
| `isContainerResizing` | private | resize |
| `nodeModels` | private | nodeModels |

**Approach:** Remove `private` from these and add a `/** @internal */` JSDoc tag. TypeScript's `private` keyword has no runtime effect — this is purely documentation. The extracted functions are co-located in the same `layout/lib/` directory, so this is reasonable encapsulation.

---

## Success Criteria

- [ ] `layoutModel.ts` is under 450 lines
- [ ] No file in `layout/lib/` exceeds 550 lines (layoutTree.ts is currently 544)
- [ ] All existing layout tests pass (`layoutModel.test.ts`)
- [ ] Manual verification: split panes, resize, magnify, close, directional nav, ephemeral nodes, reload
- [ ] Zero import changes outside `layout/lib/`
- [ ] `tsc --noEmit` passes
- [ ] Hot reload works in `task dev`

---

## Testing Plan

Each extracted module should be testable by creating a partial `LayoutModel` mock:

```typescript
// Example: testing layoutFocus.ts
function createMockModel(overrides: Partial<LayoutModel>): LayoutModel {
    return {
        getter: (atom) => atomValues.get(atom),
        setter: (atom, value) => atomValues.set(atom, value),
        treeState: { rootNode: testTree, focusedNodeId: null, ... },
        focusedNodeIdStack: [],
        leafs: atom([]),
        leafOrder: atom([]),
        ...overrides,
    } as unknown as LayoutModel;
}
```

New tests to add:
- `layoutFocus.test.ts` — directional navigation with known node positions
- `layoutResize.test.ts` — resize context computation, min size enforcement
- `layoutGeometry.test.ts` — updateTreeHelper with known tree shapes
