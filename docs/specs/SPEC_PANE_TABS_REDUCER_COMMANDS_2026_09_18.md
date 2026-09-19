# SPEC: Pane tabs as reducer commands — one writer for "which blocks are in which pane"

**Date:** 2026-09-18
**Status:** active — Phase 0 implemented in #3414 (§4); Phases 1–3 not started.
**Author:** AgentA@Area54
**Related:** `docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md`
(§6 names this as its follow-up; its orphan reaper, §4.9, is the backstop for
what this spec prevents),
`docs/specs/SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md` (the reducer is the
single writer of `db_layout`),
`docs/specs/SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` (proposed:
frontend sends intents, reducer computes the tree — this spec is a narrow,
stack-only slice of that direction),
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (introduced
`blockStack` / `activeBlockId`, §4.3),
`docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md` (§A3 chose
"zero backend work" for adding a pane tab — this spec revisits that),
`docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` (had asked
for a generic reducer path, l.341-348).

---

## 0. Why

A pane (a layout leaf) can hold several blocks as tabs. The reducer stores that
list, but only the frontend ever changes it. So "this block exists in this tab"
and "this block is in a pane" are two separate writes, from two different
places, and they can come apart. When they do, the block keeps running with no
pane showing it. That is how the Manoz orphan in
`SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md` §2 happened, and §2 below
finds three more ways it can happen today.

The goal: **every block in a tab is in exactly one pane, and the reducer
enforces it.** Tab adds, switches and closes become reducer commands; creating a
block and placing it becomes one step.

## 1. How it works today (verified in code)

### 1.1 Where stacks live

- A leaf's `data` holds `block_stack` and `active_block_id`, with the rule
  `block_id == active_block_id ∈ block_stack` when the stack is non-empty
  (`agentmux-common/src/layout_types.rs:32-56`). Nothing checks that rule;
  `validate_layout_invariants` does not cover it.
- There is **no stack command** in the reducer. The layout commands
  (`agentmux-common/src/ipc.rs:486-655`, dispatched at
  `agentmux-srv/src/reducer.rs:78-213`) insert, delete, move, swap, resize,
  split, replace and clear whole leaves; none adds, activates or removes a
  stack member.

### 1.2 Every change is a frontend edit plus a whole-tree push

- All stack writers are in `frontend/layout/lib/layoutStack.ts`:
  `pushBlockOntoStack`, `setActiveBlockInStack`, `closeBlockInStack`,
  `addWidgetAsPaneTab`. Each edits the frontend's tree in place and calls
  `persistToBackend()`. The file's header (l.12-17) says they deliberately skip
  `treeReducer`.
- `persistToBackend` (`frontend/layout/lib/layoutPersistence.ts:391`) is a
  100ms trailing debounce. It sends the whole tree through `UpdateObject`,
  which becomes `Command::LayoutSetTree`
  (`agentmux-srv/src/server/service/object.rs:486-627`).
- `handle_layout_set_tree` (`agentmux-srv/src/reducer/layout.rs:86-170`)
  replaces the tree wholesale. It has **no base version, no compare-and-swap**.
  The last writer wins.
- The frontend never adopts a backend-changed tree once it has one
  (`onBackendUpdate`, `layoutPersistence.ts:59-82`); backend changes reach it
  only through the `pendingbackendactions` queue. A backend change that isn't
  also queued is overwritten by the next frontend push.

### 1.3 Creating a tab is two steps

Every "new pane tab" does `pane.open { skip_placement: true }` first — the block
now exists in the tab with no pane — then pushes it onto a stack on the
frontend:

- "+" in the pane chrome → `addWidgetAsPaneTab` (`layoutStack.ts:78-114`);
- the agent History tab → `open-history-tab.ts:108-144`;
- quick fork → `quick-fork.ts:204-219`.

`pane.open` with `skip_placement` (`agentmux-srv/src/server/app_api/mod.rs:167-210`)
queues no layout action. No backend path places a block into an existing stack.

### 1.4 The backend→frontend channel has no stack verbs

The frontend understands `insert`, `delete`, `insertatindex`, `clear`,
`replace`, `splithorizontal`, `splitvertical`
(`frontend/layout/lib/types.ts:56-64`, handled in `layoutPersistence.ts:213-385`).
Every insert/replace/split builds a fresh leaf with a single block. Nothing can
say "add this block to that pane's tabs" or "remove this one tab".

