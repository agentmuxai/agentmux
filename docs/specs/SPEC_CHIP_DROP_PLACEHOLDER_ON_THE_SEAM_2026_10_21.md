# Spec — A dragged minimized pane's drop placeholder sits on the seam it is inserted at

**Date:** 2026-10-21
**Author:** Maricon@charlie
**Type:** Spec (a fix to `SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md` §3 A)
**Status:** implemented
**Requested by:** the operator: *"The ghost should land with 50% above, and 50% below where it will land, which follows the existing pattern. but with your collapsed pane, the ghost appears offset. For example, when dragging over to another collpased pane, the dragged collpased pane's ghost lands directly atop. it should land 50% off, bc it will insert."*

## 1. The existing pattern

While a pane is dragged, the drop placeholder (`getPlaceholderTransform`, `frontend/layout/lib/layoutModel.ts`) shows where it will be inserted. For an expanded pane it is half the size of the pane it is placed against, and:

- **between two panes**, it is centred on the seam between them: from ¾ of the pane before the seam to ¼ into the pane after it, half on each side;
- **at the first or last place** of a column or row, it is flush inside that edge, since there is no pane on the other side.

## 2. What is wrong

For a minimized pane (a chip), #5267 gave the placeholder the chip's own size, but put it flush against the near edge *inside* the target pane: its bottom (or right) edge when inserting after it, its top (or left) edge before it. Between two chips the placeholder is then exactly the size and place of the target chip, so it reads as dropping onto that chip (a swap), not between two chips. Over an expanded pane it sits at that pane's bottom edge, not on the seam.

## 3. The change

The chip-sized placeholder takes the existing pattern's position:

1. **Between two panes**, it is centred on the seam: `seam − size/2`, along the parent's direction (top in a column, left in a row).
2. **At the first or last place**, it stays flush inside the edge, as the expanded placeholder does.
3. Its size is unchanged: the chip's size, never more than the target pane's.

The geometry moves into a pure helper, `chipPlaceholderRect` (`frontend/layout/lib/chipPlaceholder.ts`), called from `getPlaceholderTransform`. The swap placeholder (a chip dropped on a pane's centre) is unchanged: a swap takes that pane's slot, so it stays at the slot's top.

## 4. Tests

`frontend/layout/tests/chipPlaceholder.test.ts`:

1. Between two chips in a column, the placeholder straddles the seam (top = seam − 18 for a 36 px chip).
2. At the first place it is flush with the top; at the last, flush with the bottom.
3. After an expanded pane with a pane below it, it is centred on the seam.
4. In a row, it straddles the vertical seam with the chip's width.
5. It is never bigger than the target pane.

## 5. Manual check

A column of three chips plus an expanded pane. Drag the bottom chip between the first two: the placeholder sits half over each, not over the first. Drag it above the first: flush at the top. Drag it below an expanded pane that has a pane under it: on the seam.
