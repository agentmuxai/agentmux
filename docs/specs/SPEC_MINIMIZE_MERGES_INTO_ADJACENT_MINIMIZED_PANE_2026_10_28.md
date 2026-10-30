# Minimizing a pane next to a minimized pane merges its tabs into that pane

**Status:** active. Approved by the operator 2026-10-28 ("yes" to the four recommended answers in §4); built in task #8675 by Camper-2@narko.
**Author:** Camper@narko, at the operator's request (2026-10-28).
**Scope:** [`frontend/layout/lib/layoutMinimize.ts`](../../frontend/layout/lib/layoutMinimize.ts) (`minimizeNodeToggle`), [`layoutStack.ts`](../../frontend/layout/lib/layoutStack.ts) (`moveBlocksInStack`), [`layoutSlip.ts`](../../frontend/layout/lib/layoutSlip.ts) (what a minimized pane docks onto), [`frontend/app/element/PaneTabStrip.tsx`](../../frontend/app/element/PaneTabStrip.tsx) (the landing animation).

The operator's words (10-28): *"If a user minimizes a pane and that pane ends up adjacent to another minimized pane, collapse the pane tabs in one. They'd just bubble into the already minimized pane. Add a quick animation (same as pane tab dnd)."*

## 1. What exists

- **Minimize is a display mode**: a minimized pane is a leaf with `minimized: true` and nothing else; its size is never touched, and restore is clearing the flag (`layoutMinimize.ts`). Its header renders as a chip: in a Row it docks onto the nearest expanded sibling (right first, else left; `resolveRowSlipTargets`), in a Column it stacks along the column.
- **Merging panes already exists as drag and drop**: `moveBlocksInStack(model, blockIds, targetBlockId, activateBlockId)` appends every tab of one pane to another's strip in layout order, as one local edit plus one `pane.moveTabs`; a pane it empties is removed as a move, never a close (agents and their state travel with their block). A drop plays the **landing bounce** on the pills that arrived (`.pane-tab--landing`, `LANDING_BOUNCE_MS` = 400 ms).
- **No two minimized panes are ever merged.** Each stays a chip of its own, so a person who minimizes five panes gets five chips.
- The motion primitives Agent1 is building (#8672, `mergeInto`) are not in the code yet.

## 2. The change

1. **Adjacent** means: two leaves that are siblings in the same split (the same Row or the same Column), with no expanded pane between them, both minimized. A fully-collapsed branch (a nested split whose leaves are all minimized) is a group, not a pane, and is not merged into. A pane in another split is never adjacent, however close it looks (question 1). A run of minimized siblings is therefore one candidate group. **In a Row, chips stacked over the same expanded pane are adjacent too** (2026-10-30): a minimized pane in a Row docks onto the nearest expanded sibling, so a chip on each side of one expanded pane both stack over it, and are neighbours on screen though that pane sits between them in the tree. The pane being minimized takes the nearest such chip as its target when it has no tree-adjacent one. In a Column chips lie along the column, so only the tree-adjacent rule applies.
2. **When the person minimizes a pane next to a minimized one, it merges instead of collapsing.** `minimizeNodeToggle` looks for the target before it commits: the nearest minimized sibling before it in the split (above or left), else the nearest after. With a target, it calls `moveBlocksInStack` (all of the pane's tabs, appended at the end of the target's strip, in their strip order) and does **not** set `minimized`: the emptied pane is removed as a move and its space goes to the expanded panes, as it does for a minimize. Without a target it minimizes as today.
3. **Only the person's act merges.** The header's Minimize button, the keyboard shortcut, the pane menu and the palette go through `minimizeNodeToggle` and merge. An agent's `Pane` minimize, a layout restore, an expanded pane being restored and minimized again by loading a saved layout, and a pane that was already minimized before this change never merge on their own: the next minimize of a pane that has a minimized neighbour is what merges (question 2).
4. **Restoring and un-merging.** Expanding the merged pane expands it with all its tabs, as for any pane with a stack. A tab comes back out by dragging its pill, as today (to a split, to another pane). There is no undo (question 3).
5. **What does not merge:** a pane that cannot minimize (the last expanded pane), a locked pane (`isNodeLocked`) and a floating pane. A maximized pane minimized from the header (`collapseMaximizedNode`) ends the maximize first and then follows the same rule.
6. **Focus** lands on the minimized pane that took the tabs (`moveBlocksInStack` focuses the pane that received them), as a plain minimize leaves focus on the chip it minimized; the merged pane's visible tab is the target's, unchanged.

## 3. The animation

The merged pane's pills **bubble into** the target's strip: each pill of the merging pane travels from where its header was to the end of the target's strip and lands with the existing **landing bounce**, the feel of a pane-tab drop. The target's strip scrolls to show what arrived when it overflows (the reveal that selecting a tab does).

- **How:** the landing bounce is today a state of the strip a drop handler sets for the pills it dropped. The build adds a small way to say "these pills just arrived" to a strip from code (a registry beside `pane-strip-registry.ts`), which the merge calls with the moved block ids; the travel itself is a FLIP of each pill from its old rectangle. If Agent1's `mergeInto` (#8672) is ready when the build starts it supplies the travel and the bounce for both this and drop; otherwise the PaneTabStrip landing animation is extracted into a shared helper both use.
- **Motion policy:** the app's, category `transition` (`shouldReduceMotion("transition")`), not the OS setting. With motion off the tabs appear in the target at once.

## 4. Questions for the operator

1. **Adjacent across splits?** Only siblings of one split merge. *Recommend that:* a chip in another split is a different place on screen, and merging across would move tabs a long way.
2. **Merge only on the person's minimize?** Yes as in §2.3. *Recommend that:* a load or an agent's own action should never rearrange the person's tabs.
3. **Undo?** None; dragging a tab out is the way back, and the animation shows where they went. *Recommend that,* unless you want a "Merged 3 tabs into <pane> — Undo" notice (it would need the layout to remember the old pane for a few seconds).
4. **Which side?** The nearest minimized neighbour before (above or left), else after. *Recommend that:* new tabs go to the end of an existing group, as a new tab does elsewhere.

## 5. Verification

- **Unit** (pure, over layout trees): the target choice for a Row and for a Column, with and without an expanded pane between, with a nested fully-collapsed branch, and with none; a minimize with a target moves every tab (in strip order) and leaves no `minimized` leaf behind; a minimize without one is unchanged; a locked pane, the last expanded pane and a locked pane do not merge; an agent's minimize and a layout restore never merge.
- **Component:** the merged tabs show in the target strip with the landing class, and the strip reveals them; with motion off no animation runs.
- **Live** in my own task dev build: two stacked panes in a Row and in a Column, minimize one next to the other, screenshots of the strip before and after, and a recording of the bubble.

## 6. Not in scope

Merging expanded panes; merging when a pane is *restored*; automatic grouping of chips by agent or type; the srv-side minimize used by agents.
