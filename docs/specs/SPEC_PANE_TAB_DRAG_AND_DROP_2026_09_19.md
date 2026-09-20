# SPEC + PLAN: Pane Tab Drag-and-Drop — reorder, cross-pane move, tear-off-to-floating-pane, and reserved whole-pane-drag space

**Date:** 2026-09-19
**Author:** Agent3
**Status:** active — Phase 1 (§6, `LayoutStackMove` reducer command, backend
only) implemented in PR #3441. Phase 2 (§3.6, reserved whole-pane drag space)
implemented in PR #3442. Phase 3 (§3.1/§3.2, same-pane drag-reorder: the
`pane.moveTab` RPC, `moveBlockInStack`, and the actual per-pill drag UI)
implemented — PR pending at time of writing. Phases 4-6 (cross-pane drops,
tear-off-to-floating-pane) not started.
**Scope:** Dragging an individual **Pane Tab** pill (the pills rendered by
`PaneHeaderTabStrip`/`PaneTabStrip`, one per `block_stack` member — see
terminology below) to: reorder it within its own Pane, move it into a
*different* Pane (as a new split, or as a new tab of that Pane), or tear it
out of the window entirely into a floating pane window. Also covers
preserving today's "drag the whole Pane by its header" gesture now that the
header *is* the tab strip, including reserving enough non-tab space in the
header row to grab for that gesture when the strip is scrolled to its end.
**Companion / supersedes-the-deferral-in:**
`docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §4.8 (scoped
this work out, named it "Phase 4," gated it on the drag-session refactor) and
`docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md` (Task Group
C's "still genuinely deferred" list: *"Tab reorder (drag + keyboard) and
tear-off-from-stack ... both explicitly gated behind the separate drag-session
refactor"*). This document is that deferred work, scoped concretely instead of
left open.
**Builds on (backend):** `docs/specs/SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md`
§6 ("Not in scope: Dragging a tab between panes... when they're built they
should be commands here (`LayoutStackMove`), not whole-tree pushes") — this
spec is that follow-up.
**Must read before touching drag or render state (per the universal redesign
spec's own citation list, still true here):**
`docs/specs/SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` (Draft,
**never implemented** — `dragSession.ts` does not exist in the tree as of this
writing; 6 diagnosed architectural defects behind 4 rounds of dead-tab
incidents; do not add a new pile of scattered drag state on top of this),
`docs/specs/SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` (§3.1/3.2's
global epoch/transaction model — rejected after 4 review passes, do not
re-propose), `docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md`
(this repo's most recent, and explicitly shelved, from-scratch drag rewrite —
read its "Related history" section before assuming a clean slate; the
project's own recorded lesson from that session was *"stop broadening
scope."* This spec is written under that discipline — see §2 Non-goals).
**Prior art reused, not reinvented:** `docs/specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`
(the existing whole-pane-drag machinery: spring-loaded tab-bar hover-switch,
the ghost/`ComputeMove` ride-along preview, the `RedockFloatingPane`-style
saga-plus-layout-queue pattern) and the existing pane tear-off-to-floating-window
path (`CrossWindowDragMonitor`'s `performTearOff`, `WorkspaceService.TearOffBlock`).

---

## 0. Terminology

Reused verbatim from `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §2.1
— **Pane** (a split-tree leaf, a container), **Pane Tab** (one `block_stack`
member inside a Pane — a widget instance), **Window Tab** (the outer,
browser-style strip at the top of the window, unrelated to this spec). This
document is entirely about dragging **Pane Tabs**. "The header" below always
means a Pane's own `PaneHeaderTabStrip` row (`[data-role="block-header"]`),
never the Window Tab bar.

---

## 1. Grounding — what exists today (verified in code, not from the specs' own prose)

### 1.1 The whole-Pane drag handle is the ENTIRE header row, and nothing inside it currently competes

- `BlockFrame_Header` (`frontend/app/block/blockframe.tsx:520-530`) attaches
  `props.nodeModel.dragHandleRef` to the **outer header `<div class="block-frame-default-header" data-role="block-header">`** — the whole row, not a sub-element.
