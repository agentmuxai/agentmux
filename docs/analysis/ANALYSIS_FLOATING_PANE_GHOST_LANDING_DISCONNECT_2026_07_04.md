# ANALYSIS: Floating Pane Ghost-to-Landing Disconnect

**Date:** 2026-07-04
**Status:** Root causes confirmed by direct file inspection (see citations). Builds on and
supersedes the sizing portion of `ANALYSIS_FLOATING_PANE_REDOCK_SIZE_2026_06_23.md`, whose
Phase 4b fix is now shipped. This document identifies what Phase 4b did **not** fix, plus a
mechanical failure mode never previously documented.

**Update (2026-07-04, same day):** Fix Direction A and B below (Phase 4c) have been
implemented and are covered by tests on both sides:
- Backend: `agentmux-srv/src/backend/obj.rs` (`LayoutActionData.nodesizefraction`),
  `agentmux-srv/src/server/service/layout_helpers.rs` (`queue_target_layout_split` now sends
  an exact 0.5/0.2 fraction instead of the old `None`/`Some(3)` guess) —
  `cargo test -p agentmux-srv layout_helpers` passes (2 new tests covering all 8 directions'
  fraction/actiontype/position mapping, plus the Center/unknown fallback).
- Frontend: `frontend/layout/lib/types.ts` (`sizeFraction` on both split actions),
  `frontend/layout/lib/layoutPersistence.ts` (threads `action.nodesizefraction` through),
  `frontend/layout/lib/layoutTree.ts` (`applySizeFraction` — a single helper applied
  uniformly to both the wrap and splice branches, computed from the target's live
  `.size` at split time, run *before* the wrap branch's group-node construction so the
  group still inherits the target's correct pre-split footprint) — `vitest run` passes
  (full suite: 105 files / 1585 tests, including 4 new tests exercising the splice
  no-dilution case, the outer-fraction case, the wrap-branch footprint-preservation case,
  and an end-to-end non-default-target-size case).
- Implementation detail worth noting: the originally-sketched Fix B pseudocode below
  (independently computing `newNode.size` via the Fix A formula, then separately
  "carving" from `targetNode.size`) does not actually conserve the parent's pool — the
  shipped fix unifies both into one calculation (`applySizeFraction`) so `newNode.size +
  targetNode.size` after the split exactly equals `targetNode.size` before it, for both
  branches. See that function's doc comment in `layoutTree.ts` for the corrected math.

---

## Problem (as reported)

Dragging a floating pane onto a docked layout shows a ghost overlay that previews an exact
sub-rect of the hovered leaf. When the pane is dropped:

1. The pane does not land at the rect the ghost showed — proportions are visibly off.
2. **Other, unrelated panes resize** to make room, even though the ghost never touched them.
3. The overall experience feels like the ghost and the landing are two disconnected systems
   rather than the ghost "becoming" the pane.

Goal: make the landing geometry deterministically equal to the last-shown ghost rect, with no
side effects on panes outside that rect — i.e., the ghost visually *is* a preview of the final
pane, not a rough hint.

---

## What Phase 4b already fixed (confirmed still in place)

`ANALYSIS_FLOATING_PANE_REDOCK_SIZE_2026_06_23.md` diagnosed that the ghost's `DropDirection`
was computed only in the target window's renderer and never reached the backend — the block
always landed via a generic `InsertNode` at `DefaultNodeSize` regardless of which zone was
highlighted. That gap is closed:

- `frontend/app-init.ts:263-270` — after computing `dir` and the hovered leaf's `blockId`, the
  target renderer pushes `{ window_label, block_id, dir }` to the CEF host via
  `set_floating_redock_target`.
- `agentmux-cef/src/commands/window/motion.rs` — hosts a `floating_redock_ghost` map keyed by
  window label (per the 06-23 doc's "push-then-store" design).
- `frontend/app/workspace/floating-pane-workspace.tsx:512-519` — the floater queries
  `get_floating_redock_target` at drop time and threads `target_block_id` + `direction` into
  `RedockFloatingPane`.
- `agentmux-srv/src/server/service/layout_helpers.rs:109-148` (`queue_target_layout_split`) —
  now routes to `SplitHorizontal`/`SplitVertical` (not generic insert) for all 8 non-Center
  directions, so the block lands **adjacent to the correct target leaf, on the correct axis,
  on the correct side.** This part of the disconnect — landing next to the wrong pane, or as a
  same-axis sibling appended at the end — is resolved.

What remains is **size**, and a second, previously undocumented mechanism.

---

## Root cause 1 (known, still open): server-side size is a hardcoded guess

`agentmux-srv/src/server/service/layout_helpers.rs:105-129`:

```rust
// Outer directions use `nodesize = Some(3)` so the new node occupies ≈23%
// (3 / 13) — the nearest representable integer to the ghost's `height/5`
// (20%) when the target node is at DefaultNodeSize (10).
// Inner directions use `nodesize = None` (DefaultNodeSize = 10 → 50/50).
let (actiontype, position, nodesize): (&str, &str, Option<u32>) = match dir {
    0 | 4 => ("splitvertical",   "before", if dir >= 4 { Some(3) } else { None }),
    ...
```

The comment is explicit: this is only correct **when the target leaf is at exactly
`DefaultNodeSize` (10)**. The ghost's rect (`app-init.ts:150-174`, `rectForDirection`) is always
computed from the target leaf's *live* `getBoundingClientRect()` — i.e. it reflects the leaf's
**actual current size**, including any prior user resize or any parent with more than the
default two children. The backend has no way to know that size (`layout_helpers.rs:118-121`:
*"the exact ratio depends on the target node's current flex size which isn't available
server-side"*) — so any leaf that isn't at the pristine default 50/50 gets a landing ratio that
doesn't match what the ghost showed. This reproduces most visibly on:
- A leaf that the user has manually resized before the drop.
- A leaf that is one of 3+ siblings in a row/column (its rendered fraction is not 50%, but the
  server still assumes it is).

**This part of the disconnect is a known limitation, not a bug** — the doc that introduced it
already flagged the approximation as temporary. It is not fixable in Rust without either (a)
the backend maintaining a shadow copy of the frontend's live layout tree (rejected in the 06-23
doc's design — see `layout_helpers.rs:56-66`'s comment on why layout state is frontend-owned),
or (b) moving the size computation to where the real size already lives — see Fix Direction A.

---

## Root cause 2 (new): the *splice* code path resizes uninvolved siblings — this is not a bug, it's what `size` means

This is the mechanical explanation for "other panes must resize," and it is independent of the
size-accuracy problem above — it would happen even if the backend sent a perfectly accurate
size.

### The rendering model treats `size` as a shared-pool flex ratio

`frontend/layout/lib/layoutGeometry.ts`, `updateTreeHelper()`:

```ts
// line 139-140
const totalChildrenSize = node.children.reduce((acc, child) => acc + getNodeSize(child), 0);
const pixelToSizeRatio = totalChildrenSize / nodePixels;
// line 150-151
width: nodeIsRow ? childSize / pixelToSizeRatio : nodeRect.width,
height: nodeIsRow ? nodeRect.height : childSize / pixelToSizeRatio,
```

Every child's *rendered pixel size* is its own `size` divided by a ratio computed from **all**
sibling sizes combined. `size` is not a percentage of a slot; it's a flex-grow-style share of a
shared pool. Adding any sibling with nonzero `size` to a children array necessarily shrinks
every other sibling in that array, proportionally, by construction.

### The split code has two branches, and only one of them exposes this pool to sibling panes

`frontend/layout/lib/layoutTree.ts`, `splitHorizontal()` (lines 462-502, `splitVertical` mirrors
it at 506-544):

```ts
const parent = findParent(layoutState.rootNode, targetNodeId);
if (parent && parent.flexDirection === FlexDirection.Row) {
    const insertIndex = position === "before" ? index : index + 1;
    parent.children.splice(insertIndex, 0, newNode);          // ← SPLICE branch
} else {
    const groupNode = newLayoutNode(FlexDirection.Row, targetNode.size, [targetNode], undefined);
    groupNode.children = position === "before" ? [newNode, targetNode] : [targetNode, newNode];
    parent.children[index] = groupNode;                        // ← WRAP branch
}
```

- **Wrap branch** (split axis ≠ parent's existing flex axis — e.g. splitting Top/Bottom on a
  leaf that lives in a horizontal row): the target is replaced *in place* by a brand-new
  2-child group node, and that group is given `targetNode.size` — i.e. it takes over exactly
  the pixel footprint the target used to occupy in the outer parent. **No other sibling in the
  outer parent is touched.** Inside the new group, only `targetNode` and `newNode` share the
  pool — so only Root Cause 1 (wrong ratio) applies here, not sibling resize.

- **Splice branch** (split axis == parent's existing flex axis — e.g. splitting Left/Right on a
  leaf that is one of several panes already in a row): `newNode` is spliced directly into the
  **same children array as every other sibling in that row**, unrelated ones included.
  `pixelToSizeRatio` is then recomputed over the enlarged pool (§ above), so **every sibling in
  that row shrinks**, not just the target — exactly the "other panes must resize" symptom the
  user is describing. This is unavoidable under the current model whenever the ghost's
  direction matches the parent's existing axis, which is precisely the common case for a row or
  column of 2+ panes (Left/Right in a row, Top/Bottom in a column) — arguably the *majority* of
  real redock drops.

Neither branch touches `targetNode.size` (confirmed: no assignment to `targetNode.size` appears
in either function) — the target's own size is never reduced to "make room"; room is manufactured
by diluting the shared pool instead. That is the structural reason the landing feels
disconnected from a ghost that only ever highlighted a sub-rect of one leaf: the ghost implies
"carve this rect out of the target," but the implementation instead does "add a new claim to
everyone's shared pool."

---

## Why the fix belongs in the frontend, not the backend

`agentmux-srv/src/server/service/layout_helpers.rs:118-121` says the target's current size
"isn't available server-side" — true, but the frontend handler that actually *applies* the
split already has it, unused:

`frontend/layout/lib/layoutPersistence.ts`, `handleBackendAction()`, `SplitHorizontal` case
(lines 215-242, `SplitVertical` mirrors at 243-270):

```ts
case LayoutTreeActionType.SplitHorizontal: {
    const targetNode = model?.getNodeByBlockId(action.targetblockid);   // ← real, current size is right here
    ...
    const newNode = newLayoutNode(undefined, action.nodesize, undefined, {  // ← but only action.nodesize (backend's guess) is used
        blockId: action.blockid,
    });
    const splitAction: LayoutTreeSplitHorizontalAction = {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId: targetNode.id,
        newNode: newNode,
        position: action.position,
    };
    model.treeReducer(splitAction, false);
```

`targetNode` (with its live `.size`) is resolved on line 216/244 and then discarded for sizing
purposes — `action.nodesize` (the backend's `None`/`Some(3)` approximation) is used instead of
`targetNode.size`. This is the same asymmetry as Root Cause 1, but it also implies the fix
location: **this handler, not the Rust backend, is where an accurate size can be computed**,
because this is the one place in the whole pipeline that has simultaneous access to (a) the
direction the ghost computed and (b) the target's real, current `size`.

---

## Recommended fix direction ("ghost becomes the pane")

Two independent changes, addressing Root Cause 1 and Root Cause 2 respectively. Both are
frontend-only (no backend/IPC schema change required beyond optionally simplifying what the
backend sends).

### A. Compute exact size from `targetNode.size`, in `handleBackendAction`, not in Rust

Replace the backend's absolute `nodesize` guess with a **target-relative fraction** the ghost
already encodes as a constant (`app-init.ts:150-174`: inner = 0.5, outer = 0.2 of the leaf).
Send that fraction (or just the direction, since the frontend already maps direction → fraction
for the ghost rect) instead of an absolute integer, and derive the new node's size in
`layoutPersistence.ts` as:

```
desiredFraction = 0.5 for inner directions, 0.2 for outer directions   // matches rectForDirection exactly
newNode.size = desiredFraction * targetNode.size / (1 - desiredFraction)
```

This guarantees `newNode.size / (targetNode.size + newNode.size) === desiredFraction` for
*any* current `targetNode.size`, not just the default — closing Root Cause 1 exactly, using data
that already exists on the client. (`nodesize` in `LayoutActionData` would need to become a
float or the fraction sent as a separate field — `agentmux-srv`'s `LayoutActionData.nodesize`
is currently `Option<u32>`; either widen it or add `nodesizefraction: Option<f64>` and prefer it
when present.)

### B. Make the splice branch carve from the target instead of diluting the pool

In the splice branch of `splitHorizontal`/`splitVertical` (`layoutTree.ts:471-479` /
`515-523`), reduce `targetNode.size` by the portion being handed to `newNode` instead of leaving
it untouched:

```ts
if (parent && parent.flexDirection === FlexDirection.Row) {
    const carved = newNode.size ?? DefaultNodeSize;
    targetNode.size = Math.max(targetNode.size - carved, MIN_NODE_SIZE);
    parent.children.splice(insertIndex, 0, newNode);
}
```

Combined with Fix A (`newNode.size` already computed as an exact fraction of `targetNode.size`),
this makes `totalChildrenSize` for the row/column **unchanged** before and after the splice —
`pixelToSizeRatio` stays the same, so **every sibling other than the target keeps its exact
current pixel size**. Only the target's slot visually splits into the ghost's two sub-rects.
This is what turns "ghost previews a sub-rect, landing dilutes everyone" into "ghost previews a
sub-rect, landing carves exactly that sub-rect out of the target and nothing else moves" — i.e.
the ghost rect and the final rendered rect become the same rect, by construction, for both the
splice and wrap branches.

### Suggested validation

- Unit test `updateTreeHelper`/`layoutTree` pair: given a 3-pane row (sizes e.g. `[10, 10, 10]`),
  simulate a `Right` (inner, 0.5) split on the middle pane → assert the two *other* panes' pixel
  width is unchanged after the split, and the middle pane's original footprint is now occupied by
  two panes at exactly 50/50.
  the same test with `OuterLeft` (outer, 0.2) → assert new pane occupies exactly 20% of the
  target's original footprint and outer siblings are untouched.
- Manual repro: resize a pane to a non-default size, then redock a floater onto it from each of
  the 9 zones; the landing rect should pixel-match the last ghost rect shown before release.

---

## Files referenced (all read directly, not inferred)

| File | Role |
|---|---|
| `frontend/app-init.ts:113-273` | Ghost geometry + push-then-store hand-off (Phase 4b, already shipped) |
| `frontend/app/workspace/floating-pane-workspace.tsx:491-577, 889-1006` | Drop-time ghost query + `RedockFloatingPane` call |
| `agentmux-srv/src/server/service/layout_helpers.rs:94-148` | Server-side direction→split-action mapping + hardcoded `nodesize` approximation (Root Cause 1) |
| `frontend/layout/lib/layoutPersistence.ts:105-274` | Frontend action handler — has `targetNode` with live size, currently unused for sizing (fix location for A) |
| `frontend/layout/lib/layoutTree.ts:462-544` | `splitHorizontal`/`splitVertical` — wrap vs. splice branches (Root Cause 2, fix location for B) |
| `frontend/layout/lib/layoutGeometry.ts:88-197` | `updateTreeHelper` — the shared-pool flex-ratio rendering model that makes splice-branch dilution mechanical, not incidental |
| `frontend/layout/lib/layoutNode.ts:16-34` | `newLayoutNode` — confirms `size` defaults to flat `DefaultNodeSize=10` when omitted |

## Related prior docs

| Path | Relevance |
|---|---|
| `docs/analysis/ANALYSIS_FLOATING_PANE_REDOCK_SIZE_2026_06_23.md` | Diagnosed the process-boundary direction-loss gap and designed the push-then-store fix this doc confirms is shipped; flagged (but did not solve) the size-approximation gap this doc addresses as Root Cause 1. |
| `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` | Broader tear-off/redock/browser-pane architecture reference; predates Phase 4b. |
| `docs/specs/SPEC_FLOATING_PANE_REDOCK_2026-05-27.md` | Original Phase 4 spec (ghost + cross-window highlight goal). |
| `docs/specs/SPEC_FLOATING_PANE_REDOCK_PHASE_4A_SCOPING_2026-05-27.md` | MVP scoping that deferred direction-aware landing to "Phase 4b" (now shipped; size accuracy still open — this doc's Root Cause 1/2). |
| `docs/specs/SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md` | Argues point-fixes on this subsystem keep recurring; this doc's Fix B (carve-from-target) is a structural rather than symptomatic change, in that spirit. |
| `docs/specs/SPEC_PANE_COLOR_PANEL_TOPLEVEL_2026_07_01.md` | Checked — unrelated (pane-header color popover anchoring, not drag/dock). |