## 2. Ways a block ends up in a tab but in no pane

1. **The two-step create is interrupted.** Between `pane.open { skip_placement }`
   and the frontend's push (plus its 100ms debounce), a reload, crash or window
   close leaves the block with no leaf. `addWidgetAsPaneTab` deletes the block
   if the pane vanished (`layoutStack.ts:110`), but only if the frontend is
   still there to do it.
2. **A backend `delete` for one tab removes the whole pane.** The frontend's
   `delete` handler looks the leaf up with `getNodeByBlockId`, which also
   matches background stack members (`frontend/layout/lib/layoutNodeModels.ts:222-230`),
   then deletes the whole leaf (`layoutPersistence.ts:232-269`). Every
   `delete_block` saga run queues exactly that action
   (`agentmux-srv/src/sagas/delete_block.rs`, Step 2b). So when a terminal tab's
   shell exits and close-on-exit closes it, the frontend drops its whole pane —
   and every sibling tab's block, agents included, is left running with no
   pane. The backend side disagrees: its `find_node_id_by_block` matches only
   the visible block (`agentmux-srv/src/backend/layout/mod.rs:140-151`).
   Tear-off / redock / promote queue the same `delete`
   (`agentmux-srv/src/server/service/tear_off.rs`, `tab_move.rs`) and have the
   same effect on a stacked pane.
3. **A stale whole-tree push drops a backend-placed member.** Any stack change
   the backend makes (today: the `ClosePane` MCP tool; with this spec, backend
   placement) is lost if a frontend push built from an older tree lands after
   it (§1.2).

(The pane-close spec's own bug — the pane's × deleting only the visible tab —
was the fourth, fixed in #3402.)

Smaller inconsistencies found on the way:

- When the visible tab's block is gone, the backend activates the **first** live
  member (`reactivate_dangling_active_stack_members`,
  `agentmux-srv/src/backend/layout/mod.rs:266-300`); the frontend activates the
  **right-hand neighbour** (`closeBlockInStack`, `layoutStack.ts:181-189`).
- The frontend's own dangling-leaf prune (`layoutPersistence.ts:134-174`) checks
  only `data.blockId`, not stack members.

## 3. Design

### 3.1 Invariants the reducer owns

For every tab, over its non-floating, non-sub-block `block_ids`:

- **I1** — each block is in exactly one leaf's members (`block_stack`, or
  `[block_id]` for a leaf with no stack).
- **I2** — a leaf with a stack satisfies
  `block_id == active_block_id ∈ block_stack`, with no duplicates.
- **I3** — no leaf references a block that doesn't exist (already enforced by
  `prune_dangling_block_refs` on every layout write).

I1 is the new one. It is checked, not assumed: `validate_layout_invariants`
gains I1 and I2 and reports violations. A block that is in a tab but in no
leaf **and** was created more than N seconds ago (the pane-close spec's reaper
rules, §4.9) is an I1 violation.

### 3.2 Stack commands

New `Command`s, each applied to the reducer's tree for the tab, each emitting a
`LayoutTreeReplaced`-style event the persist subscriber already handles:

| Command | Effect |
|---|---|
| `LayoutStackPush { tab_id, target_block_id, block_id, activate }` | Add `block_id` to the stack of the leaf holding `target_block_id` (creating the stack from a single-block leaf). |
| `LayoutStackActivate { tab_id, block_id }` | Make `block_id` the visible member of its leaf. |
| `LayoutStackRemove { tab_id, block_id }` | Remove `block_id` from its leaf. If it was visible, activate the **right-hand neighbour** (the frontend's existing rule; the backend's first-live rule changes to match). If it was the last member, delete the leaf. |
| `CreateBlockInStack { tab_id, target_block_id, meta, activate }` | `CreateBlock` + `LayoutStackPush` in ONE reducer step. The block never exists outside a pane. |

Addressing by block id, not node id, on purpose: node ids are frontend-minted
and change across a reload's rebuild; block ids don't.

### 3.3 Frontend: intents, applied optimistically

`layoutStack.ts` keeps its local edit (so the tab strip reacts instantly) and
**also** sends the matching command. It sends a client-minted `actionid`
with it and records that id as already processed. The backend applies the
command and queues a pending action (`stackpush` / `stackactivate` /
`stackremove`) with the same `actionid`. The originating frontend skips it —
`processPendingBackendActions` already de-duplicates by `actionid`
(`layoutPersistence.ts:180-206`) — while any other view of the tab applies it.

