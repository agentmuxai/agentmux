# Blank Panes: Orphaned Layout Nodes

**Date:** 2026-04-09
**Instance:** v0.33.69 dev (db at `%APPDATA%/ai.agentmux.cef.v0-33-69/db/wave.db`)

## Symptom

Two blank/dark pane areas in the layout. No content renders; the pane frame exists but is empty.

## Root Cause

The layout tree (`db_layout`) contains leaf nodes that reference blocks which no longer exist in `db_block`. When a block is deleted, its entry is removed from:
- `db_block` (the block row is deleted)
- `db_tab.blockids` (the block ID is removed from the tab's block list)

But the layout tree (`db_layout.rootnode.children[]` and `db_layout.leaforder[]`) is **not pruned** — the deleted block's node remains in the flex tree, occupying space but rendering nothing.

## Evidence

```
Layout 354837d7 has 3 leaf nodes:
  1. blockid: d525dd68  -> NOT IN db_block (orphaned)
  2. blockid: 34128bdc  -> NOT IN db_block (orphaned)
  3. blockid: 1d9a8231  -> EXISTS (agent view)

Tab blockids: [1d9a8231]  (only 1 block, but layout has 3 slots)
```

## Affected Code Path

Block deletion flows through `DeleteBlock` in the service layer:
- `agentmux-srv/src/server/service.rs` — handles the HTTP `object.DeleteBlock` call
- Deletes from `db_block` and removes the ID from `db_tab.blockids`
- Does NOT prune the corresponding node from `db_layout.rootnode`

The frontend's `onNodeDelete` callback (in `tabcontent.tsx`) attempts to remove the node from the layout model, but if the frontend is out of sync or the delete races with a layout update, the orphaned node persists in the database.

## Proposed Fix: Layout Self-Healing

Instead of relying on the delete path to be perfect, add a **layout validation pass** that runs:

1. **On tab load** (when a tab becomes active):
   - Walk `rootnode.children` recursively
   - For each leaf node with a `blockId`, check if the block exists in `db_block`
   - If not, remove that leaf node from the tree and update `leaforder`
   - Save the pruned layout back to the database

2. **After any `DeleteBlock` call** (belt-and-suspenders):
   - Same validation: prune any leaf nodes referencing the deleted block ID
   - This handles the case where the frontend's layout update was lost

3. **Frontend fallback** (defense in depth):
   - In the pane renderer, if a block ID has no corresponding block data, render nothing and trigger a layout prune via RPC
   - Prevents blank rectangles from ever being visible to the user

### Key Principle

The layout tree should be treated as a **derived structure** from the block list — if a block doesn't exist, its layout node must not exist. This invariant should be enforced on read (self-healing), not just on write (delete path).

## Quick Fix (manual, for the current session)

```sql
-- Remove orphaned leaf nodes from the layout
-- Run against: %APPDATA%/ai.agentmux.cef.v0-33-69/db/wave.db
-- (requires app restart after)
```

Or: close the blank panes via the UI (click X on the pane header), which should trigger `onNodeDelete` and prune them.

## Files to Modify

| File | Change |
|------|--------|
| `agentmux-srv/src/server/service.rs` | Add layout pruning after `DeleteBlock` |
| `agentmux-srv/src/backend/wcore/tab.rs` | Add `validate_layout()` called on tab activation |
| `agentmux-srv/src/backend/obj.rs` | Helper: `prune_orphaned_layout_nodes(layout, valid_block_ids)` |
| `frontend/app/tab/tabcontent.tsx` | Render empty placeholder + trigger prune RPC for missing blocks |
