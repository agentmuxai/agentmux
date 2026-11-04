# Spec: Redock puts a floating pane back where it was torn from (best effort), else to the right as today

**Date:** 2026-11-03
**Author:** Agent2@narko
**Type:** Spec (task #21884; the build starts after the operator approves it)
**Status:** implemented, 2026-11-04, in the build task #21885 (see §7 for what the build did differently)
**Requested by:** the operator, 2026-11-03: *"we want to make best effort to remember what the pane's restore position was. this may have already been speced out. essentially in simple cases where the user didnt change anything in the layout, we can simply restore a pane back to its place, in cases where minor changes like resizes, that should be possible too. in compleicated cases like a closed tab and an entirely differently arranged layout from where the pane was original torn off of, that then simply gets the pane reappended to the rightside of the window, just like now"*
**Builds on:** `SPEC_FLOATING_PANE_REDOCK_BUTTON_2026_10_15.md`, whose decision D5 ("remember the tab a floater was torn from") is this spec. Not specced before: that spec §2 says "Nothing remembers which window or tab a floater was torn from".
**Coordinate with:** Camper (layout and drag owner, #21881).

## 1. What happens today

- **Tear-off.** `TearOffBlock` (`crates/srv/src/server/service/tear_off.rs`, saga `sagas/tear_off_block.rs`) moves the block, or the pane's whole tab stack, into a new single-tab workspace and takes its leaf out of the source tab's tree. Nothing about where it was is kept.
- **Redock.** `RedockFloatingPane` (same file, saga `sagas/redock_floating_pane.rs`) moves the block into a target tab and places it through `layout_intents::place_block`: a `Placement::Split { target_block_id, horizontal, position, size_fraction }` when a drop gave a target leaf and a direction, else `Placement::Append`. `Split` already falls back to `Append` when the target pane is gone or minimized.
- **The Redock button** (`frontend/app/block/floating-redock.tsx`, `pane:redock`) always targets the main window's active tab and gives no leaf, so the pane is appended there.
- **The layout is srv's.** The tree of a tab is `tabs[tab].rootnode` in srv (`LayoutNode`: `children`, `flex_direction`, `size`, a leaf's `data.block_id` and `block_stack`), and `layout_intents` edits it and mirrors the change to the window. So everything below can be decided in srv, with no window involved.

## 2. The rule

> Redock by the button, the menu or `pane:redock` returns the pane to the place it was torn from when that place can still be recognised, and otherwise does what it does today.

### 2.1 Recording the origin (at tear-off)

`handle_tear_off_block` reads the source tab's tree **before** the leaf is removed and stores an origin record in the block's meta, key `floater:origin`, on the torn-off block (the visible one, which is the one the floater keeps as the pane's first member). Block meta is persisted, so the record survives a restart and an upgrade. The record:

| Field | Meaning |
|---|---|
| `tab_id`, `workspace_id` | where the pane was |
| `stack_mate` | for a pane-tab tear-off that leaves the pane standing: a block that stayed in the pane (the origin is then "as a tab of that pane") |
| `anchors` | up to four candidate neighbours, nearest first (§2.2) |

Each anchor is `{ block_id, horizontal, position: "before" | "after", fraction }`: a block of a neighbouring pane (any member of its stack), the split axis and side the torn-off pane sat on relative to it, and `fraction`, the share of the two panes' combined size that the torn-off pane had (`own / (own + anchor)`, from the nodes' `size` in their shared parent). The fraction is a share, not pixels, so a resize of the window or of other panes does not matter.

Anchors, nearest first: the previous and next sibling in the parent (the nearest leaf inside each when the sibling is a split), then the same for the parent's own siblings, up to four. A pane alone in its tab has none.

The record is dropped when the pane is docked again by any path (button, menu, drag redock), and with the block.

### 2.2 Redock to the origin (the button, the menu and `pane:redock`)

`RedockFloatingPane` takes a new optional last argument, `toOrigin`. With it srv resolves the place and then uses the same move, layout and floater-closing steps as before. The target the call names is the fallback: what the button passes today, the main window's active tab. In order:

1. **No record, the origin tab is gone, or its workspace is open in no window** → **(c)** below.
2. **(a′) Stack mate.** If `stack_mate` is still a pane member in the origin tab, the pane goes back as a tab of it (the existing `as_tab` path), active. This comes first: the pane never left, so its own tab strip is the place.
3. **(a) The neighbour is still there.** The first anchor whose block is in a pane of the origin tab's tree, and not minimized: `Placement::Split` against it with the recorded axis, side and fraction. Resizes, and panes added elsewhere, do not change this.
4. **(b) The first neighbour is gone, a later one is there.** The same split against the next surviving anchor in the list. This is "the nearest surviving neighbour from the recorded path": the list is ordered by nearness, so the first survivor is the nearest.
5. **(c) Beyond recognition → as today.** No anchor survives (all closed, minimized or moved to other tabs), or the origin tab is gone: append to the fallback tab, exactly the current behaviour.

"Beyond recognition", testably: **the origin tab does not exist, or none of the recorded anchors is a non-minimized pane of it.** It does not look at how the rest of the layout changed: a layout rearranged around a surviving neighbour still redocks next to that neighbour.

The answer tells the caller where the pane went: `placed` is `"stack"`, `"neighbour"` or `"fallback"`, with `target_tab_id` and `target_workspace_id`.

### 2.3 After the redock

As in the button spec the floater closes through the empty-tab watcher, and the window that shows the destination comes forward (decision D3 of that spec), now the origin window rather than always the main one. **The tab does not change** (the operator's rule that clicks do not switch tabs): when the pane lands in a tab the window is not showing, a quiet notice names the tab with a "Go there" action, the same notice an agent's open uses (`reveal-app-scope.ts`, `offer`). Question 1.

### 2.4 What does not change

- **A drag redock onto a chosen tile** goes where it is dropped (`CrossWindowDropOverlay`). The origin only applies to the button, the menu and `pane:redock`; a drag redock clears the record like any docking.
- The saga's contract, the dwell gate and the ghost overlay.
- Tear-off of a whole tab or window; only a pane or a pane tab becomes a floater with a record.

## 3. Where the code goes

| Piece | File |
|---|---|
| Build the record from the source tree | a new `redock_origin.rs` next to `tear_off.rs` (pure: tree and block ids in, record out) |
| Store it | `handle_tear_off_block`, before the leaf is removed; block meta `floater:origin` |
| Resolve and place | `toOrigin` in `handle_redock_floating_pane` (`tear_off.rs`), using `layout_intents::place_block`, `redock_as_tab` and the existing source-delete step |
| Button, menu, `pane:redock` | `floating-redock.tsx` calls the new service and brings the window that shows the destination forward; the drag overlay is untouched |

No frontend drag code changes, so this does not overlap #21881's drag-preview work. The build touches `tear_off.rs`, `layout_intents.rs` (a helper to find a non-minimized pane by block) and `floating-redock.tsx`; Camper is told before it starts.

## 4. Tests

Srv, on the layout test harness (`layout_intents_tests.rs`):

1. **Record:** tearing a pane out of a three-pane tab stores the previous or next sibling first, the axis, the side and a fraction that matches the node sizes; a lone pane stores no anchors; a stack tear-off stores `stack_mate`.
2. **(a)** Tear off the middle of three panes, resize the window and the others, redock: the pane is next to the same neighbour, on the same side, with the same share.
3. **(a′)** Pane-tab tear-off, redock: the tab is back in its pane.
4. **(b)** Tear off, close the nearest neighbour, redock: the pane is beside the next surviving anchor.
5. **(c)** Close the origin tab; and, separately, close every anchor: the pane is appended to the fallback tab, as today.
6. **Drag path:** `RedockFloatingPane` with a target and direction is unchanged, and the record is dropped.
7. **Restart:** the record is read back from the stored block meta.

Frontend: the button calls the origin service with the main window's active tab as the fallback, and shows the notice when the destination is not the shown tab.

## 5. Questions for the operator

1. **The origin tab is not the one the window shows.** Recommend: do not switch tabs; the pane lands in its tab, the origin window comes forward, and a notice with "Go there" says where. The alternative is to switch to that tab so the pane is visible at once.
2. **The origin tab exists but nothing in it can be recognised.** The task says to append to the main window's active tab as today. The alternative is to append to the origin tab, which is closer to where it came from. Recommend: as today, as asked.
3. **How many neighbours to remember.** Four is enough for a few closed panes; more makes the record larger for little gain.

## 6. Not covered

- Choosing a window or tab on purpose (D2 of the button spec).
- Remembering where a floater was moved on screen; this is about the tile layout.
- I did not run the app for this spec: the call paths and tree structures come from reading `tear_off.rs`, `layout_intents.rs`, `layout/mod.rs` and the redock saga.

## 7. What the build did differently

- **One argument, not a new call.** `toOrigin` is an optional last argument of `RedockFloatingPane` (`services.ts`, `floater-panes.ts`), so the saga, the stack members, the floater closing and the failure rollback are the existing code, and the drag overlay's call is byte-for-byte what it was.
- **The record is on every torn-off block**, not only the first, so whichever block of a floater's stack a redock asks about finds it. A docked pane drops it.
- **Stack mate before neighbour** (§2.2 step 2).
- **No "Go there" notice yet.** The floater's renderer closes as the redock ends and there is no channel for a toast in another window, so a notice raised there would never be seen. The window that shows the pane comes forward and no tab is switched. A notice from the destination window needs a window-targeted notice event; filed as a follow-up.
- **Tests.** `redock_origin.rs` covers the record, (a), (a′), (b) and (c) on trees (including resizes and chips); `tear_off.rs` covers the button end to end (neighbour, fallback when the origin window is gone) and the drag path ignoring the origin; `floating-redock.test.tsx` covers the call and the window that comes forward.