For creation, the frontend calls `pane.open { stack_onto_block_id }` (new
parameter) instead of `skip_placement` + push. The backend runs
`CreateBlockInStack` and queues a `stackpush` for the frontend.
`skip_placement` stays for callers that genuinely want an unplaced block;
none of the three stack callers in §1.3 do.

### 3.4 Backend `delete` means "remove this block from the layout"

Every emitter of the `delete` pending action means "this block is no longer in
this tab's layout" (a move or a close). The frontend handler applies it to
that block only: remove it from its leaf's stack, and delete the leaf only if it
was the last member. The backend's `LayoutDeleteNodeByBlock` gets the same
semantics (member removal via `LayoutStackRemove`), so the two sides agree.

This fixes §2.2 on its own and doesn't depend on the rest of the spec — it
ships first (Phase 0).

### 3.5 Stale pushes can't drop stack members

A whole-tree push (resize, split, drag) still replaces the tree. To keep it
from undoing a stack change it never saw:

- The reducer keeps a per-tab `layout_revision`, bumped by every
  backend-originated change (every command in §3.2 not initiated by this
  frontend's own `actionid`, every queued pending action).
- The frontend sends the revision it last drained with each push.
- `handle_layout_set_tree` with a stale revision still applies the push, but
  first re-applies every stack command queued since that revision, the same
  way the frontend would when it drains them. The pushed geometry wins; stack
  membership from undrained commands is kept.

This is narrower than a general conflict resolver. Only stack membership is
merged; everything else keeps last-writer-wins, as today.

### 3.6 One rule for "which tab becomes visible"

`LayoutStackRemove`'s neighbour choice is the single implementation. The
backend's dangling-active repair calls it, and the frontend's optimistic
`closeBlockInStack` mirrors it, with a shared test vector (the same inputs and
expected outputs in a Rust test and a vitest).

## 4. Phases

**Phase 0 — the `delete` action fix (§3.4).** Implemented in #3414:
`frontend/layout/lib/stackMembers.ts`, `backend::layout::remove_stack_member`,
and `LayoutDeleteNodeByBlock` member removal. Frontend handler change, the
`LayoutDeleteNodeByBlock` semantics change, and tests: a terminal tab exiting
in a stack with an agent tab leaves the agent tab's pane and block intact. Small;
closes the live orphan path in §2.2.

**Phase 1 — commands and atomic create.** §3.2 commands,
`pane.open { stack_onto_block_id }`, the three frontend callers moved onto it,
pending-action verbs and handler (§3.3), unified neighbour rule (§3.6), I1/I2 in
`validate_layout_invariants` (report-only).

**Phase 2 — stale-push protection (§3.5).**

**Phase 3 — enforce.** The pane-close spec's reaper uses I1 to find orphans;
`validate_layout_invariants` violations are logged loudly with the tab and
block ids.

## 5. Tests

Each must fail on today's code:

- Phase 0: a `delete` pending action for a background stack member removes only
  that member; for the visible member, activates the right-hand neighbour; for
  the last member, removes the leaf. Close-on-exit of a terminal tab in
  `[agent, term]` leaves the agent block in `tab.block_ids` AND in a leaf.
- `CreateBlockInStack` produces a block that is in a leaf in the same reducer
  step; there is no reducer state in which it exists without a leaf.
- `LayoutStackPush` / `Activate` / `Remove` keep I1 and I2.
- A frontend push built before a backend `LayoutStackPush` keeps the pushed
  block in its stack (§3.5).
- The backend and frontend pick the same neighbour for the same stack and
  removal (shared vector).

## 6. Not in scope

- Dragging a tab between panes, reordering tabs, tearing one tab out of a stack.
  None exists today (`PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md:214`);
  when they're built they should be commands here (`LayoutStackMove`), not
  whole-tree pushes.
- Moving other layout edits (resize, split, drag) onto commands.
  `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` covers that.

## 7. Open questions

1. `PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md` §A3 chose "zero
   backend work" for pane tabs. Was that for speed only, or is there a reason
   the backend shouldn't place blocks into stacks? Nothing in the plan says.
2. Floating (torn-off) windows have their own tab context. Does I1 hold for
   them unchanged, or do they need their own rule?
3. Should `layout_revision` (§3.5) reuse the layout object's existing `version`
   field? It is currently ignored on this path.
