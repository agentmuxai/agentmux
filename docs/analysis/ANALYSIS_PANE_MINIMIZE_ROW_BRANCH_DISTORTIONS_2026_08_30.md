# Analysis: pane-minimize distortions when a Row of panes sits between a top and a bottom pane

**Date:** 2026-08-30
**Status:** Analysis only — **no code changed**. Two defects (§2, §3) reproduced
end-to-end through the real `LayoutModel.updateTree()`; one seam flagged for a
product decision (§4); one item investigated and **withdrawn as a defect** — it
turned out to be intentional, regression-locked behaviour (§5).
**Area:** `frontend/layout/lib/layoutGeometry.ts`, `frontend/layout/lib/layoutMinimize.ts`
**Trigger:** operator report — "multiple panes open side-by-side with 1 pane atop
and 1 below; if I start minimizing the middle panes, we get distorted behavior."

**Confidence, per finding — these are not all the same strength:**

| § | Finding | How it was measured |
|---|---|---|
| §2 | Dead space under a minimized Row branch | **End-to-end**: built the tree, called `model.updateTree()`, read `model.additionalProps()` |
| §3 | Resize handles vanish | **End-to-end**: same, read `resizeHandles` off the parent's props |
| §4 | Discontinuous chip reflow | Exported pure functions (`resolveRowSlipTargets`, `computeMainAxisAllocation`) |
| §5 | Zero-height host | **Not a defect** — existing intended behaviour, see §5 |

An earlier revision of this document claimed all findings came from calling
exported functions. That was wrong for two of them, which were measured by
copying non-exported snippets (the Phase C handle predicate, the Phase B host
arithmetic) into a scratch test — and it contradicted this document's own §7.
Codex caught it on the PR. §2 and §3 have since been genuinely re-measured
through `updateTree`; the numbers did not change.

What I still have **not** done is run the app and watch it happen. Which finding
produced the visual the operator actually saw is not established — see §6.

---

## 1. The layout under test

```
┌─────────────────────────────────┐
│              TOP                │   leaf
├─────────┬─────────┬─────────────┤
│    A    │    B    │      C      │   ← "the middle panes"
├─────────┴─────────┴─────────────┤
│             BOTTOM              │   leaf
└─────────────────────────────────┘
```

Tree: `root(Column) → [ TOP, MIDDLE(Row) → [A, B, C], BOTTOM ]`

The important structural fact: **MIDDLE is a Row branch nested inside a Column
parent.** Every finding below traces to that, and the existing test suite has no
case of this shape — its only branch-level minimize regression test uses a
*Column* branch (`layoutMinimize.test.ts`, "minimizedCrossAxisPx: one
header-height for a minimized leaf, N for an N-leaf minimized branch", where
`rightCol = newLayoutNode(FlexDirection.Column, …)`).

Constants used throughout: `HeaderHeightPx = 33`, `gap = 3`,
`MinimizedRowSlotWidthPx = 180`.

---

## 2. Finding A — 72px of dead space once all three middle panes are minimized (root cause)

**Measured end-to-end** — tree built, `model.updateTree()` called, rects read
back from `model.additionalProps()` (800×600 container, gap 3):

| | |
|---|---|
| MIDDLE's rect | top 246, height **108px** |
| Chip heights (A, B, C) | 36, 36, 36 |
| Chip widths (A, B, C) | 183, 183, 183 |
| Bottom of the chip strip | y = **282** |
| Bottom of MIDDLE's rect | y = **354** |
| **Dead space** | **72px** (y 282 → 354) |
| BOTTOM pane starts at | y = 354 |

Same subtree declared as a Column branch instead: allocated 108, consumed 108 —
exact. The defect is direction-specific.

**Root cause.** `minimizedFixedPx` (`layoutGeometry.ts`) computes a
fully-minimized child's extent as:

```ts
if (parentIsRow) return MinimizedRowSlotWidthPx + gapPx;
return countLeafPanes(node) * (HeaderHeightPx + gapPx);
```

`countLeafPanes` is a plain recursive leaf count. It encodes the assumption that
a collapsed subtree's chips **stack along the measured axis** — one header-height
per leaf. That is true only when the subtree's own `flexDirection` matches the
axis being measured.

MIDDLE is a **Row**. Its three minimized children lay out *side by side*, each
one header tall. Its true vertical extent is `1 × (33+3) = 36px`. The formula
returns `3 × (33+3) = 108px`.

The function's own doc comment states the assumption explicitly, and is wrong in
exactly this case:

> "Same formula as `minimizedFixedPx`'s Column-parent branch — a stack's
> required extent doesn't depend on which axis it's being measured for."

A stack's extent doesn't, but **this isn't always a stack.** A Row branch is a
*strip*, and a strip's extent very much depends on the axis.

**Where it shows up.** Both call sites are affected, because
`minimizedCrossAxisPx` just delegates to `minimizedFixedPx(child, false, gap)`:

- Root Column's main-axis allocation → MIDDLE gets 108px of height instead of 36px.
- Phase A's cross-axis clamp → chips inside MIDDLE are 36px tall in a 108px rect.

Net user-visible effect: **an empty horizontal band roughly two header-heights
tall between the chip strip and BOTTOM**, appearing only after the *last* middle
pane is minimized.

**Correct formula.** Extent must recurse on direction, not count leaves:

```
collapsedExtent(node, axis):
  leaf                       → axis === vertical ? Header+gap : MinimizedRowSlotWidth+gap
  branch, flexDirection == axis   → Σ children collapsedExtent(child, axis)
  branch, flexDirection ⟂ axis    → max children collapsedExtent(child, axis)
```

`countLeafPanes` is the `Σ`-always special case — correct only when every branch
on the path is aligned with the measured axis, which is why the all-Column test
passes and this doesn't.

---

## 3. Finding B — minimizing a middle pane deletes the resize handle between its two expanded neighbours

**Measured end-to-end** for `A | B(minimized) | C` via `model.updateTree()`,
reading `resizeHandles` off the parent's own props (800px-wide container):

| | |
|---|---|
| Resize handles generated | **0** |
| Baseline, nothing minimized | **2** |
| Rendered rects (left, width) | A (0, 400), C (400, 400) — **flush at x=400** |
| B's chip | (400, 400) — docked on top of C |

B contributes zero main-axis width (it slips onto C), so **A and C are rendered
flush against each other** — and there is no divider between them anywhere in
the row. The user can no longer resize A against C at all.

**Root cause.** Phase C skips a handle whenever *either* flanking child is
effectively minimized:

```ts
if (isEffectivelyMinimized(prevChild) || isEffectivelyMinimized(child)) continue;
```

With the minimized pane in the *middle*, that predicate kills both candidate
handles — `A|B` (because B is minimized) and `B|C` (because B is minimized) —
leaving the row with none. The rule is right for the chip's own edges; it just
doesn't account for a zero-width slip child collapsing two real edges into one.

**Fix direction.** Generate handles between consecutive *rendered* slots rather
than consecutive array indices: skip over zero-width slip children when choosing
the flanking pair, so `A|C` yields one handle whose `parentIndex` still refers to
A. Note `onResizeMove` looks up flanking children by `parentIndex`, so the pair
it resolves must be the pair actually being resized — this needs care, not just a
predicate tweak.

---

## 4. Finding C — the row's chip layout flips discontinuously on the last minimize

**Measured**, same row, widths `[A, B, C]` in a 1200px row:

| State | A | B | C | How chips render |
|---|---|---|---|---|
| A, B minimized; C expanded | 0 | 0 | **1200** | chips **full-width**, stacked above C |
| A, B, C all minimized | **183** | **183** | **183** | chips **narrow**, side by side |

One toggle takes A's chip from **1200px wide to 183px wide** and moves it from a
vertical stack into a horizontal strip.

**Root cause.** Not a defect in either branch — it's the seam between two
deliberately different mechanisms. `resolveRowSlipTargets` returns a target only
when *some* sibling is still expanded; when the last one collapses it returns an
empty map (documented behaviour), and every child falls back to
`computeMainAxisAllocation`'s fixed-chip path. The two paths produce
qualitatively different geometry and nothing interpolates between them.

**Assessment.** This is the finding I'm least sure is a *bug* — the all-minimized
strip may well be the intended presentation. But it is a large, abrupt, unanimated
reflow triggered by one click, and "distorted" is a fair description of how it
reads. Worth an explicit product decision rather than leaving it as an emergent
seam.

---

## 5. WITHDRAWN — "docked chips crush their host to zero height" is intended behaviour

**An earlier revision of this document listed this as a fourth defect and
proposed adding a minimum host height. That was wrong, and the proposed fix
would have reverted a regression test.**

The behaviour is real — with enough minimized panes docking onto one short row,
Phase B's arithmetic yields a host height of exactly 0:

```ts
const clampedSlipHeight = Math.min(totalSlipHeight, targetProps.rect.height);
const shrunkRect = { ...targetProps.rect,
    height: targetProps.rect.height - clampedSlipHeight };   // → 0 when saturated
```

But it is **deliberate and locked in by a test**.
`frontend/layout/tests/layoutModel.test.ts` (the "scales down chip heights
proportionally when a slip group's total height exceeds the target's space" case,
added for a reagent P1 on **PR #2211**) sets up exactly this saturated slip group
and asserts:

```ts
// The anchor's own content area is fully consumed (0 height left) —
// the whole container is chips when they overflow this badly.
expect(props[anchor.id].rect.height).toBeCloseTo(0, 0);
```

So "the whole row becomes chips" is the specified outcome when the stack
saturates, not an accident. Reserving a host-height floor would break that test
by design.

**What remains, reframed as a product question rather than a bug:** *is
"saturate to all-chips" the behaviour we want when a slip group overflows, or
should the host keep a minimum visible height?* There is a defensible argument
either way — the current rule is at least predictable and has no dead space. Any
change here is a deliberate reversal of #2211's decision and needs to be made as
one, with that test updated intentionally rather than "fixed".

Recorded here rather than deleted, so the next person who notices the
zero-height host finds the reasoning instead of re-opening it.

---

## 6. What is NOT established

- **Which finding the operator actually saw.** A is the best match for "distorted"
  in the described 3-pane scenario (a visible empty band), and B is the best match
  for something feeling subtly broken without being obviously wrong. I did not
  reproduce the report visually and am not claiming to have.
- **Whether any of this reproduces in the running app.** All measurements are at
  the pure-function level. The functions are the ones `updateTreeHelper` calls
  with these exact arguments, but a live check is the honest next step —
  especially since the FLIP/render layer downstream could mask or compound the
  geometry.
- **Whether Finding C is a defect at all** (see §4).
- **Any Column-direction equivalents.** I only tested Row branches under Column
  parents, the reported shape. The mirrored case — a Column branch under a Row
  parent, fully minimized — is presumably affected by the same `countLeafPanes`
  assumption in the other direction, but I did not test it.

## 7. Reproducing the measurements

All measurements came from scratch vitest files, since deleted.

**§2 and §3 (end-to-end).** Copy the preamble of
`frontend/layout/tests/layoutModel.test.ts` (lines 1–99: its imports, the
`@/app/store/global` `vi.mock`, and `createLayoutModel()`), then build the tree
shape with `newLayoutNode`, set `n.minimized = true` on the intended leaves,
assign `model.treeState.rootNode`, call `model.updateTree()`, and read
`model.additionalProps()`. §2 reads each node's `rect`; §3 reads
`props[rootNode.id].resizeHandles`. This exercises the real
`updateTreeHelper` — Phases A, B and C — not a reimplementation of it.

**§4 (pure functions).** `resolveRowSlipTargets` and
`computeMainAxisAllocation`, both exported from
`frontend/layout/lib/layoutGeometry.ts`, called directly.

Constants: `HeaderHeightPx = 33`, gap 3, container 800×600 unless stated.
`createLayoutModel()`'s default bounding rect is 800×600, which is where §2's
absolute y-coordinates come from.

## 8. Suggested order of work

1. **§2 (dead space)** — replace `countLeafPanes` with the direction-aware
   `collapsedExtent`. Root cause of the visible band, self-contained, and the
   suite's blind spot is easy to close: add the Row-branch mirror of the
   existing Column-branch test.
2. **§3 (missing handles)** — handles between adjacent *rendered* slots rather
   than adjacent array indices. Needs care around `parentIndex` and
   `onResizeMove`'s lookup, so it is not a one-line predicate change.
3. **§4 (discontinuous reflow)** — product decision first, implementation only
   if wanted.
4. **§5** — nothing to do unless we deliberately reverse PR #2211.

## References

- `frontend/layout/lib/layoutGeometry.ts` — `minimizedFixedPx`,
  `minimizedCrossAxisPx`, `computeMainAxisAllocation`, `resolveRowSlipTargets`,
  `updateTreeHelper` Phases A/B/C
- `frontend/layout/lib/layoutMinimize.ts` — the display-mode model
- `frontend/layout/tests/layoutMinimize.test.ts` — existing coverage; note its
  branch-level cases are all Column
- `docs/research/RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md`
- `docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md`
- `docs/specs/SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md`,
  `SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md` — the slip/dissolve requirement
