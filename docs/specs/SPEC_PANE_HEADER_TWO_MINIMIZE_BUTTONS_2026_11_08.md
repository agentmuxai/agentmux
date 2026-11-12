# Spec: the pane header's two stacked minimize buttons

**Status:** implemented in PR #7090, approved by the operator 2026-11-12 ("yes lets build it": all five decisions of §10 as recommended) — task #29020 (spec), build #29021. Camper@narko, 2026-11-08. Written against `origin/main` f2569c2ec; every file and line anchor below was checked again against `origin/main` 2b39f059e on 2026-11-12 and still holds. One change since: a pane that leaves the tree (the merge removes the emptied pane) now glides its neighbours into the space and fades the pane's frame (#7036), and "Minimize to header" and restore glide too (#7066), so the two buttons need no motion of their own. Amends decision 4 of `SPEC_MINIMIZE_MERGES_INTO_EXPANDED_NEIGHBOUR_2026_10_30.md` (Shift+click for the chip) and the operations of PR #5884.

The operator asked (relayed by AgentY): the single minimize button in the pane header becomes two stacked buttons. Bottom: **Minimize to header**, the solid underscore, keeps the pane as a header chip. Top: **Merge into next pane** (name approved by the operator), a broken line with two breaks (three segments), same shape and size as the solid one, stacked directly above it.

## 1. What exists today

- One button, `OptMinimizeButton` (`app/block/blockframe.tsx:341`, class `block-frame-minimize`), in `EndIcons` between the view's extra buttons and Maximize/Close. Title "Minimize", glyph FA `regular@window-minimize`. For a minimized chip it is the same slot showing "Restore" (`PaneRestoreGlyph`).
- Shown when `!ephemeral && numLeafs() > 1 && canMinimize()` (`blockframe.tsx:441`; `canMinimize` is `numLeafs - minimized > 1`, i.e. never for the last expanded pane).
- Click: `props.toggleMinimize({ chip: e.shiftKey })` -> `NodeModel.toggleMinimize` (`layoutNodeModels.ts:180`) -> `minimizeNodeToggle` or, for a maximized pane, `collapseMaximizedNode` (`layoutMinimize.ts`).
  - Plain click (no `chip`): `minimizeMerger` = `mergeIntoNeighbour` first (`layoutMinimizeMergeApply.ts:27`: a minimized neighbour, else the nearest expanded sibling after, else before; all tabs appended inactive via `moveBlocksInStack`, the emptied pane removed), and **only if there is no target, falls back to the chip**.
  - Shift+click, or the pane menu's "Minimize to Header" (`pane-actions.ts:195`, `minimizeToHeader()` in `blockframe-hooks.ts:20` = `toggleMinimize({ chip: true })`): the chip, never a merge.
- So both operations the operator wants as buttons already exist; today they are one button plus a hidden modifier. The new work is two controls and the glyphs; the wiring reuses the same two paths.
- "The window-header pane-minimize button" in the request is this button. There is no other pane-minimize in the window header. (The OS window's minimize in `window-controls.{win32,darwin,linux}` minimizes the window and is untouched.)

## 2. What the existing button does now (the explicit answer)

Its **merge behaviour moves to the top button, unchanged in what it does** (`mergeIntoNeighbour`, same target choice, same animation, same focus on the receiving pane). The slot no longer has a combined button: the bottom button is the chip, explicitly, with no modifier. Differences from today:

1. **No silent chip fallback.** Today a plain click that finds no merge target quietly makes a chip. The top button never does that: with no target it is disabled (§6). The chip is the bottom button's job only.
2. **Shift is retired.** Shift+click on either button does the button's own thing. (The Shift gesture existed only to reach the chip; the bottom button replaces it.)
3. A minimized chip's header keeps the **single Restore** button as now (one slot, one control). The pair is only for an expanded pane.

## 3. Layout and sizing

The pair replaces the one button in the same place, same width: the column of the end controls is 26 px wide (`.block-frame-end-icons .ui-button--chrome`, `mixins.scss`). Height: the header is `--header-height` 33 px with 4 px vertical padding and a 1 px bottom border, so the existing 26x26 buttons sit with 3 px above and below.

- **Recommendation: the pair fills the header's inner height**, 32 px (33 minus the 1 px border), as two 26x16 halves with no gap between them (the top half's top corners and the bottom half's bottom corners rounded as a single button's would be; the hover/pressed fill covers one half only). It extends into the header's vertical padding (`margin-block: calc(-1 * var(--space-1))` on the wrapper, `align-self: stretch`), so the two targets are as large as the header allows. The alternative, two 12 px halves inside the padding, is rejected: 12 px-high targets are too small to hit reliably (§7).
- The wrapper is one flex child of `.block-frame-end-icons` (a `div.block-frame-minimize-pair`, `role="group"`, `aria-label="Minimize"`), so the narrow-header tiers hide it as a unit: it is a `> :not(.block-frame-default-close)` child, hidden at `data-fit="min"` like Minimize/Maximize today. The pair adds no width, so the tiers' thresholds do not move.
- Chrome zoom: the header uses `zoom: var(--zoomfactor)`; the pair scales with it, and never exceeds the header's height, so it does not add to the 150 % header clipping that Posa is fixing (#26652).
- Header custom hue (`frame:hue`), light and dark: colour is `currentColor`, hover/pressed use the existing `.ui-button--chrome` rules.

