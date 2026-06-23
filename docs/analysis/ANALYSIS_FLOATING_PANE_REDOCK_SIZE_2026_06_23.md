# ANALYSIS: Floating Pane Redock — Ghost vs. Actual Size Mismatch

**Date:** 2026-06-23  
**Status:** Root causes confirmed. Phase 4b implementation underway.

---

## Problem

When a user drags a floating pane over a docked window to redock it:

1. A ghost overlay appears in the target window showing where the pane will land
   (e.g. top-half of a leaf = "Top" drop, thin left band = "OuterLeft" drop).
2. The user releases — but the docked pane **ignores the ghost's position and size**,
   landing wherever `findNextInsertLocation` happens to put it, at the default 50/50
   flex split — regardless of which zone the ghost showed.

The mismatch is worst for **Outer\* directions** (ghost shows 20% band, reality is 50%)
and for **multi-pane targets** where default flex sizes produce unexpected proportions.

---

## Root Cause: Direction Never Reaches the Layout Engine

The ghost is a purely visual DOM element (`floating-redock-drop-placeholder`). The
`DropDirection` computed to position it is **captured only in the target window's
renderer process** and is **never transmitted to the RPC or the layout tree**.

### Data flow — what actually happens

```
[Floater renderer]
    onMouseMove
    → update_floating_redock_hover(x, y)      ← IPC to CEF backend

[CEF backend → all windows]
    floating-redock:hover-state event with { target_label, cursor_x, cursor_y }

[Target window renderer — app-init.ts:186]
    floating-redock:hover-state event received
    → elementFromPoint(clientX, clientY)       ← find leaf under cursor
    → determineDropDirection(leafRect, cursor) ← compute dir  ← STORED HERE ONLY
    → rectForDirection(dir)                    ← compute ghost rect
    → placeholderEl.style = ghost rect         ← purely visual

    ⚠️  dir + targetBlockId are local variables; never emitted, never stored.

[Floater renderer — floating-pane-workspace.tsx:924]
    onMouseUp (armed)
    → WorkspaceService.RedockFloatingPane(
          sourceBlockId,
          sourceTabId,  sourceWsId,
          targetTabId,  targetWsId,
                                           ← NO direction, NO targetBlockId
      )

[Rust backend — service.rs:2412]
    queue_target_layout_insert(store, target_tab_id, block_id)
    → InsertNode action queued on target LayoutState.pendingbackendactions

[Target window renderer — layoutPersistence.ts:105]
    onBackendUpdate → processPendingBackendActions
    → InsertNode action → insertNode() → findNextInsertLocation()
    → new node gets size = DefaultNodeSize (10)  ← fixed default, not ghost %
```

### The ghost system (app-init.ts:138–245)

`installFloatingRedockHoverListener` renders a DOM div sized exactly per direction:

| Direction | Ghost size |
|-----------|-----------|
| Top / Bottom | 50% of leaf height, full width |
| Left / Right | 50% of leaf width, full height |
| OuterTop / OuterBottom | 20% (1/5) of leaf height, full width |
| OuterLeft / OuterRight | 20% (1/5) of leaf width, full height |
| Center | 100% of leaf |

These sizes are correct and match the within-window pane drag ghost
(`getPlaceholderTransform`, `layoutModel.ts:674–684`). But they're visual only.

### The landing system (layoutPersistence.ts:105 + layoutNode.ts:245)

`queue_target_layout_insert` queues an `InsertNode` action (not a split action) on the
target LayoutState. The frontend processes this via `insertNode()` →
`findNextInsertLocation()` — finds the deepest node in the tree that has room for a
child. The new node gets `size = DefaultNodeSize = 10` (types.ts:210).

If the target tab has one existing pane with size 10, the result is **50/50** regardless
of direction. If the user hovered an OuterLeft zone (ghost: 20%), they still get 50%.

