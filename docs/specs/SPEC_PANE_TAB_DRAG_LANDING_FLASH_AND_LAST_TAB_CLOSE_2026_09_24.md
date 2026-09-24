# SPEC: Pane Tab drag — Window-Tab-style landing flash on the destination, moving a pane's last tab closes that pane, and removing `pane:tabstrip = "multi-only"`

**Date:** 2026-09-24
**Status:** implemented — §8 (remove `pane:tabstrip = "multi-only"`) in PR
#3692, §3 (flash + landing bounce) in PR #3694, §4 (last-tab close) in PR
#3698. Design decisions in §6 are confirmed by the repo owner. Not yet
verified in a running app (`task dev`); see §5's manual checks.
**Author:** Camper
**Trigger:** direct repo-owner request after live use of the shipped Pane Tab
drag (PRs #3441, #3444, #3447, #3449): "it works well, but we want some
tweaks — (1) when dragging over the destination, we want a flashing effect
similar to the one window tabs have during landing; (2) if a pane tab is the
last in its pane and is dragged away, the pane it was in closes when it lands
in the new pane." Follow-up in the same session: flash the header except the
tabs, as close to the Window Tab look as possible, and (3) remove the
`"multi-only"` setting, because a pane header should always show its tabs
(§8).
**Amends:** `docs/specs/SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` —
§3.4 (the "one-shot hover-flash" was specced but shipped as a static
outline), §4.1 step 3 and `moveMemberAcrossStacks`' doc (the "refuse to move
a pane's only member" rule is reversed for the cross-pane case), and §7 open
question 2 (answered: flash yes; still no dwell).
**Supersedes:** `docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`
§4.1's `pane:tabstrip` paragraph and §7 resolution 1 (the setting is
removed, §8).

---

## 0. Terminology

