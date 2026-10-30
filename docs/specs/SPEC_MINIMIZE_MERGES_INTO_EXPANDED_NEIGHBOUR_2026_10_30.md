# Minimizing a pane next to an expanded pane puts its tabs in that pane

**Status:** active. Approved by the operator 2026-10-30 with all seven recommendations (relayed by AgentY, "approve all recommended"). Amends [SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md](SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md) (§2.1, §2.5, §6).
**Author:** Camper@narko, from the operator's report (2026-10-30).
**Scope:** `layoutMinimize.ts` (`minimizeNodeToggle`), `layoutMinimizeMerge.ts` (target choice), `layoutMinimizeMergeApply.ts`, the pane-tab context menu.

The operator's words: *"2 panes side by side, minimize one should result in a single pane with both pane tabs in one. But I still see the stack."* The approved rule merges a minimized pane only into a pane that is **already minimized**, so with two panes (one expanded, one minimized) there is nothing to merge into and the minimized one stays a header chip stacked over the other. That is what the operator sees.

## 1. What exists today

- **A minimized pane** is a leaf with `minimized: true`: a header-only chip, its content laid out but hidden. In a Row it docks onto the nearest expanded sibling (right first, else left) and stacks over it; in a Column it lies along the column. Restoring clears the flag and the pane is back in its own tile.
- **A pane with several tabs** is a stack: one active member shown, the rest **background members**, kept mounted and dormant (`SPEC_AGENT_PANE_TAB_KEEPALIVE`), each with a pill in the strip that shows its work state (progress line, unseen-turn dot, activity flash).
- A minimized pane's tabs are already treated like background tabs by the visibility signal (`pane-minimized.ts`: dormant, not on screen). The agent behind a tab runs in srv whatever the frontend shows.
- **Merging panes** exists: `moveBlocksInStack` appends a pane's tabs to another's strip and removes the emptied pane as a move; it is what drag and drop and the merge on minimize use.

So *"one pane whose strip has both tabs, the minimized one not showing"* is exactly a stack with a background member. No new per-tab "minimized" state is needed.

## 2. The change

1. **Result.** When the person minimizes a pane and no minimized neighbour exists to merge into (the §2 rule of the earlier spec), its tabs move into the nearest **expanded** pane's strip: appended at the end, in their order, **inactive**. The target's own active tab stays showing, the minimized pane's tile is removed and its space goes to the others, as for a minimize. One pane, one strip, both tabs.
2. **Priority.** The earlier rule first (a minimized neighbour, tree-adjacent or stacked over the same pane); only when there is none, the expanded neighbour. A minimize therefore leaves a chip only where no neighbour can take the tabs: it no longer does for the person's own minimize of a pane in a layout of two or more expanded panes.
3. **Which expanded pane.** The same pane the chip would dock onto: the nearest expanded sibling after it (right or below), else before it (left or above). If that sibling is a split of several panes, the leaf of it nearest to the minimized pane (its leftmost for a Row, topmost for a Column, found down the near edge). Only siblings of one split, as before; a pane in another split is not a target.
4. **Same rules as before.** Only the person's minimize (button, shortcut, pane menu, palette). An agent's `Pane` minimize, a layout load and a restore never merge. A locked pane, a floating pane and the last expanded pane do not. A maximized pane minimized from the header ends the maximize first. Focus lands on the receiving pane (`moveBlocksInStack` focuses it). The bubble animation is the one built for the first merge.

## 3. Restoring

A tab that went into a pane comes back in one of these ways:
- **Click its pill:** it becomes the active tab of that pane (the other tab goes to the background). This is the "restore the minimized tab" the operator will reach for first, and it already works for any background tab.
- **Show it as its own pane again:** a new item on the pill's menu, **"Open in its own pane"**, splits the tab out next to the pane it is in (to the right of a Row, below a Column) and focuses it. Dragging the pill out to a split does the same, as today.
- **Back to the old spot:** not remembered. Where the old tile was, and its size, is not worth storing: the layout has usually changed since (the earlier spec chose the same, §2.4 and its question 3). The new menu item puts it next to its host.

## 4. Columns and three or more panes

- **Column:** the same, vertically: a pane minimized in a Column goes into the pane below it, else the one above.
- **Three or more:** `[A | B | C]`. Minimize A: A's tabs go into B (`[B+A | C]`). Minimize C: into B (`[A | B+C]`). Minimize B: into C (`[A | C+B]`), else into A if B is last. Minimize A and then C: both go into B, one strip, `[B+A+C]`: the first minimize leaves no chip, so the second has no chip to stack with and goes into the same expanded pane.
- **A pane with several tabs** brings all of them, in strip order. The target's strip scrolls to show what arrived when it overflows, as in the first merge.
- **Chips that already exist** (an agent's minimize, a layout saved before this, a restore) stay chips and still take a person's later minimize, as the earlier spec says.

## 5. Focus and the agents behind the tabs

- **Focus:** the receiving pane is focused, with its own active tab, so keys go to what the person was already looking at, not to the pane that just went away.
- **Agents:** nothing stops. A minimized agent already runs in srv while hidden; in its new home it is a background tab: kept mounted and dormant, its pill shows the progress line while it works and the unseen-turn dot when a turn finished, and an activity flash pulses the pill. Clicking the pill shows its transcript. A question or a failure in it surfaces as it does for any background tab. Closing the tab closes that agent as today (with the busy-agent prompt).
- **Terminals, browsers and other panes:** the same as a background tab in a stack today (dormant, kept alive where their type is).

## 6. Decisions (each with my recommendation)

1. **Should the person's minimize of a pane with an expanded neighbour merge into it?** *Yes* (the operator's example). Consequence the operator should know: from then on their own minimize almost never leaves a chip.
2. **Result: tabs in the neighbour's strip, inactive, the pane's tile gone** (this spec), **or a chip kept but its tabs also drawn in the neighbour's strip?** *The first.* The second keeps restore-in-place but needs the chip row hidden, geometry that gives its space back, and a pill that belongs to another pane in every strip: a much larger change for the same picture.
3. **Restore:** click the pill, plus **"Open in its own pane"** on the pill menu, **no memory of the old spot.** *Recommend that.* The alternative, remembering the old neighbour and size per block, is more storage and goes stale.
4. **Keep a way to get a chip:** *Shift+click on Minimize (and a pane menu item "Minimize to header") keeps today's chip,* for someone who wants a header-only row that stays in its place. Otherwise there is none for the person's own minimize.
5. **Which neighbour:** after (right/below) first, else before, as the chip docks today. *Recommend that.*
6. **Neighbour is a split of several panes:** the nearest leaf to the minimized pane. *Recommend that,* over "the focused one" (it is unpredictable) or "refuse".
7. **Order against the first rule:** a minimized neighbour wins over an expanded one. *Recommend that,* so tabs that were hidden together stay together.

## 7. Verification (when built)

- **Unit** (pure over trees): the expanded-target choice for a Row and a Column, with a split sibling, with none after, with a minimized neighbour present (it wins); a minimize with a target moves every tab, keeps the target's active tab, leaves no `minimized` leaf; locked, last expanded, floating, agent minimize and layout load never merge; "Open in its own pane" splits the tab out to the right of a Row and below a Column.
- **Component:** the arrived pills in the strip, inactive, with the landing class; the pill menu item.
- **Live** in my own task dev build: two panes side by side, minimize one; three panes; a Column; a pane with three tabs; an agent tab (running) after the merge; screenshots and a recording.

## 8. Not in scope

Merging expanded panes by any other act; changing an agent's or srv's minimize; remembering the old tile.
