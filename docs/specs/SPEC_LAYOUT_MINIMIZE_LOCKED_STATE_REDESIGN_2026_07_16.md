# Spec — Pane Minimize as a Locked State (redesign)

**Date:** 2026-07-16
**Type:** Investigation + redesign proposal
**Status:** implemented — §8's display-mode model shipped in PR #2197 (i3 pattern:
one leaf-only `minimized` flag, geometry derived, slip/dissolve deleted),
extended by #2211. §§1-7 record the *rejected* locked-state alternatives and
are kept for that reasoning.

*(Previously marked `SUPERSEDED` with no `Superseded-by:` pointer, which the
status vocabulary requires. There is no successor document to point at: this
spec was superseded by a later section of itself, so "superseded" was simply
the wrong status — the doc describes shipped behaviour.)*
Option B (locks, §6) shipped in #2180, then was retired the same day after the layout
doctor caught cascade arithmetic producing negative sizes live — Option B's locks
faithfully preserved the garbage. External research
(`docs/research/RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md`) confirmed no mature
system stores minimize as in-tree size state; the implemented design is the research's
recommendation §7.3 (i3 pattern: display-mode flag + render-derived geometry).
**Trigger:** User report: a minimized pane can still be resized by dragging the adjacent
resize handle. Minimize is supposed to be a *locked* state; today it is not.
**Prior related work:** `INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`
(dead-space, PRs #2036/#2039), PR #2176 (dissolved-column direction flip).

---

## 1. TL;DR

Minimize is currently implemented as *advisory data inside the flex tree* — marker fields
(`minimizedSize`, `slipMinimize`, `columnDissolve`, `_slipAnchor`) that every other layout
mutation path is individually expected to notice and respect. Almost none of them do. An
audit of every mutation surface (§4) found exactly **two** paths with any minimize
awareness, and one of them (`layoutResize.ts`) is aware **backwards**: it explicitly
*exempts* minimized nodes from the minimum-size guard so drags keep working next to them —
which is precisely what makes a minimized pane freely resizable.

This is the third bug in a row with the same root shape (after the dead-space resurrection
and the dissolved-column direction flip): a "cooperating writers" invariant that every
writer must remember, and doesn't. The fix pattern that already worked twice in this
codebase — enforce the invariant at the write choke point, not in each caller
(`prune_dangling_block_refs`, WRR) — applies directly. §5 proposes:

- **Near term (Option B):** make "minimized ⇒ size is locked and untargetable" an
  enforced write-path invariant on both the TS and Rust sides, plus suppress the resize
  affordance (handle + cursor) on locked edges.
- **Longer term (Option C):** the "more sophisticated algorithm" — take minimized panes
  *out of the flex tree entirely* (a per-column header dock). This eliminates the entire
  slip/dissolve special-case machinery, whose existence is itself a symptom of forcing
  locked panes through a solver built for free ones.

## 2. Reported symptom

With a pane minimized (header-height only), the resize handle on its edge still renders,
still shows a resize affordance, and dragging it resizes the minimized pane like any other.
Consequences:

- The header-height invariant is silently broken: the pane can be stretched to reveal a
  strip of stale content, or crushed below header height (clipping the header), while the
  minimize toggle still reports "minimized" (`minimizedSize` remains set, the node stays in
  `minimizedNodeIds`). UI state and visual state disagree.
- Restore math corrupts: `minimizeNodeToggle`'s restore path returns
  `minimizedSize − currentSize` worth of flex units to siblings. If a drag changed
  `currentSize` in between, the redistribution is wrong and sibling sizes drift on every
  minimize/resize/restore cycle.
- The drag is persisted through the normal resize commit (`CommitPendingAction` →
  backend `ResizeNodes`), so the broken state round-trips through `db_layout` and survives
  restart — same persistence shape as the dead-space bug.

## 3. How minimize works today (current design)

State lives in the layout tree itself (`frontend/layout/lib/types.ts`), mirrored untyped on
the Rust side through `LayoutNode.extra`:

| Field | On | Meaning |
|---|---|---|
| `minimizedSize` | leaf | Set ⇒ node is minimized; stores the *original* size for restore. Current `size` is squeezed to header height. |
| `slipMinimize` | leaf | Row-parent variant: the header "slips" into an adjacent column. |
| `columnDissolve` | column | Whole column dissolved into an adjacent sibling after all its leaves minimized; stores restore context (`targetColumnId`, original row slot). |
| `_slipAnchor` | row | Guard flag telling `balanceNode` to skip its single-child hoist for this subtree. |

Every geometry pass runs `balanceNode` over the whole tree
(`layoutGeometry.ts:49`), and the backend runs the ported `balance_node` +
`prune_dangling_block_refs` at each reducer write. The minimize marker fields survive those
passes only because of targeted carve-outs (`_slipAnchor` for the hoist rule; the
`columnDissolve` direction-flip guard added in PR #2176).

The key property: **a minimized pane is still an ordinary flex node** with an ordinary
`size`, sitting in the ordinary tree, reachable by every ordinary mutation.

## 4. Audit: which mutation surfaces know minimize exists

Grep basis: `minimizedSize|slipMinimize|columnDissolve|minimizedNodeIds` across
`frontend/` and `agentmux-srv/`. Only `layoutMinimize.ts`, `layoutModel.ts`,
`layoutNode.ts`, `layoutResize.ts`, `types.ts` (TS) and `balance_node` (Rust) reference
any of it.

| Surface | File | Minimize-aware? | What can go wrong |
|---|---|---|---|
| Resize handle **generation** | `layoutGeometry.ts:145-189` | ❌ No | Handles render between every sibling pair, including edges of minimized panes. The affordance itself is the bug's front door. |
| Resize handle **drag** | `layoutResize.ts:101-108` | ⚠️ **Backwards** | Explicit exemption: *"Minimized nodes can legitimately be smaller than MinNodeSizePx — skip the guard for them."* Added so drags adjacent to a sub-40px minimized pane aren't rejected — side effect: the minimized pane itself resizes freely. |
| Resize **apply** (frontend reducer) | `layoutTree.ts:394-401` | ❌ No | Applies any `resizeOperations` to any node. |
| Resize **apply** (backend reducer) | `backend/layout/mod.rs:551-571` (`resize_nodes`) | ❌ No | Validates only 0–100 range and node existence; blindly writes `size` onto minimized nodes. Agent/API-driven resizes bypass the frontend entirely. |
| Move / insert / swap / split / delete | `layoutTree.ts` (all handlers), Rust ports | ❌ No | A split can target a minimized pane (spawning a full-size sibling inside a header strip); a move can drop an arbitrary node *into* a dissolved column; swap can teleport a minimized node under a Row where its squeezed size means something else. |
| Cross-tab / floating DnD | `crossTabDrag.ts`, `dragInFlight.ts` | ❌ No | Same class as move. |
| Magnify | `layoutMagnify.ts` | ❌ No | Magnifying a minimized pane shows full content while state says minimized (benign-ish, but same disagreement). |
| Tree normalization | `balanceNode` / `balance_node` | ⚠️ Two carve-outs | `_slipAnchor` (hoist) + `columnDissolve` (direction flip, PR #2176). Each was added *after* a corruption shipped. |
| Backend-driven inserts (agents creating panes) | `handle_layout_insert_node*` | ❌ No | New panes can be seeded into a dissolved column's slot. |

Two structural observations:

1. **The one aware path is aware in the wrong direction.** The `layoutResize.ts` exemption
   exists because a minimized pane's size (~header height ≈ 33px) is below `MinNodeSizePx`
   (40px), so *without* the exemption every drag on an adjacent handle computes a
   below-minimum size for the minimized node and gets rejected — the handle would be dead.
   The patch chose "keep the handle alive" over "keep the lock", because there was no
   concept of a lock to keep. It's a faithful record of what happens when each writer
   improvises its own minimize policy.
2. **The slip/dissolve machinery exists to work around the same absence.** Slip-minimize
   and column-dissolve are both answers to "a minimized pane still occupies a flex slot,
   and the solver will do something ugly with it." They add restore-context bookkeeping,
   two `balanceNode` carve-outs, and an `_undissolveColumn` inverse — all of which are
   themselves new invariants that other writers can (and do) violate.

## 5. Redesign: minimize is a locked state

Target invariant, stated once:

> **I1.** A minimized node's `size` is owned exclusively by the minimize subsystem. No
> other writer may change it, on either side of the wire.
> **I2.** No generic tree mutation (resize / move / swap / split / magnify-restore
> shuffle) may target a minimized node or a dissolved column, nor insert into one.
> **I3.** A locked edge presents no affordance: no resize handle, no resize cursor.

### Option A — point-guards in every caller (rejected as the strategy)

Patch each row of the §4 table individually. This is the "cooperating writers" model that
produced the last three bugs; each new mutation path (there were four new ones added in the
last two months) re-opens the hole. Elements of A that are UX-necessary (handle
suppression, I3) are kept, but as *presentation* of the lock, not as its enforcement.

### Option B — write-point enforcement (recommended now)

Same shape as `prune_dangling_block_refs` (PR #2039): enforce at the reducer/model choke
points every mutation already flows through, so individual callers don't need to know.

1. **Reject locked-target ops at both reducers.**
   - Frontend: `layoutTree.ts::resizeNode` drops any `ResizeNodeOperation` whose target
     has `minimizedSize`/`columnDissolve` set (drop the *pair* — a resize op always comes
     as a before/after pair whose sum is conserved; dropping one side would leak units).
     Same rejection in move/swap/split handlers when the target or destination parent is
     locked.
   - Backend: `resize_nodes` (`backend/layout/mod.rs`) gains the same check via
     `extra` (`minimizedSize`/`columnDissolve` keys), returning a new
     `LayoutError::NodeLocked` — closing the agent/API path the frontend never sees.
2. **Snap-back normalization pass** (belt-and-braces, catches writers that bypass the
   reducers, e.g. a full `SetTree` push of a stale tree — the dead-space lesson): a
   `enforceMinimizedLocks(root)` / `enforce_minimized_locks(root)` pass run exactly where
   `balanceNode`/`prune_dangling_block_refs` already run. For every node with
   `minimizedSize` set, force `size` back to the recorded locked size. This requires
   persisting the *locked* size at minimize time (new field `minimizedLockedSize`,
   set by `minimizeNodeToggle` alongside `minimizedSize`), because header height in flex
   units depends on the column's pixel ratio at minimize time and cannot be recomputed
   from the tree alone. Delta from a snap-back is returned to the nearest unlocked
   sibling, mirroring `_dissolveColumn`'s existing "steal from first child" arithmetic.
3. **Affordance suppression (I3).** `layoutGeometry.ts` handle loop: skip emitting a
   handle when either flanking child is locked (`minimizedSize`/`columnDissolve`).
   This also deletes the backwards exemption in `layoutResize.ts:101-108` — with no
   handle on locked edges, the exemption has nothing left to serve and the
   `MinNodeSizePx` guard becomes unconditional again.
4. **Tests.** Mirror the established pattern: pure oracle tests in
   `frontend/layout/tests/` (resize op on a minimized node is a no-op; handle list
   contains no locked edges; SetTree with a tampered minimized size snaps back), ported
   Rust tests in `backend/layout/tests.rs`, reducer-level tests for the `NodeLocked`
   rejection, red/green verified against reverted guards.

Estimated scope: ~1 day. No persistence migration (new field is additive; old trees
without `minimizedLockedSize` fall back to current behavior until next minimize).

### Option C — structural separation: minimized panes leave the flex tree

The deeper redesign the current glitchiness is pointing at. A minimized pane stops being a
flex node at all:

- Each column (or tab) owns a **minimized dock**: an ordered list of block IDs rendered as
  a fixed-height header strip *outside* the flex solver's domain (a sibling DOM region,
  not a tree node). Minimize = remove leaf from tree (existing `delete_node` collapse
  semantics — already battle-tested by `prune_dangling_block_refs`) + append to dock.
  Restore = re-insert (existing insert-at-index machinery) + remove from dock.
- **Everything in §4 becomes correct by construction.** The solver never sees a locked
  node, so resize *cannot* touch one; there is no edge to suppress; move/split/swap
  cannot target what isn't in the tree. I1–I3 stop being enforced invariants and become
  type-level facts.
- **The entire slip/dissolve apparatus is deleted:** `slipMinimize`, `columnDissolve`,
  `_slipAnchor`, `_dissolveColumn`/`_undissolveColumn`, both `balanceNode` carve-outs,
  and their restore-context bookkeeping. Column-dissolve behavior (headers migrating to a
  neighbor when a column fully collapses) falls out naturally: an empty column's dock
  docks onto the adjacent one.
- Costs: a persisted-schema addition (`dock` list per tab/column) with a one-time
  migration for trees currently containing minimize markers; frontend render work for the
  dock strip; SPEC_864 single-writer treatment of dock membership in the Rust reducer.
  Estimated scope: ~1–2 weeks including migration and both-sides tests.

### Recommendation

Do **B now** — it closes the reported hole and every adjacent one in the §4 table with the
already-proven enforcement pattern, and it is not throwaway: the reducer-level rejection
and affordance suppression survive into C unchanged. Then decide on **C** as its own
spec'd effort; it is the honest fix for the glitchiness class (this is the "more
sophisticated algorithm"), and every future minimize bug that B's snap-back absorbs is
additional evidence for prioritizing it.

## 6. Option B implementation notes (as shipped)

- **"Locked"** = `minimizedSize` (minimized leaf) ∪ `slipMinimize` (slipped header) ∪
  `columnDissolve` (dissolved column) — `isNodeLocked` (TS) / `is_node_locked` (Rust).
- **Lock recording:** `minimizedLockedSize` written at all three squeeze sites (normal
  minimize, slip, dissolve) and cleared at all three restore sites.
- **Guards:** frontend `resizeNode`/`moveNode`/`swapNode`/`splitHorizontal`/`splitVertical`
  reject locked targets (resize rejects the whole before/after pair — applying half would
  leak flex units); Rust `resize_nodes`/`move_node`/`swap_nodes`/`split_impl` return the
  new `LayoutError::NodeLocked`. `move` also rejects a locked *destination* (insertion
  into a dissolved column).
- **Snap-back:** `enforceMinimizedLocks` runs in `updateTree` before `balanceNode`;
  `enforce_minimized_locks` runs at the same three reducer choke points as
  `prune_dangling_block_refs`. Delta repaid to the nearest unlocked sibling.
- **Affordance:** handle generation skips locked edges. This forced a latent-bug fix:
  handle `parentIndex` was `resizeHandles.length`, correct only while every gap got a
  handle — it is now the child-array index. The backwards `MinNodeSizePx` exemption in
  `layoutResize.ts` is deleted (nothing left for it to serve) and a stale-handle guard
  added at resize-context creation.
- **Known limitations (deliberately out of B's scope, motivate C):**
  - The lock is in flex units, so a minimized pane's *pixel* height still scales with
    container resizes (pre-existing behavior). Pixel-true headers require either a
    ratio-aware enforcement pass or Option C's dock.
  - `findNextInsertLocation` (both sides) can still choose an insertion slot inside a
    dissolved column for blind inserts; reducer-guarded paths reject it, but the
    heuristic itself is not minimize-aware.

## 7. Tracking

Consolidated GitHub tracking issue: agentmuxai/agentmux#2179 (created 2026-07-16, this
report attached). Supersedes stale issue #247 (macOS resize-cursor — root-caused against
the pre-CEF WKWebView/tao stack, already flagged stale in triage 2026-06-15, confirmed
working in 0.50.3).

Merged history this consolidates:
- PR #2036 — dead-space point fix (delete-saga frontend notification).
- PR #2039 — `prune_dangling_block_refs` write-path invariant ("WRR-style").
- PR #2176 — dissolved-column direction-flip guard in `balanceNode`/`balance_node`.
- `docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`.

## 8. Final design (implemented 2026-07-16): minimize is a display mode

After the layout doctor (#2184) caught a live cascade computing **negative sizes** —
which Option B's locks then preserved — external research
(`RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md`; i3, tmux, Eclipse, IntelliJ,
FlexLayout, AvalonDock, Dockview, all source-verified) established that no mature system
stores minimize as in-tree size state. The implemented model is the research's
recommendation §7.3, the i3 pattern:

- **State:** one leaf-only flag, `minimized: true`. Nothing else. Stored flex sizes are
  **never touched** by minimize; restore = clear the flag, original geometry intact by
  construction.
- **Geometry is derived, per render pass** (`computeMainAxisAllocation` in
  `layoutGeometry.ts`): a minimized leaf renders as a header chip (header height in a
  Column parent; fixed `MinimizedRowSlotWidthPx` chip in a Row parent, cross-axis clamped
  to header height); a fully-minimized subtree renders as a stacked chip strip (one
  header height per leaf), with expanded siblings absorbing the remainder proportionally
  to their untouched flex sizes. Chips scale down when the container is too small.
- **Deleted wholesale:** `_slipMinimize`, `_dissolveColumn`, `_undissolveColumn`, the
  `slipMinimize`/`columnDissolve`/`_slipAnchor` bookkeeping, `minimizedSize` restore
  arithmetic, `minimizedLockedSize` + `enforceMinimizedLocks` on the frontend, and all
  steal-from-neighbor math. `balanceNode`'s two carve-outs remain as inert pre-migration
  protection; backend `enforce_minimized_locks` remains for unmigrated legacy trees.
- **Kept:** the reducer guards both sides (`isNodeLocked`/`is_node_locked` now keyed on
  the flag + legacy markers) — minimized panes stay untargetable by resize/move/swap/
  split/insert; the last-expanded-pane guard; the collapsed-header reduction; the layout
  doctor (I2 extended to the flag; I4/I5/I6/I7 now legacy-detection).
- **Migration:** `rebuildMinimizedSet` converts legacy state in place at load
  (`minimizedSize` → size restored + flag; `slipMinimize` → flag in place;
  `columnDissolve`/`minimizedLockedSize`/`_slipAnchor` → dropped).
- **Why each historical bug class is now structurally impossible:** direction flips
  can't corrupt what geometry derives (bug 1); there is no squeezed size to resize and no
  handle on chip edges (bug 2); one leaf-only flag instead of four marker fields
  (bug 3, doctor I2 still watches promotions); no arithmetic exists to compute a negative
  size (bug 4).
