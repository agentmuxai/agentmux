# SPEC: Shift+drag group resize — fix borders that move opposite the drag direction

**Date:** 2026-08-17
**Status:** implemented (this branch)
**Author:** Clamk (agent)
**Tracking discussion:** user report, this session — "if I shift+drag any pane
to the right, all the pane borders should move to the right... currently,
sometimes if I shift+drag a pane border in one direction, some of the panes
along that direction move in the opposite direction — is that a design flaw
or intentional?"

Builds directly on `SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md` (PR #2401,
merged `6dba4bc51`) — read that first for the feature's original scope
decisions (Scope A: resize confined to the dragged handle's own parent,
Shift as the modifier). Nothing here revisits those; this only fixes the
distribution math within Scope A.

---

## 1. Verdict: design flaw, not intentional

Confirmed by reading `computeGroupResizeSizes`
(`frontend/layout/lib/layoutResize.ts`) and its spec: the reversed-direction
border movement is a real, unconsidered gap, not a deliberate tradeoff.

- The original spec's §5.2 algorithm distributes the complementary delta
  across **every other sibling under the parent, weighted only by current
  size** — with no regard for whether a sibling sits before or after the
  driven pane in the parent's child order.
- §3's prior-art survey does flag the tmux/i3/sway "T-junction" ambiguity,
  but only to justify confining the feature to one parent's children
  (Scope A vs. Scope B) — it never revisits per-border directionality
  *within* Scope A.
- The 8 shipped unit tests (`frontend/layout/tests/layoutResize.test.ts`)
  only assert total-size conservation and min-size floor clamping. None of
  them construct a case where the driven pane has siblings on *both* sides
  of it — the exact precondition needed to observe the bug — so the
  behavior below was never exercised or asserted on.

---

## 2. Root cause, with a worked example

Row of 4 panes, all under one parent: `A(100) B(100) C(100) D(100)`
(sum 400). User Shift-drags the `B|C` handle to the right by an amount that
shrinks `C` (the driven pane, i.e. `afterNode`) by 40.

Current algorithm: `others = [A, B, D]` (every sibling except the driven
one, regardless of position), each absorbing a share of the 40 proportional
to its own size — 100/300 each ⇒ +13.33 apiece:

| Pane | Before | After |
|------|--------|-------|
| A | 100 | 113.33 |
| B | 100 | 113.33 |
| C (driven) | 100 | 60 |
| D | 100 | 113.33 |

Border positions (cumulative sum from the left):

| Border | Before | After | Δ | Direction |
|--------|--------|-------|---|-----------|
| A\|B | 100 | 113.33 | +13.33 | right — matches drag |
| B\|C (the dragged handle) | 200 | 226.67 | +26.67 | right — matches drag |
| C\|D | 300 | 286.67 | **−13.33** | **left — opposite the drag** |

`D` sits on the far side of the driven pane `C`. Because `D` is entitled to
the same size-proportional share of `C`'s shrinkage as `A` and `B` are, `D`
grows too — and since `D` is *after* `C`, growing `D` pulls the shared
`C|D` border **toward** `C`, i.e. left, even though the user is dragging
right. This happens on any handle that isn't the outermost one in the
group — i.e. whenever the driven pane has siblings on both sides of it.
The more siblings past the driven pane, the more of the give-up they
absorb, and the more visible the reversal is.

---

## 3. Why "exclude the far side" isn't the right fix

The obvious quick fix — only redistribute to siblings positioned before the
driven pane, leave everyone past it untouched — does eliminate the reversed
border, but at a real cost: it silently narrows the feature from "every
sibling in the row moves together" (the original ask and spec's stated
goal, §1 of the original spec) down to "only the near side moves," with no
indication to the user why panes past their cursor stopped participating.
Rejected for that reason — see `docs/retro`-style reasoning below rather
than shipping the narrower behavior silently.

## 4. Fix: two-block proportional scaling, split at the handle