The `handleBackendAction` in `layoutPersistence.ts` already has **fully wired handlers**
for `SplitHorizontal` (line 213) and `SplitVertical` (line 241) — these place the new
node in the correct position relative to a named `targetblockid` with a given `nodesize`.
The gap is that `queue_target_layout_insert` always uses `InsertNode` instead.

### The tracking comment

This gap is explicitly noted in the source (added during Phase 4a MVP):

- `frontend/app-init.ts:106–109`:
  ```
  // The block STILL lands as a sibling in the target tab's layout for
  // MVP (backend ignores the direction). Phase 4b will wire the
  // direction through `RedockFloatingPane` so the block lands in the
  // exact slot the user previewed.
  ```

---

## Additional Fragility Found During Deeper Analysis

### The process boundary problem

The ghost direction is computed **in the target window's renderer process**.
`RedockFloatingPane` is called **from the floater's renderer process**. These are
separate Chromium renderer processes. The CEF backend (motion.rs) broadcasts the
hover-state event but **does not store the last-computed ghost state** — it only
forwards the cursor position. So the two processes can never directly share the
`{ targetBlockId, dir }` local variables.

This is the core architectural gap: the original spec
(`SPEC_FLOATING_PANE_REDOCK_2026-05-27.md`) intended `RedockFloatingPane` to be
called by the **target window's renderer** (which has the ghost context) — Phase 4a
reversed this for expedience.

### R1: `onNodeDelete` conflates "moved" with "closed" (partially fixed, BUG-TRACE still active)

`tabcontent.tsx:61–71` (`onNodeDelete`) unconditionally calls `DeleteBlock` when a
layout node is removed. For pane closes this is correct; for `DeleteNode` layout
actions emitted by tear-off/redock, this deletes the block that was supposed to
be moved. The R1 fix was applied (the dedicated block-move guard was removed;
`DeleteNode` handler in `layoutPersistence.ts:153` now uses `model.treeReducer` directly),
but the BUG-TRACE logging in `onNodeDelete` is still active, indicating ongoing
monitoring. This is harmless but indicates the fix is recent and being watched.

### R2: Floater is a raw Win32 popup, not CEF Views (white flash)

The floater window is a raw Win32 popup (no CEF Views wrapper), causing a brief white
flash on tear-off. This is a separate R2 issue unrelated to the size bug.

### R3: Pool-promoted windows lack stable label identity

Pool windows can be promoted with mismatched labels, causing redock target resolution
bugs. This is a separate R3 issue.

### `move_block_to_tab` queues no target layout action

The old wcore path (`dnd.rs:move_block_to_tab`, used by older flows) never queued any
layout action for the destination. The new reducer path (`reducer/block.rs:handle_move_block`)
emits only `BlockMoved` — no layout action. The `service.rs:RedockFloatingPane` handler
must manually call `queue_target_layout_insert` (or the new `queue_target_layout_split`)
after the saga succeeds and then broadcast the updated LayoutState WaveObject.

---

## Fix Architecture (Phase 4b): Push-Then-Store

### Why push-then-store

The floater calls the RPC — it cannot see the target's computed `dir + targetBlockId`.
The target's renderer cannot call the RPC (different process). The solution is to use
the CEF backend as a shared state store:

```
Target renderer → CEF backend: "I see dir=3, blockId=X for window label=main"
Floater → CEF backend: "What did the target see last?" → { dir=3, blockId=X }
Floater → srv RPC: RedockFloatingPane(..., target_block_id=X, direction=3)
```

### Step 1 — CEF backend: store/retrieve ghost state

Add `FloatingRedockGhostState { block_id: String, dir: u8 }` and
`floating_redock_ghost: Mutex<HashMap<String, FloatingRedockGhostState>>` to `AppState`.

Add two IPC commands:
- `set_floating_redock_target { window_label, block_id, dir }` — stores ghost state for a window (clear by passing null/absent block_id)
- `get_floating_redock_target { window_label }` — returns `{ block_id, dir }` or `{}`