Same as the parent spec §0: **Pane** = split-tree leaf; **Pane Tab** = one
`block_stack` member (a pill in the Pane's header); **Window Tab** = the outer
tab bar. "Destination header" = the `[data-role="block-header"]` row of the
Pane being dropped onto (`foreignDropRootFor`, `PaneTabStrip.tsx:110`).

---

## 1. Grounding — what's on `main` today (b96df30cb, verified in code)

### 1.1 Destination feedback is a static 1px outline

- `PaneTabStrip.tsx:265-311` registers one `dropTargetForElements` on the
  destination header (`canDrop`: `type === paneTabItemType &&
  sourceNodeId !== paneKey`). `onDragEnter`/`onDragLeave` flip a
  `foreignHover` signal, which toggles `.pane-header--foreign-hover` on the
  header element imperatively.
- `PaneTabStrip.scss:162-165`: `.pane-header--foreign-hover { outline: 1px
  solid var(--accent-color); outline-offset: -1px; }`. No animation at all.
  The parent spec's §3.4 recommended "a one-shot hover-flash class ...
  reusing the visual language of `.tile-drop-hover` / the strobe invert",
  and the Phase 4 commit message describes a "dwell-free hover flash." What
  actually shipped is the outline only.

### 1.2 What the Window Tab "landing" flash actually is

`frontend/app/tab/tabbar.scss:135-166, 205-231`. It runs when a whole
**Pane** is dragged over a Window Tab (`droppable-tab.tsx:220-275`,
`hoveredDropTabId` → `.tile-drop-hover` on `.tab-drop-wrapper`):

```scss
&.tile-drop-hover .tab {
    outline: 2px solid var(--accent-color);
    outline-offset: -2px;
    border-radius: 0;
    animation:
        tab-drop-invert-strobe 400ms steps(1, end) 2,       // 2 hard invert flashes
        tab-drop-pulse 0.8s ease-in-out infinite alternate; // then steady outline pulse
    @media (prefers-reduced-motion: reduce) {
        animation: tab-drop-pulse 0.8s ease-in-out infinite alternate;
    }
}
@keyframes tab-drop-invert-strobe { 0% { filter: invert(1) } 50% { filter: invert(0) } 100% { filter: invert(0) } }
@keyframes tab-drop-pulse { from { outline-color: var(--accent-color) } to { outline-color: rgba(34,197,94,.5) } }
```

Three constraints come with it, all recorded in that file's comments:
- **WCAG 2.3.1:** 2 flashes over 800ms (2.5 Hz). The original 20 Hz
  version was rejected in review (PR #2105, P1). Any reuse must keep this
  rate or go slower.
- **The reduced-motion opt-out on the strobe is intentional and stays.**
  App-wide reduced-motion support was removed on 2026-07-11
  (`mixins.scss` `respect-reduced-motion`, `global.ts`
  `prefersReducedMotionAtom` is hard `false`). This one media query was kept
  because a seizure-trigger strobe is a different category from cosmetic
  motion.
- **Retriggering:** the class is removed on leave and re-added on the next
  enter, so the strobe replays on every new hover entry.

The separate `tab-bounce` (`.tab-bouncing`, `tab-reorder.ts:240`) is the
post-drop settle on a *reordered* Window Tab, not the hover flash. See §3.4
for whether to borrow it.

### 1.3 Moving a pane's only tab is refused in three places

| Layer | Where | Behavior |
|---|---|---|
| Frontend pure fn | `stackMembers.ts:62-73` `moveMemberAcrossStacks` | `sourceMembers.length <= 1` → `return false`, neither side touched |
| Frontend mutator | `layoutStack.ts:234-279` `moveBlockInStack` | on `false`: `console.error("... or is that pane's only member")`, no RPC |
| Backend pure fn | `agentmux-srv/src/backend/layout/mod.rs:327-329` `move_stack_member` | `source_members.len() <= 1` → `false` ("that's closing the pane") |
| Backend reducer | `reducer/layout.rs:608-621` `handle_layout_stack_move` | `false` → `Event::Error` |
| Other-window mirror | `layoutPersistence.ts:316-347` (`StackMove` case) | calls `moveMemberAcrossStacks`, so it would refuse too |

The destination's `canDrop` does **not** check this. A lone tab dragged onto
another pane's header therefore shows the hover highlight, accepts the drop,
and then silently does nothing apart from a console error. The user is told
the drop is valid and then it doesn't happen. Tweak 2 fixes this.

### 1.4 Removing a pane without destroying its block has precedent, and so does getting it wrong

- `closeNode` (`layoutMagnify.ts:42-93`) is the **close** path:
  `beforeNodeDelete` confirmation ("something in this pane is still
  running"), un-magnify, `treeReducer(DeleteNode)`, `clearLeafRevealGate`,
  then **`onNodeDelete` → `DeleteBlock`**. This destroys the block, e.g. it
  kills a live agent session.
- `layoutPersistence.ts:232-281` (`DeleteNode` pending action) is the
  **move** path. It calls `treeReducer(DeleteNode)` directly and never
  `onNodeDelete`. Its comment records incident **R1 (#1681)**: using
  `closeNode()` for a move "ran onNodeDelete → DeleteBlock and destroyed the
  just-moved block."
- Backend `handle_layout_delete_node` (`reducer/layout.rs:314+`) deletes a
  leaf with `delete_node` (which collapses a single-child parent by rewriting
  its id to the promoted child's, `mod.rs:957-982`), then reconciles any
  `focused_node_id`/`magnified_node_id` that no longer resolves.
  `stack_tree_edit` (`reducer/layout.rs:508-536`) emits a **tree-only**
  `LayoutTreeReplaced` (`slices: None`) and does no such reconciliation. That
  is fine today because no stack edit removes a leaf.

---

## 2. Non-goals

Same discipline as the parent spec §2. Specifically:
- No change to the whole-Pane drag, the Window Tab bar, `tabbar-dnd.ts`,
  `droppable-tab.tsx`, `TileLayout.*`, `crossTabDrag.ts`. `tabbar.scss` is
  only *read from*: its keyframes get promoted to a shared partial (§3.2)
  without changing their values.
- No dwell / spring-switch on the pane header. The parent spec §3.4's
  reasoning still holds: the destination pane is already visible, so there's
  nothing to reveal. Tweak 1 is a visual change only, with no change in
  timing semantics.
- Not making the `"multi-only"` plain-title header support tab drag. §8
  removes that mode instead. Not drop-on-content splits
  (parent §3.3) or tear-off (parent §3.5).
- Not position-aware header drops. A foreign tab still always appends
  (parent §7 Q3).

---

## 3. Tweak 1 — Window-Tab-style flash on the destination header

### 3.1 Behavior

**Design rule (repo owner, 2026-09-24):** match the Window Tab look as
closely as possible, so dropping onto a pane reads the same as dropping onto
a Window Tab. Same keyframes, timing, outline width and colors; no
pane-specific variant. The one deliberate difference: **the tab pills don't
flash.** Only the rest of the header row does (§3.2).

When a Pane Tab from a **different** pane enters a pane's header drop zone:

1. **Immediately:** two hard invert flashes over 800ms (the exact
   `tab-drop-invert-strobe 400ms steps(1,end) 2` from the Window Tab bar) on
   the **header row minus its tab pills**: the header background (including
   the pane color or tail color) plus the non-tab chrome in it
   (ConnectionButton, header text, the minimize/magnify/close icons). The
   pills and "+" stay exactly as they are.
2. The same 2px accent outline `outline-offset: -2px` the Window Tab gets,
   drawn inside the whole row, **pulsing** accent ↔ green for as long as the
   hover lasts (`tab-drop-pulse 0.8s ease-in-out infinite alternate`).
3. **On leave:** everything stops at once. **On re-entry:** the strobe replays
   from the start.
4. **On drop:** the hover styling clears (existing `setForeignHover(false)`),
   and the arrived pill plays the Window Tab landing bounce (§3.4).
5. `prefers-reduced-motion: reduce` → skip the strobe and keep the pulse,
   exactly as the Window Tab bar does.

Same-pane reorder feedback (`--dragging`, `--drop-before/after` insertion
line) is unchanged. It has its own grammar and doesn't need a flash.

### 3.2 Implementation

**Share the keyframes rather than copying them.** Move `tab-drop-pulse`,
`tab-drop-invert-strobe` and `tab-bounce` (§3.4) from `tabbar.scss` into a small partial (e.g.
`frontend/app/drop-feedback.scss`, or a `@mixin drop-target-flash($width)` in
`mixins.scss`). Import that partial from both `tabbar.scss` and
`PaneTabStrip.scss`. Keep names and values byte-identical so the Window Tab
bar renders exactly as before. That way the two can't drift, and a future
change to the WCAG-sensitive rate lands in both places at once.

**Replace the static rule** at `PaneTabStrip.scss:162-165`:

```scss
.pane-header--foreign-hover {
    outline: 2px solid var(--accent-color);
    outline-offset: -2px;          // stays inside the row: no clipping by the
                                   // pane's rounded corners, no overlap with a neighbour
    animation:
        tab-drop-invert-strobe 400ms steps(1, end) 2,
        tab-drop-pulse 0.8s ease-in-out infinite alternate;
    @media (prefers-reduced-motion: reduce) {
        animation: tab-drop-pulse 0.8s ease-in-out infinite alternate;
    }

    // Exclude the tabs: run the SAME strobe on the pill-strip wrapper.
    // invert(1) is its own inverse (each channel c → 1-c → c), so the header's
    // invert applied on top of the wrapper's invert gives back the pills'
    // exact original pixels. The two animations start in the same style
    // recalc (one class toggle) and use steps(1, end), so they flip on the
    // same frame.
    .block-frame-default-header-tabstrip {
        animation: tab-drop-invert-strobe 400ms steps(1, end) 2;
        @media (prefers-reduced-motion: reduce) {
            animation: none;
        }
    }
}
```

`.block-frame-default-header-tabstrip` is `BlockFrame_Header`'s own wrapper
around `leadingTabStrip` (`blockframe.tsx:747`), so it holds exactly the
pills + "+" + the §3.6 drag-handle spacer and nothing else. The outline and
pulse stay on the header, so they trace the full row, tabs included, just
like on a Window Tab.

**Why counter-invert rather than a background overlay:** the header's
background is an inline `background-color` computed from the pane color /
tail override / non-agent default (`blockframe.tsx:707-719`). An overlay
that re-paints "the inverted background" would have to recompute that color
per case and would miss the non-tab chrome. A `::before` with
`backdrop-filter: invert(1)` stacked under the tab strip also works in
principle, but it depends on backdrop-root and stacking-context details in a
row that already carries `isolation`/`box-shadow` rules. The
double-invert is one selector, reuses the existing keyframe unchanged, and
is exact.

**Variant, if the non-tab chrome should stay put too (background only):**
counter-invert every direct child instead (`> *` instead of
`.block-frame-default-header-tabstrip`). The strobe then only touches the
header's own background. Not the default: the header's text color is picked
for contrast against the *un-inverted* background
(`pickReadableTextColor`, `blockframe.tsx:710`). If the background inverts
and the icons don't, a light icon on an inverted-to-light background
disappears for the 200ms flash. Inverting the chrome along with the
background keeps that contrast, the same way a Window Tab's label inverts
with its tab.

Apply the same treatment to `.pane-tab-strip--foreign-hover`
(`PaneTabStrip.scss:147-149`, the non-header fallback, currently no consumer)
so the two variants stay consistent. That variant uses `box-shadow`, which
`tab-drop-pulse` doesn't animate (it animates `outline-color`), so switch it
to `outline` too.

**No TS change is needed for the hover part.** The class is already added on
`onDragEnter` and removed on `onDragLeave`/`onDrop`
(`PaneTabStrip.tsx:295-308`). Removing and re-adding a class across separate
events restarts a CSS animation, which gives the per-entry replay for free.
Confirm this in `task dev` (§5). If CEF coalesces a leave+enter within one
frame (e.g. the pointer crosses a sub-element boundary), the strobe won't
replay. That's acceptable, and better than forcing a reflow.

### 3.3 Things to check during implementation

- **Invert area.** A Window Tab is ~100-230px wide. The inverted part of a
  pane header (the row minus its pills) can be most of a window's width, so
  the flash is more prominent here. WCAG 2.3.1's rate limit (≤3/s) is met
  regardless of area. This is the look the repo owner asked for; confirm it
  in `task dev`, not a design question.
- **Double-invert exactness.** It's pixel-exact in theory. Check in
  `task dev` for a visible seam or a one-frame desync at the pill-strip
  edge, e.g. if CEF promotes the header and the wrapper to separate
  compositor layers and starts their animations a frame apart. If a desync
  shows, fall back to the `backdrop-filter` overlay approach (§3.2) rather
  than dropping the exclusion.
- **`filter` creates a containing block** for `position: fixed` descendants
  for as long as it's applied. The pill tooltips are Portal-based
  (`PaneTabStrip.tsx:633-637`), and the header-text tooltip is an
  `AnchoredTooltip` (`blockframe.tsx:784-788`, verify it portals out).
  Verify that nothing inside the header (e.g. ConnectionButton's dropdown)
  is `position: fixed` and rendered in place. Hover-drag means no menu should
  be open anyway.
- **Existing `box-shadow` on the header** (block.scss, has-summary case) is
  untouched. `outline` is a separate property, which is why the shipped rule
  chose it. Keep that.
- **Header tint.** `invert(1)` on a pane-colored header gives that color's
  complement, the same as a colored Window Tab today. No special case.

### 3.4 Landing bounce on the arrived pill — the Window Tab's own `tab-bounce`

For consistency with the Window Tab bar, the landing effect is the **same
spring bounce** a Window Tab plays when it lands (`tab-bounce 300ms
cubic-bezier(0.34, 1.56, 0.64, 1)`, `transform-origin: center bottom`,
`tabbar.scss:174-178, 216-222`), not a pane-specific effect. (An earlier
revision of this spec suggested the activity flash here. That was replaced
under the "as similar to Window Tabs as possible" rule, and it also keeps the
activity flash meaning only "a tool ran".)

- **CSS:** `tab-bounce` moves into the same shared partial as the two hover
  keyframes (§3.2). `PaneTabStrip.scss` adds
  `.pane-tab--landing { animation: tab-bounce 300ms cubic-bezier(0.34, 1.56, 0.64, 1) forwards; transform-origin: center bottom; }`.
  Put it on `.pane-tab` itself. The Tooltip wrapper `.pane-tab-tip` carries
  the flex sizing, so a `transform` on the inner pill doesn't disturb layout.
- **Timing:** class on for 400ms, then off. That's the same clear-timeout
  the Window Tab bar uses (`tab-reorder.ts:240-241`).
- **Mechanism (as implemented):** a module-level Solid signal
  `landedTab` (`{ id, paneKey }`) in `PaneTabStrip.tsx`, set by
  `markLanded(blockId, destinationPaneKey)` and cleared after
  `LANDING_BOUNCE_MS` (400). Every `PaneTabStripItem` binds
  `.pane-tab--landing` to "same id AND same `paneKey`". The pane is part of
  the identity because one block can have pills in several strips (agent
  fork lineages show other panes' blocks as `extraTabs`), and only the pill
  in the pane it landed in should bounce (Codex P2 on #3694). This works both for a
  pill that mounts fresh in the destination strip (cross-pane) and for a
  pill that `<For>` just moved in place (same-pane reorder). An earlier
  draft used a one-shot `onMount` check, which can't see a reorder, since
  the pill isn't remounted. `markLanded` runs only when the move was actually
  applied: `moveBlockInStack` now returns a boolean, and `onReorder` /
  `onReceiveForeignTab` pass it through (`false` = refused, no bounce).
- **Scope:** cross-pane drops **and** same-pane reorders, for parity with the
  Window Tab bar, which bounces a reordered tab (§6: resolved yes). The
  bouncing pill is the one that moved, not the one it was dropped on.

---

## 4. Tweak 2 — dragging a pane's last tab to another pane closes the source pane

### 4.1 Behavior

Dropping a Pane Tab from pane **S** onto the header of a different pane
**D**, when that tab is **S's only member**:

- The tab joins D's stack at the end, active (same as today's multi-member
  case).
- Pane **S** is removed from the split tree, and its siblings reflow to fill
  the space, exactly as if S had been closed.
- **The moved block is NOT destroyed.** Its agent session, terminal
  process, editor buffer, etc. carry on, now in D. This is a *move*.
- **No close confirmation.** `beforeNodeDelete`'s "something is still
  running" prompt doesn't apply: nothing is being stopped.
- Focus goes to **D**, since the tab the user just dragged is now visible
  there. The multi-member cross-pane move didn't explicitly move focus
  before this change either; it now follows the same rule, for consistency
  (§4.2 step 4). Same-pane reorders don't touch focus.
- If S was magnified, it's un-magnified first (the same as `closeNode`
  does).

Unchanged: dragging a pane's only tab onto **its own** header is still
rejected by `canDrop` (`sourceNodeId === paneKey`). The whole-Pane drag is
untouched.

### 4.2 Frontend

**`stackMembers.ts`:** the pure primitive stays pure and leaf-local. Change
`moveMemberAcrossStacks` to **stop refusing** when the source has one
member, and return which case happened so the caller knows to remove the
leaf:

```ts
/** "moved" — source keeps ≥1 member; "emptied" — blockId was the source's
 *  only member: it's now in the target and the SOURCE LEAF MUST BE REMOVED
 *  by the caller (this fn can't: it only sees leaf data, not the tree).
 *  false — blockId isn't a member of sourceData; nothing touched. */
export function moveMemberAcrossStacks(src, dst, blockId, activate): "moved" | "emptied" | false
```

In the `"emptied"` case, don't call `removeMemberFromStack` (it refuses a
lone member). Leave `sourceData` as it is, since the leaf is about to be
deleted, and just `addMemberToStack(dst, blockId, activate)`. Update the doc
comment: the "closing the pane is a different operation" rationale is
replaced by a pointer to this spec.

**One shared helper for the tree step**, used by both the local mutator and
the other-window mirror so they can't diverge. As implemented it lives in
`layoutMagnify.ts`, next to `closeNode` (which both importers can already
reach without an import cycle), and un-magnifies with `setState=false`
because the caller commits:

```ts
/** Remove a leaf emptied by a cross-pane tab move. A MOVE, not a close:
 *  treeReducer(DeleteNode) directly — never closeNode(), which runs
 *  beforeNodeDelete (wrong prompt) and onNodeDelete → DeleteBlock (destroys
 *  the block that was just moved — incident R1 / #1681, see
 *  layoutPersistence.ts's DeleteNode case). */
function removeLeafEmptiedByMove(model: LayoutModel, nodeId: string): void {
    if (nodeId === model.magnifiedNodeId) magnifyNodeToggle(model, nodeId, false);
    model.treeReducer({ type: LayoutTreeActionType.DeleteNode, nodeId } as LayoutTreeDeleteNodeAction, false);
    clearLeafRevealGate(nodeId);
}
```

**`moveBlockInStack`** (`layoutStack.ts:234-279`), cross-leaf branch:

1. `const result = moveMemberAcrossStacks(src.data, dst.data, blockId, activate)`.
2. `false` → existing `console.error` + return. The message no longer
   mentions "only member".
3. `"emptied"` → `removeLeafEmptiedByMove(model, sourceNode.id)`.
4. Focus D: resolve D **by blockId after** the delete
   (`findNodeByBlockId(root, targetBlockId)`), **not** by the `targetNode`
   reference held from before. `deleteNode` → `removeChild` → balance can
   collapse D's parent and re-home D's id/data (the same hazard
   `closeBlockInStack`'s comment cites for `balanceNode`, reagent P1 on
   #3422). Then set focus the same way the rest of `layoutModel` does. Use
   the existing focus action/helper, not a raw `focusedNodeId` write.
5. Existing `updateTree(false)` → `localTreeStateAtom` → `persistToBackend()`
   → `pane.moveTab` RPC. **One** persist and **one** RPC for the whole
   operation, never a separate close RPC.

**Commit after pragmatic-dnd has finished the drop.** In the emptied case,
the drop now unmounts the **whole source pane**: its header (the
whole-Pane `draggable()` registration in `TileLayout.core.tsx`), its strip,
and the dragged pill, which is the live drag **source**. Today's
multi-member move already unmounts the dragged pill mid-drop and evidently
survives. Unmounting a registered `draggable` element and an ancestor
`draggable` during pragmatic-dnd's synchronous drop dispatch is the exact
class of bug `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` catalogs
(teardown relying on events a detached source may never get). Apply the
parent spec §5 "commit-after-teardown" principle narrowly: in
`PaneTabStrip`'s header `onDrop`, defer `props.onReceiveForeignTab(blockId)`
to the next task (`setTimeout(..., 0)`) or next frame. Defer only the
commit; clear the hover styling synchronously as it does now. Do this for
both the moved and emptied cases so there's one code path. If testing shows
the synchronous version is fine in the multi-member case, still defer: the
cost is one frame.

**Other windows/tabs mirroring the move** (`layoutPersistence.ts:316-347`,
`StackMove`): switch to the new return value. On `"emptied"`, call the same
`removeLeafEmptiedByMove(model, leaf.id)` from `layoutMagnify.ts`. This path must also skip
`closeNode`/`onNodeDelete`. The block was moved, not closed.

**`canDrop`: no change.** It already accepts the lone-tab case. The change
is that the drop now does what the highlight promised.

### 4.3 Backend

**`move_stack_member`** (`backend/layout/mod.rs:296-334`), cross-leaf branch,
replacing the `source_members.len() <= 1 → false` refusal:

```rust
// Validate target exists (already done above, before any mutation).
if source_members.len() <= 1 {
    // The source pane's only tab: moving it empties the pane, so the pane goes.
    // Place it FIRST (by block id — survives the collapse below), then delete
    // the source leaf by node id. Both inside this one tree edit, so no
    // intermediate tree (block in two leaves, or in none) is ever observable.
    let source_node_id = source_leaf.id.clone();
    if !push_stack_member_at(tree, target_block_id, block_id, position, activate) { return false; }
    return delete_node(tree, &source_node_id).is_ok();
}
```

Notes:
- `push_stack_member_at` finds D by `target_block_id`, and
  `find_leaf_containing_block` has already confirmed D ≠ S, so the push can't
  land in S. After the push, `block_id` is briefly in both S and D. The
  delete removes S straight after, inside the same `&mut` borrow, before any
  event is built.
- `source_leaf` is a shared borrow of `tree`. Clone its `id` (and
  `source_members`) **before** the mutable calls. The current code already
  clones `source_members` via `leaf_members`, and `id` needs the same
  treatment or it won't compile.
- **Root case:** S can't be the root, since D is a different leaf in the same
  tree, so the root has ≥2 descendants. As implemented, an explicit
  `if tree.id == source_node_id { return false; }` runs *before* any
  mutation instead of a `debug_assert!`. `delete_node` silently no-ops on
  the root id, so without the guard a future caller breaking the assumption
  would leave the block duplicated instead of being refused.
- The delete can't fail after the push: the source leaf was just found by
  walking this same tree, and `push_stack_member_at` only edits leaf data,
  never node ids.

**`handle_layout_stack_move`** (`reducer/layout.rs`): the tree-only
`stack_tree_edit` isn't enough once a leaf can disappear, because focus and
magnify can point at S, or at an id rewritten by the single-child collapse.

**As implemented** (this replaces an earlier draft that proposed emitting
`LayoutTreeReplaced` *slices* for the reconciled fields; that's wrong, since
slices carry frontend-push semantics, where an absent
`pending_backend_actions` means "clear the queue". Sending them from here
would wipe the backend-to-frontend action queue, including the `stackmove`
this very RPC queues):
- A new pure `source_leaf_emptied_by_move(tree, block_id, target_block_id)`
  tells the reducer *before* the edit whether this move will delete a leaf
  (and which).
- Every other move keeps the existing tree-only `stack_tree_edit` path,
  byte-for-byte.
- The emptied case applies `move_stack_member` to the live tree, then
  reconciles focus/magnify with `clear_dangling_focus_magnify`, which is
  factored out of `handle_layout_delete_node` so both share one
  implementation. It emits a **`LayoutNodeDeleted`** for the source leaf,
  with the full post-move tree in `new_tree` (`tree_cleared: false`) and the
  reconciled `was_focused`/`was_magnified`. That's exactly what a node
  delete emits, so persistence (`persist_subscriber`: writes `new_tree`) and
  every consumer already handle it. Nothing reads a `LayoutNodeDeleted` as
  "delete the block".

The `not_found` error string drops "or is its pane's only tab", and both doc
comments (`move_stack_member`, `handle_layout_stack_move`) are updated.

**Block lifecycle (backend):** unchanged, and this is intentional. No
`DeleteBlock`, no saga. The block's `tab_id` is the same (both panes are in
the same Window Tab), so no `TearOffBlock`-style reparenting is needed.
`pane.moveTab` (`app_api/mod.rs:390-452`) needs no change beyond its
comment. It still queues one `stackmove` for other windows, which they now
apply with the emptied-leaf removal (§4.2).

**`validate_layout_invariants`** (called from `stack_tree_edit`) must stay
clean after the emptied move. §5 has a test for it.

### 4.4 Interactions to confirm, not assume

- **`pane:tabstrip = "multi-only"`:** a lone-tab pane renders **no pill
  strip** (`PaneHeaderTabStrip.tsx:62-64,128`), so its only tab has no
  `draggable()`, and the pane registers no foreign-drop target either. In
  that mode, tweak 2 can't be triggered, and a lone-tab pane can't *receive*
  a tab. Under the default `"always"`, both work. §8 removes the setting,
  and with it this case. Once PR C lands, every pane header has its tabs.
- **Agent pane with a live turn in S:** a move doesn't stop anything, so
  there's no prompt. Confirm in `task dev` that the agent's stream keeps
  rendering in D after the move, i.e. the same block, not a remount that
  loses in-flight state. The multi-member move already re-parents a live
  block, so this is the same path, but it's the case that matters most.
- **Floating/ephemeral source:** `findNodeByBlockId` only searches the tree,
  so an ephemeral pane's tab can't resolve as a source and is refused as it
  is today. Out of scope.

---

## 5. Test plan

**Rust (`backend/layout/tests.rs`)**, replacing
`move_stack_member_refuses_to_strip_the_source_leaf_to_zero_members`:
- `move_stack_member_last_member_moves_and_removes_the_source_leaf`: S gone,
  block in D's stack, active if requested, D's other members intact.
- `..._collapses_a_single_child_parent`: S and D are the only two children
  of a split. After the move, the root node has absorbed D (`delete_recursive`
  copies the sole child's id/data up into the parent), and the block is in
  its stack. This is why placement is by block id, not by node reference.
- `..._last_member_into_a_single_block_destination_promotes_it_to_a_stack`.
- `..._last_member_with_missing_target_leaves_tree_untouched`: no partial
  mutation.
- `validate_layout_invariants(&tree).is_empty()` after each of the above.

**Rust (reducer):** `handle_layout_stack_move` with focus/magnify on S →
reconciled (cleared or remapped) in the emitted slices. With focus
elsewhere → untouched.

**Rust (`app_api` `pane.moveTab`):** a lone-tab move succeeds and queues one
`stackmove`.

**vitest (`stackMembers.test.ts`):** the existing "refuses to move the only
member" test becomes "returns `"emptied"`, target gains it, source data
untouched". The `"moved"` and `false` cases stay.

**vitest (`layoutStack.test.ts`):** `moveBlockInStack` lone-tab case:
- S's node is gone from `treeState`, and the block is in D's stack, active.
- **`onNodeDelete` is never called** and **`beforeNodeDelete` is never
  called**. Spy on both. This is the R1 regression guard and the single most
  important test in this spec.
- `pane.moveTab` fires exactly once. No close/delete RPC.
- Magnified S is un-magnified. D is focused afterward.

**vitest (`layoutPersistence`):** a queued `stackmove` whose source is a
lone-member leaf removes that leaf and never calls `onNodeDelete`.

**vitest (`PaneTabStrip.dropTarget.test.tsx`):** `onDragEnter` adds
`.pane-header--foreign-hover`, `onDragLeave` removes it (existing). New:
`onDrop` defers `onReceiveForeignTab` (not called synchronously, called
after timers flush), and the hover class is cleared synchronously. A pill
mounting with a matching pending-landing id gets `.pane-tab--landing`, which
is removed after 400ms (fake timers). A non-matching or expired id doesn't
get it.

**CSS:** jsdom can't verify animation. Add an assertion that the Window Tab
rule still references the same keyframe names after the partial is
extracted (a grep-style test or a snapshot of the compiled CSS, whichever
the repo already uses). Otherwise this is a `task dev` check.

**Manual (`task dev`), every item:**
1. Drag a tab from a 3-tab pane over another pane's header: the header
   (background, icons, text) inverts twice while its **pills don't change**,
   then a pulsing outline. Put it side by side with dragging a whole pane
   over a Window Tab: timing, outline and colors should match. Leave and
   re-enter: it replays. Drop: the styling clears, and the tab lands active
   with the Window Tab bounce. Repeat on a colored pane, an uncolored agent
   pane, and a terminal (non-agent default header color).
2. Drag the **only** tab of pane S onto pane D's header: S disappears, its
   siblings reflow, the tab is active in D, and **an agent tab keeps its live
   conversation/stream.**
3. Same as 2 with S magnified, and with S being one of only two panes in the
   Window Tab.
4. The Window Tab bar's pane-drag strobe looks identical to before (keyframe
   extraction regression check).
5. The whole-Pane header drag still works (the parent spec's standing
   regression check).
6. With two AgentMux windows showing the same Window Tab, repeat 2: the
   other window mirrors it with no dead/empty pane left behind.

**Cross-cutting:** `npx tsc -p tsconfig.citypecheck.json --noEmit`;
`npx vitest run frontend/app/element frontend/app/tab frontend/layout`;
`cargo test -p agentmux-srv layout`; `cargo check --workspace --tests`.

---

## 6. Decisions and open questions

**Resolved by the repo owner, 2026-09-24:**
- **What flashes:** the pane header *except the tabs*, done by
  counter-inverting the pill strip (§3.2). "Background only" is available as
  a one-selector variant but isn't the default, for the contrast reason in
  §3.2.
- **How it looks:** as close to the Window Tab drop feedback as possible.
  Same keyframes, timing, 2px outline, pulse colors, reduced-motion
  handling, and the same `tab-bounce` on landing (§3.4).

- **`pane:tabstrip = "multi-only"`:** removed. A pane header always shows
  its tabs (§8).

- **Bounce on same-pane reorder too?** (§3.4) Yes, for parity with the
  Window Tab bar, which bounces a reordered tab. Shipped in PR A.

**Still open:** nothing.

---

## 7. Phasing

Three small PRs. **Recommended order: C, then A and B (in either order).**
They don't depend on each other for correctness. Landing C first means A and
B are built and tested against one header shape only, with no `"multi-only"`
edge case to handle or leave half-supported.

- **PR C — remove `"multi-only"`** (§8): frontend setting read +
  `addButtonOnly`/`trailingAddButton` removal, and the key removed from
  types, schema and `SettingsType` (the `extra` catch-all keeps old files
  loading). Plus doc updates and §8.5 tests. Release note: "`pane:tabstrip` has been removed; pane headers always
  show their tabs."
- **PR A — flash (tweak 1):** SCSS keyframe extraction (strobe, pulse,
  bounce) + the `.pane-header--foreign-hover` rule with the pill-strip
  counter-invert + the §3.4 landing bounce + the deferred-commit change in
  `PaneTabStrip`'s `onDrop`. Frontend only.
- **PR B — last-tab close (tweak 2):** backend `move_stack_member` + reducer
  reconciliation + frontend `moveMemberAcrossStacks`/`moveBlockInStack`/
  `layoutPersistence` + tests. If PR A hasn't landed yet, PR B carries the
  deferred-commit change itself, because PR B is the one that makes it
  necessary.

**Branch policy (repo owner, 2026-09-24):** unlike the parent spec, these
PRs **merge on approval** (ReAgent review + green CI). The repo owner asked
for that explicitly for this work.

---

## 8. Remove `pane:tabstrip = "multi-only"` — a pane header always shows its tabs

**Decided by the repo owner, 2026-09-24:** remove the setting. A pane's header
always shows its tabs, including when there's only one. A pane is a container
of tabs, and one tab is just the smallest case. A header that changes form
at two tabs is confusing: users learn that one-tab panes are different and
can't drag their tab out or accept one in. There's no remaining reason to
keep it. The pill already shows the icon and title in the same row at the
same height, the setting has no UI, and every per-tab feature so far has
either skipped this mode or needed special handling for it.

### 8.1 What it is

- **One setting, two values:** `pane:tabstrip`, either `"always"` (default;
  also what you get when the key is absent) or `"multi-only"`. Typed in
  `frontend/types/srv-types.d.ts:1081-1083`, listed as a plain `string` in
  `schema/settings.json:113`, and round-tripped but never read by the backend
  (`agentmux-srv/src/backend/wconfig/types.rs:120-130`).
- **Origin:** added with universal pane tabs (#3309, 2026-09-17), per
  `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §4.1. It's an
  escape hatch "for the case of hiding the tab-pill-as-title distinction when
  a pane has exactly one tab and a user prefers the older plain-title look",
  modeled on VS Code's `workbench.editor.showTabs`. §7 resolution 1 of that
  spec set the default to `"always"`.
- **What it changes:** exactly one place reads it,
  `PaneHeaderTabStrip.tsx:62-64`:
  `usePillStrip = tabs.length >= 2 || (tabs.length === 1 && setting !== "multi-only")`.
  With `"multi-only"` and one tab, the header renders `BlockFrame_Header`'s
  old icon + view-name title (`blockframe.tsx:749-760`) plus a bare "+"
  (`trailingAddButton`) instead of one pill. When a second tab is added, the
  whole left side of the header **swaps components** to the pill strip. When
  the pane drops back to one tab, it swaps back.
- **How you'd set it:** there's **no settings UI** for it. The only way is to
  hand-edit `settings.json`. So it's effectively hidden. Nobody gets it
  without going looking.

### 8.2 What doesn't work for a lone-tab pane in `"multi-only"`

All of the per-tab features added since #3309 live on the pill, so a pane
without a pill doesn't get them:

| Feature | `"always"` | `"multi-only"`, one tab |
|---|---|---|
| Drag the tab to another pane (parent spec §3.4) | yes | **no** — no pill, so no `draggable()`. Dragging the header moves the whole pane instead |
| Receive a tab dropped from another pane | yes | **no** — the drop target is registered in the pill strip's `onMount` (`PaneTabStrip.tsx:279-311`), which never mounts |
| This spec's tweak 2 (last tab leaves → pane closes) | yes | **unreachable** (follows from row 1) |
| This spec's hover flash / landing bounce | yes | **no** (follows from row 2) |
| Activity flash on the tab (#3625) | yes | **no** — already recorded as a "known gap" in `SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md` §4 item 2 |
| Pill icon, pill color, middle-click close, pill ×, pill rename | yes | replaced by the title-header equivalents (close via end icons, rename via `ViewNameEditor` only if the view supports it) |

### 8.3 What it costs to keep

- **A second render path at the 1↔2-tab boundary.** Avoiding this was the
  redesign's stated goal ("gaining a second tab is a pure state change ...
  rather than a component swap", universal spec §4.1). `"multi-only"` brings
  the swap back.
- **Cleanup code exists only for it.** `PaneTabStrip.tsx:289-296` removes the
  drop-hover class from the header on unmount *because* the header outlives
  the strip in `"multi-only"`. There's a dedicated test for it
  (`PaneTabStrip.dropTarget.test.tsx:173`). This spec's new header strobe and
  counter-invert would need the same care.
- **Every new per-tab feature has to decide what to do without a pill.** So
  far the answer has been "it doesn't work there" (rows above). Doing it
  properly would mean making the plain title header *also* a tab-drag source,
  a foreign-drop target, and a flash target. That's a second implementation
  of each feature on a component that's otherwise legacy, for a setting that
  has no UI.
- **It hides bugs, because nobody runs it.** Found while writing this spec,
  from reading the code only (not reproduced): `PaneHeaderTabStrip.tsx`
  builds `pillStrip` eagerly as a JSX constant (lines 83-117), and Solid
  runs a component as soon as its JSX is evaluated. In `"multi-only"`, a
  pane that *starts* with one tab therefore mounts its pill strip while it
  is **detached** (not yet inside the header). The foreign-drop `onMount`
  (`PaneTabStrip.tsx:279-311`) runs then, `foreignDropRootFor(strip)` finds
  no enclosing `[data-role="block-header"]`, and it falls back to
  registering on the strip box itself. When a second tab later arrives and
  the same strip element is attached to the header, nothing re-registers.
  The likely result: that pane accepts cross-pane drops only on the pills,
  not the whole header row, and highlights the strip instead of the header.
  This goes away with the setting and needs no separate fix.
- **Performance is not a reason either way.** The only saving is the
  always-built, usually hidden "+"-only strip (`addButtonOnly`,
  `PaneHeaderTabStrip.tsx:70-81`): a few detached DOM nodes, a Tooltip, a
  wheel listener and one effect per pane. That's real but too small to
  measure. The decision is about consistency and maintenance.

### 8.4 Removal plan (as implemented in PR C)

1. **Frontend:** `PaneHeaderTabStrip` no longer reads the setting and always
   passes the pill strip as `leadingTabStrip`. PaneChrome's `tabIds` is
   never empty (it falls back to `[activeBlockId()]`), so every PaneChrome
   header has at least one pill. The `addButtonOnly` element is gone.
   **`BlockFrame_Header`'s `trailingAddButton` prop is removed too.** An
   earlier revision of this plan said to keep it, but `PaneHeaderTabStrip`
   was its only caller, so it would have been dead. `BlockFrame_Header`'s
   plain icon+title branch **stays**: the non-PaneChrome header path
   (`blockframe.tsx:1211-1221`) still renders it with no `leadingTabStrip`.
   The `leadingTabStrip` doc (`blocktypes.ts`) no longer says "only with 2+
   tabs". The per-pane name/icon/rename that ReAgent P1 on #3309 guarded
   there now live on the pill (#3373).
2. **Types/schema/backend:** `"pane:tabstrip"` is removed from
   `srv-types.d.ts`, `schema/settings.json` and `SettingsType`
   (`wconfig/types.rs`) in one step, with no grace release. This follows the
   precedent of the earlier retired keys (`ai:*`, `autoupdate:*`,
   `editor:*`, `markdown:*`), which were removed from all three the same way:
   `SettingsType`'s `#[serde(flatten)] extra` catch-all absorbs a leftover
   key, so an old `settings.json` still loads, and nothing validates settings
   against the schema at runtime.
3. **Existing users** with `"multi-only"` set just get pills on single-tab
   panes, i.e. the default look. Nothing breaks. There's no log line (none of
   the earlier retired keys had one either). The change is announced in the
   changeset / release notes.
4. **The unmount cleanup stays** at `PaneTabStrip.tsx` (foreign-drop
   `onCleanup`), along with its test. They're harmless, and the header
   element belongs to `blockframe.tsx`, so nothing ties its lifetime to the
   strip's. Only the comments were reworded so they no longer cite
   `"multi-only"` as the reason.
5. **Docs:** `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §4.1's
   setting paragraph and §7 resolution 1, and
   `PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md` A2, carry
   "superseded/removed" notes pointing here. The activity-flash spec's "known
   gap" is marked closed.

**What we lose:** a hidden option for people who prefer a plain title over
a single pill on one-tab panes. If that preference comes back, bring it back
as a *style* option on the pill (e.g. a single-tab pill that renders
borderless, like a title), never as a component swap. That keeps every
per-tab feature working. Not built now, only if someone asks.

**Rejected alternative:** keep the setting and make the plain title header
a full participant (drag source, drop target, flash and bounce target). That's
more code, in the legacy component, for a setting with no UI.

### 8.5 Tests for the removal

- `PaneHeaderTabStrip.test.tsx` (vitest), rewritten: a one-tab pane gets
  `leadingTabStrip` with exactly one `.pane-tab`, with the settings mock
  serving `"pane:tabstrip": "multi-only"` so the stale value is shown to be
  ignored. No `trailingAddButton` is passed, and the strip's own "+" is the
  only one. The two old "0 tabs → iconview stays" cases are deleted, since
  that behavior is what's being removed and PaneChrome never passes 0 tabs.
  (The plan's A2 "forced to `multi-only` renders a plain title" test was
  never actually in the tree, so there's nothing to delete for it.)
- `PaneTabStrip.dropTarget.test.tsx`: the unmount-cleanup test is kept (the
  cleanup stays, §8.4 step 4), and only its comment changes.
- Rust (`wconfig/mod.rs` `test_settings_unknown_keys_passthrough`): a
  settings JSON containing `"pane:tabstrip": "multi-only"` still
  deserializes, and the key lands in `extra`.
- Manual (`task dev`): with `"multi-only"` hand-set in `settings.json`, a
  one-tab pane shows a pill. Its tab can be dragged to another pane
  (which closes it, per tweak 2), and it accepts a tab dragged from another
  pane with the header flash (tweak 1).