Reframe the redistribution as **two independently-scaling blocks** meeting
exactly at the dragged handle, instead of "one driven node vs. an
undifferentiated pool of others":

- **`beforeBlock`** — every sibling positioned *before* the driven pane in
  the parent's child order (i.e. before the handle).
- **`afterBlock`** — the driven pane itself plus every sibling *after* it
  (i.e. at-or-after the handle).

The same aggregate transfer amount `Δ` the existing code already computes
(`clampedDesired - driven.size`) is applied to the **blocks' totals**, not
to the driven node individually:

- `beforeBlock`'s total changes by `-Δ` (shrinks when the driven pane
  grows, grows when the driven pane shrinks) — exactly mirroring the plain
  2-node baseline's `beforeNode`/`afterNode` relationship, just scaled up
  from one node to a block of them.
- `afterBlock`'s total changes by `+Δ`.
- Within each block, the change is distributed **proportionally to each
  member's own current size** — i.e. each block scales uniformly as a
  unit. The block that's shrinking uses the existing iterative
  floor-clamp-and-redistribute logic (a member clamped to `minNodeSize`
  drops out of that block's pool and its shortfall re-spreads across the
  rest of the same block); the growing block is unconstrained, as today.
- If the shrinking block's combined floor headroom is less than `Δ`, the
  growing block's actual change is capped to whatever was actually
  redistributable — same conservation-safety principle as today, just
  scoped to a block instead of "all others."

### 4.1 Why this fixes the bug

For any block of siblings that all scale by the same proportional factor
(uniform scaling — the definition of how each block redistributes above),
every border *internal* to that block moves in the same direction as the
block's own boundary that's tied to the drag. Sketch: if a block's total
changes by `Δ` and each member `i` changes by `Δ · size_i / blockTotal`,
then the position of any internal border (a partial cumulative sum within
the block) changes by `Δ · (blockTotal − offset) / blockTotal ≥ 0` when
`Δ ≥ 0` — i.e. it never has the opposite sign from the block's own
boundary shift, regardless of how many members are in the block or how
they're sized. Applying this to both `beforeBlock` and `afterBlock`
independently means **every border in the whole group moves in the same
direction as the drag, or doesn't move at all — never backward.**

Re-running the §2 example under this model: `beforeBlock = [A, B]` grows by
the full 40 (not shared with `D`) ⇒ `A=120, B=120`; `afterBlock = [C, D]`
shrinks by 40, split proportionally between `C` and `D` (100:100) ⇒
`C=80, D=80`.

| Border | Before | After | Δ | Direction |
|--------|--------|-------|---|-----------|
| A\|B | 100 | 120 | +20 | right |
| B\|C (handle) | 200 | 240 | +40 | right — tracks the cursor exactly |
| C\|D | 300 | 320 | **+20** | **right — now matches the drag** |

### 4.2 Trade-off: the driven pane's own size is no longer pixel-exact

Under the old model, the driven pane's size always equaled the raw
pixel-computed desired value exactly (it alone absorbed the delta). Under
this fix, the driven pane shares `afterBlock`'s change with whatever
siblings sit past it, so its own size generally lands somewhere other than
the raw pixel value (here, `C=80` rather than the naively-implied `60`).

This is judged the correct trade to make: the invariant that actually
matters for a drag interaction is that **the border under the pointer
tracks the pointer exactly** (verified above — `B|C` moves by the full 40,
matching the cursor 1:1, in every case tested including the floor-clamp
ones). The driven pane's own individual width was never something the spec
called out as needing 1:1 pixel tracking in the group case — only the
handle itself needs that — so giving it up to gain full-row participation
with correct directionality is a net improvement, not a regression against
anything the original spec promised.

### 4.3 Minimize-locked siblings

Unaffected by this change — they're already filtered out of
`groupSiblingStartSizes` before reaching `computeGroupResizeSizes`
(`layoutResize.ts`'s `onResizeMove`, per the original spec's §5.3). A
locked sibling simply isn't a member of either block.