## 4. The two glyphs: identical footprint

Both are SVG, drawn from one component file next to `PaneRestoreGlyph` (`app/element/pane-control-glyphs.tsx`) so they share the box and stroke:

| | Minimize to header (bottom) | Merge into next pane (top) |
|---|---|---|
| Box | 16 x 2 px at 100 % zoom | 16 x 2 px, the same |
| Shape | one solid segment, x 0 to 16 | three 4 px segments with two 2 px gaps: 0-4, 6-10, 12-16 |
| Stroke | `currentColor`, butt caps, `shape-rendering: crispEdges` | the same |
| Position | centred horizontally; vertically centred in its half (y = 8 in the half) | the same, in the top half |

- Same bounding box, so the top line is the bottom line with two gaps cut in: segment edges at whole pixels (4/2 rhythm at 16 px) so they stay crisp at 100 % and are not blurred into a dashed haze. At 125 to 150 % zoom the whole pair scales together.
- The solid glyph is **redrawn as the same SVG** instead of staying the FA font icon, so both lines share exact extents and weight. It is sized to the current FA bar (about 16 px wide); the build measures the live one and matches it before the PR.
- The two lines sit 16 px apart (centre to centre), so the pair reads as two evenly spaced rules.

## 5. Tooltips and labels

- Bottom: **"Minimize to header"** (title and `aria-label`). Sentence case, as the pane menu item "Minimize to Header" (the menu keeps its capitalisation; titles in the header are sentence case).
- Top: **"Merge into next pane"**. When disabled (§6): **"No other pane to merge into"**.
- No shortcut hint: there is no default key for either today (the code comments mention one; none is bound).
- Pane menu: "Minimize to Header" stays; a new **"Merge into Next Pane"** item goes next to it (same visibility rule, disabled when the merge has no target). Command palette: the same two as `pane:minimize-to-header` and `pane:merge-into-next` (no default keys; the person can bind them).

## 6. Wiring each to its operation, and the edge cases

- **Bottom** = `minimizeToHeader(nodeModel)` (unchanged): `toggleMinimize({ chip: true })`, shown under the existing condition (`!ephemeral && numLeafs() > 1 && canMinimize()`), for a maximized pane `collapseMaximizedNode(..., { chip: true })` (ends the maximize, leaves the chip).
- **Top** = a new `NodeModel.mergeIntoNeighbour()` that calls `mergeIntoNeighbour(model, nodeId)` (`layoutMinimizeMergeApply.ts:27`) directly and returns whether it merged; no chip fallback, so the target choice (`findMinimizeMergeTarget ?? findExpandedMergeTarget`), `moveBlocksInStack`, the pill-travel animation, the receiving pane's focus, and a maximized pane ending its maximize are all as in PR #5884. Plus a pure memo `NodeModel.canMergeIntoNeighbour` = a target exists for this node (same two finders, `layoutMinimizeMerge.ts`), the node is not locked, floating or ephemeral. It is the same pure code the click runs, so the enabled state and the click can never disagree.
- **Single pane, or the last expanded pane**: neither button is shown, exactly as the one button is hidden today (`numLeafs() > 1 && canMinimize()`); a floating (torn-off) pane has one leaf and shows neither.
- **Several panes but no merge target** (siblings that are locked, or fully-collapsed groups; a pane in another split is never a target): the pair is shown, the **top button is disabled** with the tooltip "No other pane to merge into", and the bottom still works. Disabled rather than hidden so the pair does not change shape between panes. Disabled uses `aria-disabled="true"` and a muted colour, **not** the `disabled` attribute, so it stays focusable and its tooltip reachable by keyboard; the click is a no-op.
- **Restoring** is unchanged: a merged tab comes back by its pill ("Open in its own pane", dragging the pill out, per the 10-30 spec §3); a chip restores with its Restore button.
- **Agents' own minimize** (the `Pane` tool) and a layout load still never merge; they keep calling `minimizeNodeToggle` without going through the buttons.