- The actual `draggable()` (pragmatic-dnd) registration is in
  `frontend/layout/lib/TileLayout.core.tsx:441-488`, which re-queries
  `tileNodeRef?.querySelector('[data-role="block-header"]')` fresh each time
  (two components can write the ref — primary header + its ErrorBoundary
  fallback — so it doesn't trust the ref directly). `canDrag` already gates on
  `isEphemeral()`/`isMagnified()`/a per-platform `rejectDragAt` (used today to
  exclude the resize-handle zone, `TileLayout.win32.tsx:110-131` — direct
  precedent for the same kind of gating this spec needs, §3.1).
- `PaneHeaderTabStrip.tsx:98-109` feeds the tab pills and "+" into
  `BlockFrame_Header` via its `leadingTabStrip`/`trailingAddButton` props,
  which render as DOM **children of that same draggable header div**
  (`blockframe.tsx:538-554`, `.block-frame-default-header-tabstrip` /
  `trailingAddButton`).
- **`PaneTabStrip.tsx` registers zero drag handlers today** — confirmed by
  full read and by grep (`draggable(`/`dropTargetForElements` — zero hits in
  `PaneTabStrip.tsx`, `PaneChrome.tsx`, `agent-view.tsx`, `term.tsx`). A tab
  pill's only handlers are `onMouseDown`/`onClick`/`onDblClick`
  (`PaneTabStrip.tsx:294-316`), none of which touches native drag.
- **Consequence:** today, grabbing a tab pill and dragging it drags the
  *whole Pane* (the native HTML5 drag walks up from the mousedown target to
  the nearest `draggable=true` ancestor, which is the header div — there is
  no nested draggable to intercept it first). This is exactly why per-tab
  drag is genuinely new work, not a gap in an existing per-tab mechanism —
  and why it must be built to explicitly coexist with, not silently break,
  the existing whole-Pane drag (§3.1, §3.6).

### 1.2 The header row's "dead space" that whole-Pane drag relies on shrinks to zero once tabs overflow

Header row children, in DOM order (`blockframe.tsx:534-585`,
`frontend/app/mixins.scss`):

| Element | Flex behavior | Source |
|---|---|---|
| `.block-frame-default-header-tabstrip` (wraps `PaneTabStrip`) | `flex-shrink: 3; min-width: 17px` — no grow | `mixins.scss:49-54` |
| `.block-frame-textelems-wrapper` (header text/notices, often empty for agent/term) | `flex: 1 2 auto; min-width: 0` — **the only element that grows to fill leftover space** | `mixins.scss:179-184` |
| `.block-frame-end-icons` (minimize/magnify/close cluster) | `flex-shrink: 0` — fixed | `mixins.scss:231-234` |

Today, `.block-frame-textelems-wrapper` (empty on most agent/term panes) *is*
the de facto "grab here to drag the whole Pane" dead space. It is a **shared
resource with the tab strip**, not a reserved one: `.pane-tab-strip` itself is
shrink-to-fit up to `max-width: 100%` with internal `overflow-x: auto`
(`PaneTabStrip.scss:11-27,100-111`) — as tab count grows, the tabstrip wrapper
claims more of the row before the textelems wrapper is squeezed, and once the
strip's natural content width reaches the available row width, the textelems
wrapper collapses to 0 and **all remaining header space is inside the
tabstrip's own internally-scrolling box.** At that point there is no
dead space left in the header row outside of the fixed end-icons cluster —
exactly the gap the user's request identifies. `.pane-tab-strip-add` ("+",
`PaneTabStrip.scss:328-361`) sits flush against the last tab
(`border-left`), and there is **no existing concept anywhere in
`PaneTabStrip.scss`/the two trailing-blur specs of reserved, non-scrolling,
drag-only space after it** — confirmed by full read of
`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md` and
`SPEC_TERM_PANE_TAB_STRIP_TRAILING_BLUR_2026_09_07.md`: both add a *painted*
translucent band, never a *reserved interactive* one, and both are per-consumer
scoped overrides, not shared strip behavior. §3.6 designs the fix.

### 1.3 Existing prior art this spec reuses rather than reinvents

Three already-shipped mechanisms, each a close structural match for one of
the three new drop behaviors this spec needs:

1. **Spring-loaded hover-then-commit** (today: dragging a *whole Pane* over
   the *outer Window Tab bar*). State lives entirely in
   `frontend/app/tab/tabbar-dnd.ts`: `SPRING_SWITCH_MS = 500`
   (`tabbar-dnd.ts:68`), `hoveredDropTabId` signal (`:55`),
   `dragActivatedTabIds` Set (`:74`). The dwell timer + drop target is
   `frontend/app/tab/droppable-tab.tsx:220-275`'s
   `dropTargetForElements({ canDrop: source.data.type === tileItemType,
   onDragEnter: () => { setHoveredDropTabId(...); springTimer =
   setTimeout(...) }, onDragLeave: clearSpring, onDrop: ... })`, registered
   as a *second* target on the same element that is *also* a drag source for
   itself (tab reordering) — direct precedent for a tab pill needing to be
   simultaneously a drag source and a drop target (§3.4).