---

## 5. Implementation

Confined entirely to `computeGroupResizeSizes` in
`frontend/layout/lib/layoutResize.ts` — same exported signature
(`siblings`, `drivenNodeId`, `drivenDesiredSize`, `minNodeSize`), so
`onResizeMove`'s call site is unchanged. `drivenNodeId` is now used only to
locate the split point between the two blocks (`siblings.findIndex(...)`);
it no longer identifies a single node that's treated specially in the
distribution math.

Degenerate cases:
- `beforeBlock` empty (driven pane has no siblings before it) — cannot
  happen via `onResizeMove` in production (the driven pane is always
  `afterNode`, which by construction always has a `beforeNode` ahead of
  it), but the pure function falls back to leaving everything but the
  driven pane untouched, matching the original function's "no other
  siblings" fallback.
- `afterBlock` of size 1 (driven pane is the last sibling) — degenerates
  to exactly the original 2-node transfer (`beforeBlock` absorbs the full
  `Δ`, the sole `afterBlock` member gets the full `Δ` too) — this is the
  existing "two siblings" baseline case and is unchanged.

## 6. Testing plan

- Rewrote/added unit tests in `frontend/layout/tests/layoutResize.test.ts`
  for the two-block model, including cases with siblings on *both* sides
  of the driven pane, asserting **border positions** (cumulative sums), not
  just individual sizes — this is what actually encodes the regression
  test for the reported bug (a magnitude-only assertion can't tell a
  correct-direction move from a reversed one).
- Retained floor-clamp and conservation-safety-cap coverage, rescoped to
  operate on a block instead of "all others."
- Live verification (drag a 4+ pane row with Shift held, confirm no border
  moves opposite the cursor) — flagged as a follow-up in this environment
  for the same reason as the original PR (no interactive display/CEF
  runtime in this sandbox); `task dev` started so the requester can verify
  directly.

## 6.1 Follow-on bug found via live use: premature stop before the floor

User report, same session, after raising `MinNodeSizePx` to 128: "if I
shift+drag a pane, it stops resizing before the 128px, why is that?"

Root cause: `computeGroupResizeSizes` computed
`clampedDesired = Math.max(drivenDesiredSize, minNodeSize)` and derived
`totalDelta` from that **clamped** value, before the block split. That was
correct under the pre-4.x single-node model, where driven's own final size
*was* `clampedDesired` directly. Under the two-block model, driven's real
final size is only its **proportional share** of `afterBlock`'s total
change — generally larger than `clampedDesired` whenever `afterBlock` has
other members. Pre-clamping the raw desired value to the floor freezes
`totalDelta` the instant the *unshared* cursor-implied position crosses
128, which happens long before the block's true combined headroom is
exhausted — so the pane being watched stalls above 128 and further
dragging does nothing.

Fix: compute `totalDelta` from the **unclamped** `drivenDesiredSize`
always. Floor enforcement is already handled correctly, per member, inside
`shrinkBlockBy` — the upfront clamp was redundant for the main two-block
path and actively wrong. The clamp is still needed (and correct) for the
degenerate `beforeBlock.length === 0` fallback, where driven genuinely has
no block to share with and its final size really is the directly-floored
value — that branch keeps the `Math.max` clamp, now scoped to only that
case.

Added a regression test (`layoutResize.test.ts`) with a driven pane sharing
`afterBlock` with one other sibling: dragging to a raw desired size far
below the floor must still land driven exactly at 128 (using the block's
real combined headroom), not stall wherever it was when the raw,
unshared position first crossed 128 — and dragging further past that still
holds at the correct floor rather than moving at all.

## 7. Out of scope

- Scope A vs. Scope B (cross-branch pixel-alignment resize) — unchanged
  from the original spec; not revisited here.
- Visual affordance for which panes participate — still an open item from
  the original spec (§7), untouched by this fix.