Retired: the `chip` option's meaning on the plain `toggleMinimize()` call. After this, callers are explicit: the bottom button, the menu and the palette pass `chip: true`; the top button, the menu item and the palette use `mergeIntoNeighbour()`. A bare `toggleMinimize()` (merge-else-chip) has no UI caller left and is removed with its comment, so nothing can re-create the hidden fallback.

## 7. Keyboard, focus and accessibility

- Two real `<button>`s (`.ui-button--chrome`, as every header control). DOM order: **Merge (top), Minimize (bottom)**, then Maximize, Close: Tab goes top to bottom, then right. No roving arrow-key model.
- Each has its tooltip as `aria-label`; the group is labelled "Minimize". Enter and Space activate, as for any button.
- After **Merge**, focus goes to the receiving pane (`moveBlocksInStack` focuses it) as today: the button that had it is gone with its pane. After **Minimize to header**, focus is where a minimize leaves it today.
- Target size: each half is 26 x 16 px. That is under the 24 x 24 px target guideline (WCAG 2.2, 2.5.8) in height, so both halves have an equivalent control that meets it: the pane menu items (§5) and the palette commands. Stated here so the trade is deliberate; the alternative (one 26 x 26 button) is what the operator asked to replace.

## 8. Tests (build)

- Pure: `canMergeIntoNeighbour` for a row, a column, a split sibling, a minimized neighbour (wins), locked/floating/ephemeral, and the last expanded pane; `NodeModel.mergeIntoNeighbour` never produces a chip, moves every tab, keeps the target's active tab; a maximized pane that merges ends its maximize.
- Component: the pair renders in DOM order Merge, Minimize with the two titles; Minimize calls `toggleMinimize({ chip: true })`; Merge calls `mergeIntoNeighbour`; Shift changes nothing; disabled Merge is `aria-disabled` with its reason and does not merge; a chip header shows only Restore; neither shows for one pane or the last expanded pane.
- Glyph: both SVGs have the same `viewBox`/width/height; the broken one has exactly three segments and two gaps; both draw with `currentColor`.
- Snapshots under `app/tab/__snapshots__/pane-leaf-chrome.*` that contain the header buttons are updated.
- Menu and palette: the new items appear under the same condition and disable together.
- Existing tests that assert the old single "Minimize" title or the Shift behaviour are updated, not deleted.

## 9. Visual check (the build is a visual change)

Built from current `origin/main`; macOS (Clare, starpower) and Linux (Maricon, charlie) look checks, Windows by me. Per platform: dark and light; custom header hue; chrome zoom 75, 100, 125, 150 %; a wide and a narrow pane (the pair hides with the other controls at the narrow tier, Close stays); an expanded pane with a neighbour (both enabled), a pane with no merge target (Merge disabled, tooltip), a maximized pane (both work, the maximize ends), a minimized chip (Restore only), two panes (the last expanded pane shows no pair); the two glyphs side by side at 100 % and 150 % for equal footprint; a click on each, a Tab through the header, and the tooltips. Recorded in the PR body with the commit checked.

## 10. Decisions to confirm

1. **Pair height**: the full 32 px inner height as two 16 px halves (recommended) vs two 12 px halves inside the padding.
2. **Top button with no target**: disabled with a reason (recommended) vs hidden.
3. **Shift+click retired** (recommended; the bottom button replaces it) vs kept as a duplicate of the bottom button.
4. **Menu and palette entries** for Merge (recommended: the equivalent controls that make the 16 px halves acceptable) vs buttons only.
5. **Redrawing the solid glyph as SVG** so both share one footprint (recommended) vs keeping the FA bar and matching the new glyph to it by eye.

## 11. Not in scope

- Default keyboard shortcuts for either operation.
- Changing which pane Merge targets, or the merge animation.
- Remembering the old spot for a merged tab (the 10-30 spec §3 decision stands).
- The OS window's own minimize buttons.
