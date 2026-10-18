# RCA: closing one agent tab of a pane leaves a "(No View)" tab

**Date:** 2026-10-17
**Author:** AgentY@narko
**Status:** retro. Fixed in the PR that adds this file (the ClosePane saga removes the closed tabs from srv's
own layout tree).
**Seen in:** a `task dev` build of `main` with the quick-fork history work (#5066) on top, Windows. The bug
itself is on `main` since #4946 (2026-10-15), and in releases since v0.59.27.

## 1. What happened

The operator quick-forked an agent several times, so one pane held five agent tabs (the parent and four
forks). Closing two of the fork tabs with their × didn't remove them: each tab stayed in the tab strip,
labelled "(No View)", with an empty pane behind it.

## 2. Evidence

- **srv log, 22:24:11 and 22:24:14:** each close ran the normal agent teardown (`agent_shutdown ...
  outcome: exited`, `dropped tracker on pane close`). The agents stopped and their blocks were deleted.
- **Frontend log, right after each `ClosePane`:** `MuxObj deleted block:<id>`, then
  `[layout] the local layout differs from srv's stored one; showing srv's`.
- **No `[BUG-TRACE] onNodeDelete` line:** the closes went through the shutdown-log flow
  (`closeWithShutdownLog`, `ClosePane(blockIds, true)`), not through `closeBlockInStack`'s
  `pane.removeTab` + `ClosePane`.
- **The stored layout afterwards** (read-only, from the dev instance's `objects.db`): the pane's
  `blockStack` still listed both closed blocks, and the visible one (`activeBlockId`) was one of them, but
  neither block existed any more. A tab whose block is gone has no `view`, and `paneTabLabelFor` labels
  that "(No View)".

## 3. Root cause

Two changes, each correct alone, disagree about who removes a closed tab from srv's layout.

1. **The shutdown-log close (#3784, SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5).** An agent pane's × keeps the
   pane on screen, covered by the shutdown log, and calls `ClosePane(blockIds, frontend_waits: true)`. When
   the pane survives (one tab of several), the saga deletes the block and queues a frontend `delete` for
   it, but leaves srv's tree alone. Its header said why: "the frontend has already pushed the shortened
   stack". That was true of `closeBlockInStack`, which sends `pane.removeTab` first. It was never true of
   this flow, where the frontend waits. The gap was harmless while the frontend's own edited tree was what
   got saved.
2. **Strong reducer authority (#4946, 2026-10-15).** srv's stored tree became the authority: the frontend's
   edits reach srv as intents, and when the frontend's tree differs from srv's stored one, the frontend
   shows srv's.

Together: the frontend dropped the tab on the queued `delete`, compared its tree with srv's, found srv's
still had the tab, and showed srv's tree. The closed tab came back, with nothing behind it.

**Why forks surfaced it:** any agent tab closed out of a multi-tab pane hits this. Quick Fork is the usual
way to get one, and the fork work made it quicker to stack several.

## 4. Fix

`sagas/close_pane.rs`: when the pane survives, the saga dispatches `LayoutDeleteNodeByBlock` for each closed
(or already-gone) member before queueing any frontend `delete`, as `delete_block.rs` already does. srv's
tree loses just those tabs and keeps the pane. Where the frontend's `pane.removeTab` already removed a
member, removing it again changes nothing ("a block in no pane is a no-op"), so both close flows end the
same way.

**Tests:** both one-tab-of-a-kept-pane tests (the waiting frontend and the historical flow) now check srv's
stored tree: only the sibling is left, and it is the visible tab. The waiting-frontend test fails without
the fix.

## 5. Follow-ups

- **Layouts already left in this state:** the reducer prunes dangling stack members whenever it writes a
  layout tree (`prune_dangling_block_refs`), so the next layout edit in an affected tab should clear its
  "(No View)" tabs. Not verified on an affected install.
- **The saga header assumed what one caller did.** It now says the saga removes closed tabs itself, whichever
  frontend flow closed them.