2. **Ghost/`ComputeMove` split preview** (today: dragging a whole Pane over
   *another Pane's content*). `frontend/layout/lib/tilelayout-shared.tsx`'s
   `OverlayNode` registers a per-leaf `dropTargetForElements`
   (`:436-460`) that computes a drop quadrant (`determineDropDirection`/
   `clampCrossTabDirection`), dispatches `LayoutTreeActionType.ComputeMove`
   to drive the ghost, and commits via `layoutModel.onDrop()` (same-tab) or
   `redockDraggedPane` (cross-tab). This already works, unmodified, for any
   drop payload shaped like today's `{kind:"tile", node, sourceTabId}}` — see
   §3.3 for the one thing that has to change (the commit target is a single
   block, not a whole node).
3. **Tear off to a floating window** (today: dragging a whole Pane, or a
   whole Window Tab, out of the window). `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`'s
   `handleCrossWindowDragEnd` (`:186-256`) calls `performTearOff`, which
   branches on `dragType`:
   - `"tile"` (a Pane): measures pane size, calls
     `WorkspaceService.TearOffBlock(blockId, sourceTabId, sourceWsId, true)`
     (creates a new workspace+tab holding *that one block*), then
     `open_floating_pane_window` (Rust command), and **only on IPC success**
     deletes the docked layout node (`CrossWindowDragMonitor.win32.tsx:278-408`,
     IPC-first ordering deliberate — comment at `:333-340`).
   - `"tab"` (a whole Window Tab): `WorkspaceService.TearOffTab` +
     `openTearOffWindow` — spawns an entirely new top-level
     window/instance. **Wrong granularity for this spec** — see §3.5.

   `TearOffBlock` already takes a bare `block_id`, not a whole leaf/node — it
   is very likely *already* stack-member-safe today, because
   `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md` Phase 0 (#3414, already
   merged) changed the queued `delete` pending-action's semantics from "delete
   the whole leaf" to "remove this one block, delete the leaf only if it was
   the last member" (`frontend/layout/lib/stackMembers.ts`,
   `agentmux-srv/src/backend/layout/mod.rs:256-276`
   `remove_stack_member`, wired into `LayoutDeleteNodeByBlock`,
   `agentmux-srv/src/reducer/layout.rs:414-438`). **Confirm this during
   implementation, don't assume it** (§7, Phase 3) — nothing has ever
   exercised `TearOffBlock` against a stacked pane before.

### 1.4 Reducer primitives that already exist, and the one gap

`agentmux-srv/src/backend/layout/mod.rs` already has all three pure
tree-mutation functions a cross-pane move needs:

- `push_stack_member(tree, target_block_id, block_id, activate) -> bool` (`:210-230`)
- `activate_stack_member(tree, block_id) -> bool` (`:235-246`)
- `remove_stack_member(tree, block_id) -> bool` (`:256-276`) — removes a
  member from its leaf *without deleting the leaf*, reassigning
  `active_block_id` to `next_visible_member`'s neighbour if it was visible;
  returns `false` (no-op, caller's job) if it's the leaf's only member.

Each already has an `ipc.rs` `Command` variant (`LayoutStackPush` §577-583,
`LayoutStackActivate` §585-589, `CreateBlockInStack` §595-601) and a reducer
handler (`agentmux-srv/src/reducer/layout.rs:538-600`) following one shared
pattern: `stack_tree_edit(state, op, tab_id, correlation_id, not_found, |root|
{ ...pure fn on root... })` → one `Event::LayoutTreeReplaced` (tree-only, no
focus/magnify/leaf-order slices).

**The gap, exactly as `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md` §6
predicted:** no `LayoutStackMove`/`LayoutStackRemove` command exists.
`remove_stack_member` is currently only called from the delete/close path
(`handle_layout_delete_node_by_block`). §5 designs `LayoutStackMove` as a
single new command chaining the two existing pure functions — this is a small,
low-risk addition given the primitives are already there and already tested
indirectly via Phase 0/1.

---

## 2. Non-goals (scope discipline — read this before writing any code)

This repo has a long, repeatedly-shelved history of drag/tear-off rewrites
(`docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md` lists
over twenty prior branches touching this area). The explicit, repo-owner-given
lesson from the most recent attempt was to **stop broadening scope**. This
spec is deliberately narrow:

- **Not** a rewrite or fix of `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md`'s
  six diagnosed defects in the *existing* whole-Pane/Window-Tab drag system.
  Those defects live in `TileLayout.*`, `crossTabDrag.ts`, `dragInFlight.ts`,
  `tabbar-dnd.ts` — this spec does not touch any of that code's internals,
  only reuses its *outputs* (the ghost/`ComputeMove` mechanism, the
  spring-switch pattern, the tear-off RPCs) as black boxes. See §6 for the
  one, intentionally minimal, new piece of drag state this spec *does* add.
- **Not** a Windows cursor-freeze fix, not a native-pointer-capture rewrite,
  not a macOS/Linux drag-snapback fix. Every one of those is a separately
  tracked, previously-shelved thread (see the retro's "Related history") —
  re-litigating any of them inside this feature is exactly the scope-creep
  the retro warns against.
- **Not** a change to the outer Window Tab bar (`tabbar.tsx`) — out of scope
  per the universal spec §6 and unaffected here.
- **Not** full visual/animation polish (exact flash timing, colors) — a
  design-review pass, called out per-interaction below with a recommended
  default, same treatment the predecessor specs gave this class of detail.
- **Not** keyboard-accessible reorder (`Ctrl:Alt:[`/`]` from the universal
  spec §4.9's resolved table) — real, tracked, but a separate, additive PR;
  nothing in this spec's data model blocks it (a `LayoutStackMove` command
  is equally callable from a keyboard handler).

---

## 3. Design

### 3.1 Drag source: per-Pane-Tab `draggable()`, explicitly coexisting with the whole-Pane drag

New `draggable()` registration in `PaneTabStrip.tsx`'s `PaneTabStripItem`
(pragmatic-dnd, matching the payload-shape convention already used
throughout this codebase):

```ts
type PaneTabDragPayload = { kind: "pane-tab"; blockId: string; sourceNodeId: string; sourceTabId: string };
```

Each pill's `draggable()` sets this payload on `dragstart`. Because native
HTML5 drag resolves to the *nearest* `draggable=true` ancestor of the
mousedown target, a pill marked `draggable` nested inside the header's own
`draggable` div will, in principle, already take priority for mousedowns
landing on it. **Do not rely on that alone** — this codebase already has one
documented CEF/WebView2-specific quirk in this exact area
(`TileLayout.win32.tsx`'s WebView2 comment on header drag), and the shelved
native-pointer-drag retro is a standing reminder not to trust assumed browser
drag semantics in this embedded environment without verifying. Add an
explicit, defense-in-depth gate on the *header's* own `draggable()` in
`TileLayout.core.tsx`'s `canDrag`, the same pattern already used for the
resize-handle exclusion (`TileLayout.win32.tsx:110-131`
`rejectDragAt`): reject drag-start when `input.target` (or its closest
ancestor) matches `.pane-tab`, `.pane-tab-close`, or `.pane-tab-strip-add`.
This makes the coexistence rule explicit and testable instead of implicit.

### 3.2 Same-pane reorder

Hovering another pill within the *same* `PaneTabStrip` while dragging a
`kind: "pane-tab"` payload: track an `insertionIndex` signal local to that
strip instance (a smaller-scoped version of the outer tab bar's own
`computeInsertionPoint`, `tabbar-dnd.ts:43-81` — reuse the *algorithm*, not
the module, since this needs to be per-strip-instance, not a singleton).
On drop, dispatch `LayoutStackMove` (§5) with `target_block_id` = the pill
being hovered and the same `tab_id`/leaf — same-leaf move, no cross-pane
RPC branch needed at the frontend call-site (the reducer command shape is
identical either way; see §5).

### 3.3 Cross-pane drop on a Pane's CONTENT area → new split, with ghost (reuse existing mechanism almost verbatim)

Hovering a **different** Pane's content (below its header) with a `kind:
"pane-tab"` payload: reuse `OverlayNode`'s existing per-leaf
`dropTargetForElements`/`ComputeMove`/ghost machinery
(`tilelayout-shared.tsx:436-460`) unmodified for the *preview* — it already
works for any drag payload shaped for direction resolution. The only new
work is at the **commit** step: today's commit (`props.layoutModel.onDrop()`
/ `redockDraggedPane`) moves a whole node; this spec's commit instead:

1. Splits the target leaf per the previewed direction (reuse
   `queue_target_layout_split`, `agentmux-srv/src/server/service/layout_helpers.rs:123-159`,
   unmodified — it already builds a fresh single-block leaf from a
   `block_id`, which is exactly what a torn-out Pane Tab is).
2. Removes `blockId` from the *source* leaf's stack via the new
   `LayoutStackMove`/`LayoutStackRemove` reducer path (§5) — **not**
   `queue_source_layout_delete`, which assumes whole-leaf deletion and is
   the wrong tool once the source is a multi-member stack (leaving the
   source leaf and its OTHER tabs untouched is the entire point).

### 3.4 Cross-pane drop on a Pane's HEADER → append as a new tab there (reuse the spring-loaded pattern, scoped one level in)

Hovering a **different** Pane's `PaneHeaderTabStrip` row (its
`[data-role="block-header"]`, specifically the tabstrip region — not its
end-icons cluster) with a `kind: "pane-tab"` payload: this is the interaction
the user asked to "reuse the window tab animation" for. What's actually
reusable is the *pattern* (dwell timer + one-shot hover-flash CSS class +
commit-on-drop), not a literal call into `tabbar-dnd.ts` (that module is
scoped to the outer Window Tab bar, a `dropTargetForElements` keyed off
`tabWrapperRefs: Map<tabId, HTMLDivElement>`; a Pane's header is a different
kind of target). Concretely:

- `PaneHeaderTabStrip` (or `PaneChrome`, whichever ends up owning the
  drop-target registration — implementer's call, no architectural
  significance either way) registers a `dropTargetForElements` on the
  header's tabstrip region, `canDrop: source.data.type === "pane-tab" &&
  source.data.sourceNodeId !== thisNode.id`.
- **One real difference from the outer-bar case, worth being deliberate
  about rather than copying blindly:** the outer bar's dwell exists because
  committing means *switching to a hidden Window Tab* — the user needs to see
  what they're about to drop into before committing (§3 point 2 of the
  universal redesign spec: cmux's own HN feedback on this exact issue —
  "you never really know what it's going to do on mouseup"). A target
  Pane's content is **already visible** the whole time (it's not hidden like
  an inactive Window Tab) — there's nothing to reveal by dwelling. Recommend:
  skip the `SPRING_SWITCH_MS` dwell-then-switch step entirely; instead, on
  `onDragEnter` immediately apply a one-shot hover-flash class to the target
  header (reusing the *visual* language of `.tile-drop-hover`/the strobe
  invert from `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` addendum A1, not its
  timing), and commit on drop with no dwell gate. This is a design-review
  call, not an architectural one — flagged explicitly per the house pattern
  of leaving exact timing to implementation (§4.10 of the universal spec did
  the same for tab-strip visual polish).
- Commit: `LayoutStackMove` with `target_block_id` = the target Pane's
  currently-active member, `activate: true` (a freshly-added tab becomes
  visible, matching every other "add a tab" path in this codebase —
  `addWidgetAsPaneTab`, quick-fork, etc., all activate on add).

### 3.5 Drag out of the window entirely → floating pane

On drop with no AgentMux window under the cursor (same cross-process
hit-test `CrossWindowDragMonitor` already performs for every drag type),
reuse `performTearOff`'s **`"tile"` branch** (§1.3 item 3) — `TearOffBlock`
+ `open_floating_pane_window` — driven by the dragged pill's `blockId`
directly (`TearOffBlock` already takes a bare block id, not a node). **Do
not** use the `"tab"` branch (spawns a whole new top-level
window/instance — wrong granularity for "one Pane Tab becomes a floating
pane") and **do not** implement this as `closeBlockInStack` +
re-`pane.open{floating:true}`, even though that is literally what
`SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §7 resolution 5
describes for the analogous "tear one tab out of a stack" case. That
resolution predates having read `TearOffBlock`'s actual behavior: closing
the block and reopening a fresh one of the same widget type **destroys and
recreates** it — for an agent Pane Tab, that means losing the live
conversation/session, not relocating it. `TearOffBlock` already does the
correct thing (moves the *same* block to a new workspace+tab, preserving
its identity/state) for the existing whole-Pane and whole-Window-Tab
cases; there is no reason a Pane Tab drag needs the lossy path when the
non-lossy one already exists and needs zero new backend work. **Flagging
this as a correction to the universal spec's resolved §7, to be confirmed
by the repo owner (§7 open questions below), not silently overridden.**

Source-side cleanup: same as §3.3 — `LayoutStackMove`/`LayoutStackRemove`
removes just the one member, not `queue_source_layout_delete`'s whole-leaf
semantics. Per §1.3 item 3, confirm during implementation whether the
Phase-0 `delete`-action fix already makes the existing `TearOffBlock` +
`queue_source_layout_delete` pairing stack-safe as-is (in which case no new
frontend wiring is even needed here beyond driving it from the new drag
gesture) — do not assume either way without a test.

### 3.6 Reserved whole-Pane-drag space, only when the tab strip is actually overflowing

Per §1.2's grounding: the fix is a small, non-interactive-except-for-dragging
spacer element, rendered as the **true last child of `.pane-tab-strip-inner`**
(after the "+" button) — inside the scrollable region, so it only becomes
reachable by scrolling to the strip's end, matching the user's own framing
("when tabs overflow and the user is using the scroll wheel to reach the
end").

- Width: ~2× `.pane-tab-strip-add`'s 28px (~56-60px) — a `PaneTabStrip.scss`
  constant, e.g. `--pane-tab-strip-drag-handle-width: 56px`.
- Rendered unconditionally in the DOM (simplest, matches how `.pane-tab-strip-add`
  is already always the last flex child) but given `width: 0` / `flex: 0 0
  0` unless the strip is overflowing; reuse the exact overflow check the
  wheel handler already computes (`el.scrollWidth > el.clientWidth`,
  `PaneTabStrip.tsx:199`) rather than adding a second one. When not
  overflowing, `.block-frame-textelems-wrapper`'s natural flex-grow space
  (§1.2) already serves this purpose, so a non-zero-width spacer here would
  just be redundant dead space in the common case — this is why the spec
  scopes it to the overflow case specifically, per the user's own framing,
  not "always reserve it."
- Class name distinct from `.pane-tab`/`.pane-tab-strip-add`
  (e.g. `.pane-tab-strip-drag-handle`) so it is **excluded** from §3.1's
  `rejectDragAt` gate (i.e. mousedown-drag starting here is NOT rejected by
  the header's own `canDrag` — it's deliberately exempt, the one place left
  after the redesign where "drag to move the whole Pane" still works exactly
  like today) and excluded from §3.2's reorder hit-testing and §3.4's
  drop-target registration (it is not a tab, not a drop target, purely a
  drag-source affordance).
- No new backend/reducer work — this is CSS + a small conditional render,
  the cheapest piece of this spec.

---

## 4. Backend: the reducer angle

### 4.1 New command: `LayoutStackMove`

```rust
// agentmux-common/src/ipc.rs — alongside LayoutStackPush/LayoutStackActivate,
// same field-naming convention (tab_id, ..._block_id, correlation_id, activate: bool)
LayoutStackMove {
    tab_id: String,
    block_id: String,          // the Pane Tab being moved
    target_block_id: String,   // an existing member identifying the DESTINATION leaf
                                 // (its own leaf, for a same-pane reorder; a different
                                 // leaf's member, for a cross-pane move)
    position: StackMovePosition, // Before(target) | After(target) | End
    activate: bool,
    correlation_id: String,
},
```

Handler (`agentmux-srv/src/reducer/layout.rs`, following the exact
`stack_tree_edit` pattern §1.4's existing three commands already use):

```rust
pub(super) fn handle_layout_stack_move(
    state: &mut State, tab_id: String, block_id: String, target_block_id: String,
    position: StackMovePosition, activate: bool, correlation_id: String,
) -> Vec<Event> {
    let not_found = format!("no pane holds block {block_id} or {target_block_id}");
    stack_tree_edit(state, "LayoutStackMove", tab_id, correlation_id, not_found, |root| {
        crate::backend::layout::move_stack_member(root, &block_id, &target_block_id, position, activate)
    })
}
```

New pure fn `move_stack_member` in `agentmux-srv/src/backend/layout/mod.rs`,
built by **composing the two existing primitives** (§1.4) inside one tree
walk rather than issuing two separate commands/events — avoids ever
observing (or persisting, or racing on) an intermediate tree state where the
block has been removed from its source but not yet added to its
destination:

1. Locate `block_id`'s current leaf. If it's already the leaf holding
   `target_block_id` → pure reorder: splice within that leaf's
   `block_stack` at the position relative to `target_block_id` (new
   capability — today's `push_stack_member` only appends; this needs an
   index-aware insert, the one genuinely new piece of tree-mutation logic
   this spec adds).
2. Otherwise: same neighbour-reassignment logic `remove_stack_member`
   already has (§1.4) removes it from the source leaf, `push_stack_member`-style
   insert (now index-aware) adds it to the target leaf at the requested
   position, `activate` applied if requested.
3. Refuse (return `false`, no-op) if `block_id` is its source leaf's *only*
   member — same rule `remove_stack_member` already enforces (removing the
   last member is "close the pane," not "move a tab," and is the caller's
   job, unchanged from today).

This single command covers §3.2 (same-pane reorder), §3.3's source-side
half, and §3.4's commit — one reducer code path for all three, rather than
three.

### 4.2 What's explicitly NOT changed

- The whole-tree push path (resize/split/whole-Pane drag) — untouched,
  still last-writer-wins, per `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md`
  §3.5's own scoping ("Phase 2 — stale-push protection," not this spec's
  job).
- `I1`/`I2` invariant enforcement (`SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md`
  §3.1, §3, Phase 3 "enforce") — `move_stack_member` must preserve them
  (tested, §6), but building the general enforcement/reaper machinery is
  that spec's job, not this one's.
- `LayoutStackPush`/`Activate`/`CreateBlockInStack` — unmodified; existing
  callers (the "+" picker, History tab, quick-fork) are unaffected.

---

## 5. Drag-session state: the one new piece, deliberately minimal

Per §2's scope discipline, this spec does **not** refactor the existing
whole-Pane/Window-Tab drag system. But it does need *some* place to hold
"which Pane Tab is currently being dragged, which Pane/position is currently
hovered" — and building that directly on top of the currently-scattered
model (`SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` §2 counts
~14 stores across 3 layers already) would just add store #15 and inherit the
same defect risk that spec diagnosed (teardown relying on `dragend`, which
that spec's own research confirms is not guaranteed to fire once a drag
source unmounts mid-gesture — exactly what happens here on a successful
cross-pane move).

**Recommendation:** one new, small, single-owner module,
`frontend/layout/lib/paneTabDragSession.ts`, modeled on the *shape* of that
Draft spec's proposed `DragSession` FSM (`idle → dragging → settling →
idle`, monitor-driven teardown as the single reliable signal — §3.1/§3.3 of
that spec) but scoped to **only** this new interaction:

- Owns: the dragged `{blockId, sourceNodeId, sourceTabId}`, the currently
  hovered target (`{nodeId, zone: "content" | "header" | "reorder", index?}`),
  derived signals for the three highlight states (§3.2/§3.3/§3.4's hover
  visuals all read from here, never independently toggled).
- Commit happens in exactly one place (the session's monitor-driven
  `settle()`), strictly after pragmatic-dnd's own teardown completes — same
  "commit-after-teardown" principle as that Draft spec's P3
  (§3.3 there), applied narrowly.
- Does **not** touch, wrap, or modify `TileLayout.*`, `crossTabDrag.ts`,
  `tabbar-dnd.ts`, `droppable-tab.tsx` in any way — those keep running
  exactly as they do today, for whole-Pane and Window-Tab drags. The two
  systems coexist by payload-kind discrimination (`kind: "tile"` vs.
  `kind: "pane-tab"` vs. the outer bar's `kind: "tab"`), not by sharing
  state.

This is a genuine, if small, piece of new drag-state machinery — called out
explicitly rather than silently added, since "no new drag state" was this
spec's own stated discipline in §2. The distinction being drawn: *scoped,
single-owner, FSM-shaped* state for a *brand-new* drag surface is not the
same risk class as *more* state bolted onto the *already-diagnosed-defective*
existing surface, which is what §2 actually forbids.

---

## 6. Phased implementation plan

### Phase 1 — Reducer command (backend only, no UI change)
`LayoutStackMove` (§4.1), `move_stack_member` (§4.1), unit tests: same-leaf
reorder preserves membership/order; cross-leaf move preserves I1/I2
(§1.4/§4.2); refuses to strip a leaf to zero members; neighbour-activation
on a visible member's removal matches `remove_stack_member`'s existing rule
(shared vector test, per `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md` §3.6's
precedent).

### Phase 2 — Reserved drag-handle space (frontend, CSS + small conditional render, §3.6)
Lowest-risk, purely additive, zero interaction with any drag code. Ship
independently — it fixes a real, already-present gap (dragging a Pane by its
header stops working once its tab strip overflows) regardless of whether the
rest of this spec ships. Test: a pane with enough tabs to overflow renders
the spacer at non-zero width and it's excluded from the tab-strip's own
hit-testing; a pane with few tabs renders it at zero width.

### Phase 3 — Same-pane reorder (§3.1, §3.2)
`paneTabDragSession.ts` (§5, minimal — just enough for this phase:
`dragging`/`reorder-hover`/`idle`), per-pill `draggable()`, the header's
`canDrag` exclusion (§3.1), in-strip reorder hit-testing, `LayoutStackMove`
same-leaf call. Smallest surface that proves the new payload kind and
session module work before adding cross-pane/tear-off risk. Manual pass:
drag a tab within a 3+-tab pane to every position; confirm whole-Pane drag
(grabbing the reserved space from Phase 2, or the natural dead space when
not overflowing) still works unaffected.

### Phase 4 — Cross-pane drop on content (§3.3)
Wires the new commit path into the *existing*, unmodified
`OverlayNode`/`ComputeMove` ghost machinery. Test: dragging a Pane Tab onto
another Pane's content in each of the 9 directions produces the same split
`queue_target_layout_split` already produces for a whole-Pane drag; source
leaf keeps its other members; I1/I2 hold after.

### Phase 5 — Cross-pane drop on header (§3.4)
The new, Pane-header-scoped hover/commit target. Design-review pass on the
exact hover-flash visual/timing (§3.4's recommendation: no dwell, immediate
flash) — flagged as the one place in this plan where "verified via
`task dev`, not just tests" matters most, matching how the universal
redesign's own Task Group B4 flagged its own visual-verification limits.

### Phase 6 — Drag out of window → floating pane (§3.5)
Confirm (test, not assumption) whether `TearOffBlock` +
`queue_source_layout_delete` is already stack-safe post-Phase-0 (§1.3 item
3); wire the new payload kind into `performTearOff`'s existing `"tile"`
branch. **Requires repo-owner sign-off on the §7 correction to the
universal spec's §7 resolution 5 before this phase starts** (see Open
Questions below) — everything else in this plan can proceed independently
of that answer, but this phase's chosen mechanism depends on it.

### Cross-cutting, every phase
- `npx tsc -p tsconfig.citypecheck.json --noEmit` clean.
- `npx vitest run frontend/app/element frontend/app/tab frontend/layout/lib` — existing + new tests pass.
- `cargo check --workspace --tests` clean (Phase 1 only touches Rust).
- `task dev` manual pass, console/log inspection for runtime errors — same
  explicit "I cannot confirm the visual result looks correct at a glance"
  caveat the universal redesign plan gave its own B4 task; this plan makes
  the same admission rather than silently claiming full verification.
- Per the universal redesign's own branch policy (still in force — this is
  the same class of far-reaching, drag-adjacent change): implement on a
  dedicated branch, open a PR for CI signal, **do not merge on approval** —
  stays unmerged until the repo owner has tested it locally and explicitly
  approves.

---

## 7. Open questions for the repo owner (do not resolve unilaterally)

1. **§3.5's correction to the universal spec's §7 resolution 5.** That
   resolution explicitly recommended `closeBlockInStack` + reopen; this spec
   recommends `TearOffBlock` (relocate-in-place, state-preserving) instead,
   based on code this spec's author read that the original resolution
   likely didn't. Needs explicit confirmation before Phase 6, not a silent
   override — this is exactly the kind of "policy claims a change without
   the repo owner's own confirmation" pattern this repo's own CLAUDE.md
   warns to be skeptical of, applied here to a design-doc claim rather than
   a jekt, but the discipline is the same: flag it, don't assume it.
2. **§3.4's no-dwell recommendation** — a genuine UX call (skip the outer
   bar's 500ms spring-switch dwell since there's nothing hidden to reveal).
   Reasonable default, but worth a real look during `task dev` before
   treating it as final, same as any other visual-timing decision in this
   codebase's history.
3. **Scope confirmation for Phase 5's targeting**: should dropping on a
   target Pane's header always *append* the new tab, or should it respect
   drop position along the header (drop near the left edge inserts first,
   etc.)? This spec defaults to append (simplest, matches how every other
   "add a tab" path in this codebase behaves) but position-aware header
   drop is a reasonable follow-up if requested.
4. Whether Phase 3 (same-pane reorder) alone is worth shipping standalone
   before committing to Phases 4-6, given this repo's history of drag work
   stalling mid-flight (§2) — the phases above are already ordered so each
   is independently mergeable/valuable, but confirming the repo owner wants
   the full scope up front (vs. re-evaluating after Phase 3 lands) avoids
   over-planning work that gets shelved like prior attempts.

---

## 8. Test plan summary

- Unit (Rust): `move_stack_member` — same-leaf reorder, cross-leaf move,
  refuses to empty a leaf, I1/I2 preserved, neighbour-activation rule
  matches `remove_stack_member`'s (shared vector).
- Unit (vitest): `paneTabDragSession.ts` transition table; the header
  `canDrag` exclusion (§3.1) rejects drag-start on `.pane-tab`/
  `.pane-tab-strip-add`, accepts it on the Phase 2 spacer and elsewhere in
  the header; Phase 2's overflow-gated spacer width.
- Integration (vitest/jsdom): same-pane reorder end-to-end; cross-pane
  content drop produces the expected split and leaves source siblings
  intact; cross-pane header drop appends and activates; drag-out-of-window
  produces a floating pane with the block's live state intact (not a fresh
  block) once Phase 6's mechanism is confirmed.
- Manual (`task dev`, every phase): the whole-Pane drag gesture (both via
  natural dead space and, once tabs overflow, the Phase 2 reserved space)
  continues to work exactly as it does on `main` today — this is the one
  regression this entire spec must not introduce, checked explicitly at the
  end of every phase, not just once at the end.
