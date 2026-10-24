# Spec — A minimized pane dropped on another minimized pane goes to the nearer seam

**Date:** 2026-10-24
**Author:** Maricon@charlie
**Type:** Spec (a fix to `SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md`)
**Status:** implemented
**Requested by:** the operator, on v0.59.41: *"if I drag a minimized pane over another minimized pane (even one not at the window edge) the ghost appears directly above the underlying pane, ...should be in the middle between the 2"*

## 1. What is wrong

`SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md` put the chip-sized placeholder on the seam of the insert the drop resolves to. That holds only when the drop resolves to an insert between two panes. A chip is about 36 px tall, and `determineDropDirection` divides it like any pane:

- the centre fifth of both axes is **Center**, a swap, whose placeholder is the target's own slot, so it covers the chip;
- the diagonals leave the two ends of a wide chip as **Left** and **Right**, which split the chip itself (the dragged chip goes into the target), and the placeholder is then flush over the target chip;
- only a thin band above and below resolves to the seams that the 2026-10-21 spec handles.

So most of a chip's area never reaches the seam case, and the ghost sits on the chip under the cursor.

## 2. The change

When the pane being dragged is a chip and the pane under the cursor is a chip in the same tree, the drop direction is the nearer seam along the parent's axis, from where the cursor is inside the target chip:

- in a column (a strip of chips), the upper half is **Top** and the lower half **Bottom**;
- in a row, the left half is **Left** and the right half **Right**.

That is a plain insert between two panes, so the existing placeholder rule applies: centred on the seam, flush inside the edge at the first or last place. Nothing else changes:

- a chip over an expanded pane, and an expanded pane over a chip (still a swap at the centre), use the generic zones;
- a pane dragged from another tab uses the generic zones (srv's redock can't place it in a strip);
- a drop that would not move the chip (its own neighbouring seams) still shows no placeholder.

`chipOverChipDirection` (`frontend/layout/lib/chipPlaceholder.ts`) is a pure function of the tree, the two node ids, the target's rect and the cursor. `OverlayNode` and the dead-spot fallback in `tilelayout-shared.tsx` call it before `determineDropDirection`. `layoutModel` is not touched.

## 3. Tests

`frontend/layout/tests/chipPlaceholder.test.ts`, "a chip dropped on another chip", through `computeMoveNode` and `chipPlaceholderRect`:

1. In a column, the upper half of the middle chip, at the centre, the ends and the middle of its width, gives Top, a move to index 1 and a placeholder straddling the seam above it.
2. The lower half gives Bottom and the seam below.
3. At the strip's first and last place it is flush inside the edge.
4. In a row, the left and right halves pick the nearer vertical seam.
5. It applies only to chip over chip, inside the chip, in the same tree.

## 4. Manual check

A column of three minimized panes and a fourth to drag. Drag it over the middle one: the upper half shows the ghost half over the first and half over the middle chip; the lower half, half over the middle and half over the third. Drag it over the top chip's upper half: flush at the top.