`clear_floating_redock_hover` clears all stored ghost states.

### Step 2 — Target renderer: push ghost state (app-init.ts)

After computing `dir` and `leafEl.dataset.blockid`:
```typescript
import { invokeCommand } from "@/app/platform/ipc";
void invokeCommand("set_floating_redock_target", {
    window_label: myLabel,
    block_id: leafEl.dataset.blockid!,
    dir,
});
```

In `clearPlaceholder()`:
```typescript
void invokeCommand("set_floating_redock_target", {
    window_label: myLabel,
    block_id: null,
    dir: null,
});
```

### Step 3 — Floater: query at drop time (floating-pane-workspace.tsx)

In `tryRedockAtCursorInner`, after resolving the target tab/ws, before the RPC:
```typescript
const { invokeCommand } = await import("@/app/platform/ipc");
const ghost = await invokeCommand<{ block_id?: string; dir?: number }>(
    "get_floating_redock_target",
    { window_label: target.label }
).catch(() => ({}));
```

Then include `ghost.block_id ?? null` and `ghost.dir ?? null` in the `RedockFloatingPane` call.

### Step 4 — Backend: emit directional layout action (service.rs)

Parse two new optional args (indices 5 and 6). Add `queue_target_layout_split`:

| DropDirection | actiontype | position |
|---|---|---|
| Top (0) / OuterTop (4) | `SplitVertical` | `before` |
| Bottom (2) / OuterBottom (6) | `SplitVertical` | `after` |
| Left (3) / OuterLeft (7) | `SplitHorizontal` | `before` |
| Right (1) / OuterRight (5) | `SplitHorizontal` | `after` |
| Center (8) | `InsertNode` (current fallback) | — |
| null fallback | `InsertNode` (current behavior) | — |

Node size mapping:
- Inner directions (0-3): `nodesize = None` (DefaultNodeSize = 10 = 50%)
- Outer directions (4-7): `nodesize = Some(2.5)` (2.5/(10+2.5) = 20%)

### Step 5 — Target window layout: process the action

`layoutPersistence.ts:handleBackendAction` already handles `SplitHorizontal` (line 213)
and `SplitVertical` (line 241) via `layoutTree.splitHorizontal` / `splitVertical`.
No changes needed here.

---

## Files Involved

| File | Change |
|------|--------|
| `agentmux-cef/src/state.rs` | Add `FloatingRedockGhostState` + `floating_redock_ghost` field |
| `agentmux-cef/src/commands/window/motion.rs` | Add `set_floating_redock_target`, `get_floating_redock_target`; clear in `clear_floating_redock_hover` |
| `agentmux-cef/src/ipc.rs` | Register two new commands |
| `frontend/app-init.ts` | Push ghost state on hover, clear on `clearPlaceholder()` |
| `frontend/app/store/services.ts` | Add optional `targetBlockId`, `direction` to `RedockFloatingPane` |
| `frontend/app/workspace/floating-pane-workspace.tsx` | Query ghost state before RPC call |
| `agentmux-srv/src/server/service.rs` | Parse optional args 5/6; add `queue_target_layout_split`; call split vs insert |

---

## Test Plan

| Test | File |
|---|---|
| `rectForDirection` returns correct sub-rect per DropDirection (9 directions) | `frontend/layout/tests/rectForDirection.test.ts` |
| `handleBackendAction` routes `SplitHorizontal`/`SplitVertical` correctly | `frontend/layout/tests/layoutPersistence.test.ts` |
| `onNodeDelete` NOT called for moved blocks (R1 regression guard) | existing `layoutTree.test.ts` extended |
| Rust `redock_floating_pane` saga queues `SplitHorizontal` when `dir=3` | `redock_floating_pane.rs` tests |
| Rust saga falls back to `InsertNode` when direction is absent | `redock_floating_pane.rs` tests |
