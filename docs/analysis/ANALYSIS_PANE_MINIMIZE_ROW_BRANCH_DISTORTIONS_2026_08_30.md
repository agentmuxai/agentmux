# Analysis: pane-minimize distortions when a Row of panes sits between a top and a bottom pane

**Date:** 2026-08-30
**Status:** Analysis only — **no code changed**. Four distortions identified and
reproduced against the real geometry functions; fix directions proposed but not
implemented or tested.
**Area:** `frontend/layout/lib/layoutGeometry.ts`, `frontend/layout/lib/layoutMinimize.ts`
**Trigger:** operator report — "multiple panes open side-by-side with 1 pane atop
and 1 below; if I start minimizing the middle panes, we get distorted behavior."

**Confidence, stated up front:** every number below was produced by calling the
actual exported functions (`computeMainAxisAllocation`, `resolveRowSlipTargets`,
`minimizedCrossAxisPx`) in a scratch vitest file, not by reading the code and
reasoning. What I have **not** done is run the app and watch it happen. So these
are demonstrated defects in the geometry layer; which one(s) produced the visual
the operator actually saw is not established — see §6.

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

**Measured:**

| | |
|---|---|
| Vertical space the root Column allocates to MIDDLE | **108px** |
| Vertical space MIDDLE actually fills | **36px** |
| **Dead space** | **72px** |

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

**Measured**, for `A | B(minimized) | C`:

| | |
|---|---|
| Resize handles generated | **0** |
| Child widths A, B, C | **600, 0, 600** |
| Baseline (nothing minimized) | 2 handles (`0|1`, `1|2`) |

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

## 5. Finding D — enough docked chips crush their host pane to zero height

**Measured**, 6 panes in a 120px-tall row, 5 minimized onto the survivor:

| | |
|---|---|
| Combined chip height | 180px |
| Row height | 120px |
| `clampedSlipHeight` | 120px |
| **Host pane's resulting height** | **0px** |

The row's only expanded pane renders at zero height and disappears.

**Root cause.** Phase B scales the *chips* down to fit
(`scale = clampedSlipHeight / totalSlipHeight`, added for an earlier reagent P1)
but computes the host's rect independently:

```ts
const clampedSlipHeight = Math.min(totalSlipHeight, targetProps.rect.height);
const shrunkRect = { ...targetProps.rect,
    top: originalTop + clampedSlipHeight,
    height: targetProps.rect.height - clampedSlipHeight };
```

When `totalSlipHeight ≥ rect.height`, `clampedSlipHeight` equals the full height
and the subtraction yields exactly 0. The chips are made to fit; the pane they
docked onto is not given a floor.

**Fix direction.** Reserve a minimum host height and scale the chip stack against
`rect.height − minHostPx` instead of `rect.height`. Requires a product call on
what the floor is — one header height, or something content-bearing.

**Reachability.** Needs ~5 minimized panes docking onto one short row, so it is
less likely than A–C in the reported 3-pane scenario. Included because it is the
same class of defect and cheap to fix alongside.

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

Findings A–D were produced by a scratch vitest file (since deleted) calling the
exported pure functions directly. To regenerate: build the tree shapes described
in each section with `newLayoutNode` from `frontend/layout/lib/layoutNode`, then
call `computeMainAxisAllocation` / `resolveRowSlipTargets` /
`minimizedCrossAxisPx` from `frontend/layout/lib/layoutGeometry`, using
`HeaderHeightPx = 33`, `gap = 3`. The Phase C handle predicate in §3 and the
Phase B host-height arithmetic in §5 are copied verbatim from
`updateTreeHelper`, since neither is separately exported.

## 8. Suggested order of work

1. **Finding A** — replace `countLeafPanes` with the direction-aware
   `collapsedExtent`. Root cause of the visible dead space, self-contained, and
   the existing test suite's blind spot is easy to close (add the Row-branch
   mirror of the current Column-branch test).
2. **Finding D** — host-height floor in Phase B. Small, adjacent to A.
3. **Finding B** — rendered-slot-adjacency handles. Needs care around
   `parentIndex` and `onResizeMove`'s lookup.
4. **Finding C** — product decision first, then implementation if wanted.

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
