# Pane Highlight Border — Fix Implementation Plan

**Branch:** `agenta/fix-pane-highlight-border`
**Date:** 2026-03-23
**Analysis:** `C:\Systems\agentmux-pane-border-analysis\PANE_BORDER_HIGHLIGHT_ANALYSIS.md`

---

## Problem Summary

All pane highlight border inconsistencies trace to one bad line in `blockframe.tsx`:

```typescript
const numBlocks = () => atoms.tabAtom()?.blockids?.length ?? 0;
```

Two independent flaws:
1. **Wrong scope** — `atoms.tabAtom` reads `staticTabId` (frozen at window init), not the tab the BlockFrame belongs to. In multi-tab windows, every pane in every tab reads blockids from the original tab.
2. **Wrong metric** — `blockids` is a backend registry (includes sub-blocks, background blocks), not visible pane count.

The `block-no-highlight` class it feeds suppresses the focus border with `!important`, causing:
- Single pane: border randomly missing (depends on whether unrelated blocks inflate `blockids`)
- Multi-tab window: all panes in every non-initial tab lose their border
- Tab drag onto existing window: all panes in the receiving window lose their border
- Split flash: border disappears briefly during websocket lag after a split

Secondary bug: `validateFocusedNode` never bootstraps focus on fresh tabs — no pane highlighted until user clicks.

User preference confirmed: **always show the accent border, including single-pane layouts.**

---

## Fix 1 — Remove `block-no-highlight` entirely

### 1a. `frontend/app/block/blockframe.tsx`

Remove the `numBlocks` computation, the `numBlocksInTab` prop pass-through, and the `block-no-highlight` class condition.

**Lines to change: ~692, 753, 756**

```diff
-               "block-no-highlight": props.numBlocksInTab === 1,
```
```diff
-    const numBlocks = () => atoms.tabAtom()?.blockids?.length ?? 0;
     return (
         <Show when={blockId && blockData()}>
-            <BlockFrame_Default {...props} numBlocksInTab={numBlocks()} />
+            <BlockFrame_Default {...props} />
         </Show>
     );
```

### 1b. `frontend/app/block/blocktypes.ts`

Remove the prop from `BlockFrameProps`.

```diff
-    numBlocksInTab?: number;
```

### 1c. `frontend/app/block/block.scss`

Remove the `block-no-highlight` CSS suppression rule (lines ~458–463).

```diff
-           &.block-no-highlight,
-           &.block-preview {
-               .block-mask {
-                   border: 2px solid rgb(from var(--border-color) r g b / 10%) !important;
-               }
-           }
```

Note: `block-preview` suppression is removed here too. Preview panes are ephemeral drag-state artifacts — they should either show the border or be styled separately without the `!important` sledgehammer. If a preview-specific style is later needed it can be re-added intentionally.

---

## Fix 2 — Bootstrap focus on first render

### `frontend/layout/lib/layoutFocus.ts`

Add a bootstrap path at the top of `validateFocusedNode` to handle the initial state where both `treeState.focusedNodeId` and `focusedNodeIdStack` are empty (fresh/new tab, never persisted focus).

```diff
 export function validateFocusedNode(model: LayoutModel, leafOrder: LeafOrderEntry[]) {
+    // Bootstrap: first layout computation for a tab with no persisted focus.
+    // The standard guard (focusedNodeId !== focusedNodeId) is a no-op when both are
+    // undefined, leaving all panes unfocused until the user clicks.
+    if (!model.treeState.focusedNodeId && model.focusedNodeIdStack.length === 0) {
+        if (leafOrder.length > 0) {
+            model.treeState.focusedNodeId = leafOrder[0].nodeid;
+            model.focusedNodeIdStack = [model.treeState.focusedNodeId];
+            model.setter(model.localTreeStateAtom, { ...model.treeState });
+        }
+        return;
+    }
     if (model.treeState.focusedNodeId !== model.focusedNodeId) {
```

---

## Fix 3 — Clean up LayoutModel on tab drag-out

### `frontend/app/drag/DragOverlay.tsx`

`deleteLayoutModelForTab` is called on tab close but not on tab drag-out. The source window leaks a LayoutModel for the moved tab. Add cleanup after `MoveTabToWorkspace` succeeds.

```diff
 WorkspaceService.MoveTabToWorkspace(
     data.payload.tabId,
     data.sourceWorkspaceId,
     myWsId
-).catch(...)
+).then(() => {
+    deleteLayoutModelForTab(data.payload.tabId);
+}).catch(...)
```

Also add the import for `deleteLayoutModelForTab` if not already present.

---

## Verification Checklist (manual regression)

After `task dev` is running, test:

- [ ] Fresh window, 1 pane: accent border visible
- [ ] Fresh window, 2+ panes: only focused pane has accent border
- [ ] Click a pane: border moves to clicked pane immediately
- [ ] Multi-tab window: switch tabs, each tab's focused pane retains its border independently
- [ ] Split a pane: no flash — border stays on the focused pane throughout
- [ ] Close a pane (2→1): remaining pane keeps border
- [ ] Open new tab (fresh): first pane auto-highlighted without needing a click
- [ ] Drag pane out to new window: new window's pane is highlighted
- [ ] Drag pane onto existing window: receiving window's panes retain correct highlight

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/block/blockframe.tsx` | Remove `numBlocks`, `numBlocksInTab` prop, `block-no-highlight` class |
| `frontend/app/block/blocktypes.ts` | Remove `numBlocksInTab?: number` |
| `frontend/app/block/block.scss` | Remove `block-no-highlight` + `block-preview` CSS suppression rule |
| `frontend/layout/lib/layoutFocus.ts` | Add bootstrap path in `validateFocusedNode` |
| `frontend/app/drag/DragOverlay.tsx` | Call `deleteLayoutModelForTab` after successful tab move |
