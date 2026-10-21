# Retro: Kanban Backlog cards drawn over each other (2026-10-21)

**Status:** retro
**Trigger:** the operator: "i am noticing abberitions on the kanban. Entries overlapping eachother."
**Outcome:** root-caused and reproduced outside the app. One CSS rule from #5235 (`min-height: 32px` on `.kanban-group`) lets each Backlog group shrink to 32 px when the Backlog is taller than its column, so a group's cards spill over the next group's. Fixed in #5294: `flex: none` on `.kanban-group`, and every item in a column's list kept from shrinking.
**Author:** Loap-3@narko
**Verified against:** `agentmux` `main` @ `a144c6bf2`, the operator's Kanban (Warden ▸ Kanban, 136 tasks in the Backlog), and a standalone page with the same rules rendered in headless Edge.

## TL;DR

The Backlog column lists one group per agent ("agent2-6 · 11 waiting"), each a flex item in the column's scrolling list. The other columns list cards directly. A flex item shrinks to fit its container unless its minimum height stops it, and by default that minimum is its content's height. `.kanban-group` overrode the default with `min-height: 32px`, meant only as a floor so that an empty group stays a drop target. That also became the group's **minimum**. When the Backlog's groups are together taller than the column, the list shrinks every group towards 32 px instead of scrolling. Each group's cards keep their own height and overflow the shrunken group, so they're drawn on top of the next group's header and cards. The list still scrolls, but only by the little height left over.

In Progress, Needs you, Review and Done have no groups. Their cards have no explicit `min-height`, so they keep the default minimum and never shrink. That's why only the Backlog shows it.

## What the operator saw

On the live board the Backlog held 136 tasks across a dozen agents' groups. Cards' titles printed over the next agent's group header, and lines like "#139 … certificate", "agent2-6 waiting, busy on #24" and "korp 3 waiting" sat on top of each other. The other columns looked right.

## Root cause

`frontend/app/view/warden-kanban/kanban.scss`, from #5235:

```scss
.kanban-column__list {          // the column's scrolling list
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    min-height: 60px;
}

.kanban-group {                 // one per agent in the Backlog
    display: flex;
    flex-direction: column;
    min-height: 32px;           // ← the cause
}
```

- A flex item's default `flex-shrink` is 1. Its default `min-height: auto` means "don't shrink below my content", which is what keeps the cards in other columns whole.
- An explicit `min-height` replaces that automatic minimum. With `min-height: 32px`, the browser may shrink a group anywhere down to 32 px.
- The list has a fixed height: the column is a flex column inside the board's grid, and the board fills the pane. When the groups' natural heights add up to more than that, the list shrinks every group in proportion before it lets anything overflow. A group shrunk below its content overflows downward, and `.kanban-group` doesn't clip it (nor should it), so its cards paint over the next group.

The `32px` was meant as a floor: an agent's group with no tasks left should still be tall enough to drop a card on. It never needed to be the group's minimum while shrinking.

### Reproduction

A standalone page with the same three rules (`.kanban-column__list`, `.kanban-group`, a plain card), a 300 px column and four groups of three cards, rendered in headless Edge:

- **As on main:** each agent's header lands on the previous agent's cards. "korp 3 waiting" prints over agent2-6's first card, "naki-naki-2 3 waiting" over korp's, and so on, the same pattern as the operator's board.
- **With `.kanban-group { flex: none; }`:** the groups keep their height, agent2-6's three cards stack cleanly, and the list scrolls to korp's group below.

## Why it got through

1. **No layout in the tests.** The Kanban's tests are vitest in jsdom, which computes no layout. They check which column and group a task lands in, never how tall anything is drawn.
2. **The dev-build check used three tasks.** The section was looked at in a `task dev` build with three test tasks (two in Done, one in Backlog), and later only in that build. With that few, the Backlog never outgrew its column, so nothing had to shrink.
3. **The rule came in with the drop target.** `min-height: 32px` arrived with the drag-and-drop work in the same PR, to keep empty groups droppable. Its interaction with flex shrinking wasn't considered, and review (muxreview, Codex) read it as a harmless floor.
4. **It needs real volume.** It only shows once the Backlog is taller than the pane: dozens of tasks, which the board reached through dogfooding (136 now). Wider panes and fewer tasks hide it.

## Fix (#5294)

In `kanban.scss`:

```scss
.kanban-group {
    flex: none;          // a group is as tall as its cards; the list scrolls instead
    ...
    min-height: 32px;    // still a floor for an empty group's drop target
}
```

To guard the other columns the same way, `.kanban-column__list > *` also gets `flex-shrink: 0`. Today their cards are safe only because none sets a `min-height`, and the next rule that does would break them the same way.

Lanes (#5260) aren't affected: inside a lane the list is `overflow: visible` with no fixed height, so nothing is squeezed. With the fix, the full board and lanes behave alike.

## Prevention

- **Check a list with many items, not three.** Before calling a scrolling list done, look at it holding more than fits. For the Kanban, load a dev build's queue with 50+ tasks across several agents. The global task queue is shared, so file them as held tasks for a non-existent agent and cancel them afterwards, as during the first dogfood.
- **Rule of thumb for this codebase's SCSS:** an item inside a scrolling flex list gets `flex: none` (or `flex-shrink: 0`). An explicit `min-height` on a flex item is a shrink limit, not just a floor.
- **A layout test for the board.** A Playwright or CEF-screenshot check of the Kanban with a seeded long Backlog would have caught this. The repo's visual checks don't cover Warden sections yet; that is a separate decision.

## Timeline (2026-10-20 → 21, local time)

- 2026-10-20: #5235 merges the Kanban section, with `.kanban-group { min-height: 32px }`.
- 2026-10-20: the section is first checked in a dev build with three tasks.
- 2026-10-21: the board reaches 136 Backlog tasks in normal use. The operator reports overlapping entries; root-caused, reproduced, this retro written, and fixed in #5294.
