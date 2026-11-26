# Back and Forward follow the whole trail: pane selections and link-driven moves, one history per window

**Status:** active. Approved by the operator; built in #48747.
**Date:** 2026-11-26.
**Requested by:** the operator: refine how the Back button works, especially after clicking links. Today it follows the selected pane. A click on an in-app link (an agent's name, a task that opens the Kanban) should be a step of the same trail, so that the mouse's side buttons walk the person back through what they did: select pane A in tab 1, follow a link to a task (the Kanban opens on it), click an agent inside the Kanban, then select pane B in tab 1; Back goes to the agent, then the task, then pane A, and Forward replays. Think through the edge cases and the best practice.
**Builds on:** `SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md` (the pane selection history, built in #5209), `SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md` (the one executor every link goes through), `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md`.

## 1. What is wanted

One trail per window of the places the person has been, in the order they went there, walked by Back and Forward (the mouse's side buttons and the two keyboard commands, which are already one history). A place is a pane in a tab, and where the pane's view stands inside itself: the Kanban's open card, a folder, a file. A step is one thing the person did: selecting a pane or a tab, following an in-app link, or moving inside a view that has joined. Back returns to the place before the last step with the pane selected, its view put back as it was and the caret where it was; Forward goes the other way.

Pass condition: the operator's own sequence, run in one window with real side-button presses: pane A (tab 1), a task link (the Kanban opens on that task), an agent clicked in the Kanban, pane B (tab 1). Back, Back, Back lands on the agent, then the Kanban with that task open, then pane A; Forward, Forward, Forward replays to pane B.

## 2. What the code does today

Read from the code (a first test of the build checks each of these by a run):

- The history is already one per window, in memory, not one per pane (`frontend/app/store/pane-history.ts`, `pane-history-core.ts`). An entry is a pair, tab and pane (`PaneRef`). A pure core keeps a list of entries and a cursor; the cap is 100; selecting the pane that is already current adds nothing; a selection after Back drops what was ahead; a step skips and removes entries whose pane is gone, and follows a pane moved to another tab of the window.
- Entries are recorded from one place, an effect over the active tab and the focused pane (`installPaneHistory`): whatever moves the selection is recorded, by a click, a key, a link or the host. A step suppresses it while it moves (`navigating`), and bumps an epoch so presses queued before a new selection are dropped.
- The mouse buttons and the two commands share one path (`side-buttons.ts`, `side-button-rules.ts`, `command-registry.ts`: `pane:history:back` and `pane:history:forward`). The buttons are ignored while a modal or a menu is open, with a modifier or the left button held, and a repeat within 150 ms is one press. Ctrl with the button over a browser pane is that page's own history, done in the host.
- A link that lands in another pane already moves the selection there: every in-app link goes through one of two funnels, the executor's person path (`frontend/app/deeplinks/open.ts`, `openForPerson` through the window's `openOrFocusPaneIdByView`, which focuses) or the entity openers (`frontend/app/links/entities/openers.ts`, thin adapters over the same openers). So, today, pane A, then a task link (the Kanban pane is selected), then an agent clicked in the Kanban (that agent's pane is selected), then pane B is already four entries, and Back already goes B, agent, Kanban, A.
- What is **not** in the trail: (a) where the view stands inside its pane. Back to the Kanban pane shows whatever card is open now, not task X (the Kanban holds its open card in its own state, and the link's request, `kanban:open`, is a one-shot ask that is cleared once taken); (b) a link or click that moves inside the pane that is already selected (a second task link while the Kanban is selected; another card; another board; another folder): the effect sees the same pane and records nothing; (c) a restore or a host move is recorded like a gesture, because the effect cannot tell the two apart.

So the operator's belief is right in one respect, the trail is bound to the selected pane, and wrong in another: a link that lands in a different pane is already a step. What is missing is the view's own place, and the moves inside one pane.

## 3. The model

**A place** is `{ tabId, blockId, view? }`. `view` is a small plain object the pane's view gives for where it stands (the Kanban: `{ board?, task? }`); it is absent for a view that has not joined, which is then pane-level, as today. It holds only what a link or a click names (ids, paths, a section), never content, and stays under about 1 KB.

**A step** is one entry of the list. The list and the cursor behave as today (browser-like): a step pushes after the cursor and drops what was ahead; the current place pushes nothing; the cap is 100 and drops the oldest. The pure core stays pure and gains the `view` part in its comparison and a `replace` operation.

**One gesture, one step.** A click that changes two things (opens the Kanban and puts a card in it) is one step, not two. A gesture is a short window (`runGesture`): the selection effect's own push is held while a gesture runs, and the gesture pushes one entry when it ends, after the next frame, carrying the pane and the view place together. Pushes in the same frame merge into the last one (rapid clicks coalesce by frame).

**How a view joins.** A view that wants its place in the trail registers a source for its pane while it is mounted, and unregisters on unmount:

```ts
registerPlaceSource(blockId, {
    read(): PlaceData | undefined;                  // where the view stands now; cheap, no layout read
    restore(place: PlaceData): boolean | Promise<boolean>;  // put it back; false when that place is gone
});
```

A view without a source is pane-level only. Restoring uses the view's own existing way in where it has one (the one-shot asks the deep links already write: `kanban:open`, `kanban:board`, `files:path`, `file` with `editor:line`, `audit:agent`, the `<view>:section` keys): no second way into a view is built.

**Recorders.** Three, and no others:

1. The selection effect (a pane or a tab selected), as today, for pane-level steps.
2. The two link funnels, for a person's open only (the executor's `source.kind === "person"`, the entity openers): the funnel runs the open inside a gesture, waits (up to 500 ms, in frames) for the view to take the request, and the gesture ends with the pane and its place as one step. An agent's open (`source.kind === "agent"`) takes no focus and records nothing.
3. A joined view's own click handlers, through `recordPlace(blockId)` (a card opened or closed by a click, a board picked, a folder entered). Never an effect that reacts to data.

## 4. Decisions on the edge cases

1. **How views join.** The optional source above. One gesture is one step.
2. **What never pushes.** A move made by an agent or by the host, a restore at start, a tab restoring, a live update, a pane re-rendering, a redirect, and Back or Forward themselves (the suppression counter now also covers `recordPlace` calls made while a step restores a view). A redirect (the view settles on its nearest place, `place: "missing"` in the executor's outcome) replaces the entry the gesture was making, never pushes a second one. The paths that move the person's selection on an agent's word (the MuxAPI pane focus and tab select) run inside `withoutHistory`; the build lists them by a search, one test each. Recommendation: do not record them, and give Back this rule: when the pane the person is on is not the entry at the cursor (an agent moved them), Back goes to the entry at the cursor first, without moving the cursor back one more; Forward goes to the entry after it. Without the rule Back would skip the place the person was last at on purpose.
3. **Push, truncate, dedupe, cap.** As today, plus the view part: a gesture whose pane and place equal the current entry pushes nothing. A push after Back drops what was ahead and cancels the presses queued before it (the epoch). Cap 100.
4. **A place that no longer exists.**
   - The pane or its tab is gone: Back skips the entry and removes it, never stops on an error (today's rule, kept). A pane moved to another tab of the window is followed (kept).
   - Only the view place is gone (the task was deleted, the file moved): the step still lands on the pane, with the dead place cleared from the entry. If that would show what the person already sees (the same pane, the same place), the step goes on to the next entry, so a press never looks like it did nothing.
   - Nothing earlier exists: the press does nothing. No hint in the first build (a notice on every press at the start of a trail is noise; revisit after use).
5. **Several windows and torn-off panes.** History stays per window. An entry whose pane is now in another window is dead for this window (skipped and removed), as it is today. Back never focuses another window: it would take the person's focus from where they are, and the other window has its own trail. A pane torn off into its own window starts that window's trail.
6. **Browser panes.** Back over a browser pane is unchanged: plain side buttons walk the window's trail, Ctrl and the button walk the page's own history in the host. A browser pane is a pane-level entry; the page's address is not part of a place (the page owns its history), and Back to a browser pane selects it without navigating it. A link followed inside a page is the page's history, not a step. An in-app link that opens an address in a browser pane is a step by the pane selection, as today.
7. **What a step restores.** The tab (waiting up to 1.5 s for it to settle, as today), the pane (and its dormant pane tab), the view's place, then the caret a frame later (as today). The view place is restored after the pane is selected and mounted: the step waits up to 500 ms for the view to register its source; when none registers, the step is pane-level. Scroll position stays the view's own (a view that remembers it keeps it; the trail does not store it).
8. **Persistence.** In memory, one per window, as today. Recommendation: do not keep it across a restart. A trail through places that changed while AgentMux was closed (deleted tasks, closed panes) mostly walks onto dead entries, restoring it races the layout loading, and a person starting the day does not expect Back to reach yesterday. Revisit if the operator wants it; the entry is plain data and would serialise.
9. **Overlays and text.** Menus and modals ignore the buttons, as today. New: a press during an IME composition is ignored (a focus move would cancel the composition); a text field with a selection is not hijacked because the side buttons do not edit text, and the press only moves the selection history. The two keyboard commands stay the same history, with their present key bindings.
10. **Cost.** A step must answer at once to the gesture: its handler does no layout read and no await before the first visible change in the same tab; the pure core is O(cap). A tab switch and a view mount are what take time, as today. The build adds one line per step: `[perf] history step back|forward <ms> tab=<switched|same> place=<restored|pane>`, so the cost of the step, its tab switch and its view restore are measured like the other interactions.
11. **Tests.** Unit tests of the pure core: push, truncate, dedupe by pane and place, replace, prune of a dead pane, a dead place cleared, the cursor-versus-current rule, the cap, replay. A route test for a joined view (the Kanban registers, a link gesture pushes one entry with the card, Back restores it, a source that is not registered stays pane-level). A test per recorder and per non-recorder (an agent's open, a restore, a step itself). One real run on a test host with real side-button presses and a recording of the operator's sequence (pane A, task link, agent in the Kanban, pane B, three Backs, three Forwards), then the same with the task deleted between, and with the agent's pane closed between.

## 5. Order

1. The pure core with the view part, `runGesture`, `withoutHistory` and the cursor rule, with its tests; no view joins yet, so nothing changes for the person (every entry is pane-level).
2. The Kanban joins (place `{ board?, task? }`: read from its open card and board, restore through `kanban:open` and `kanban:board`), the two link funnels and the Kanban's own card clicks record through gestures. This is the operator's sequence.
3. The agent pane: its place is the conversation it shows. If a pane can switch to another agent inside itself, that agent's id is the place and the pane's own switch records; otherwise an agent pane stays pane-level (the build finds out by reading the pane's code first).
4. The `[perf]` line and the real run with recordings. The run also checks, which the code reading above did not settle, that the selection of a floating pane is a step like any other.

Later views, one small change each (a source, plus the click handlers that move inside the view): Files (the folder and the selected entry), the Editor (file and position), the Audit view (agent, message, section), Settings (section and the row revealed), Swarm (section), Vault (entry), Memory (agent and scope), Connectors (section). The Browser stays the page's own history.

## 6. Questions for the operator (recommendation first)

1. An agent moving the person's selection is not a step, and Back then returns to where the person was (§4.2). Recommended; the alternative is to record it like any move, which keeps the trail simpler and makes Back undo what the agent did.
2. The trail is not kept across a restart (§4.8). Recommended.
3. No hint when there is nothing earlier (§4.4). Recommended.
4. When only the place inside a pane is gone (a deleted task), Back lands on the pane with the place cleared, and skips on if that shows nothing new (§4.4). Recommended.

## Sources

`frontend/app/store/pane-history.ts`, `pane-history-core.ts`, `side-buttons.ts`, `side-button-rules.ts`, `command-registry.ts` (`pane:history:back`, `pane:history:forward`); `frontend/app/deeplinks/open.ts`, `destination-meta.ts` (the places a link writes as pane meta), `open-deps.ts`; `frontend/app/links/entities/openers.ts`; `frontend/app/util/open-kanban-task.ts`; `frontend/app/view/kanban/kanban-section-state.ts` and `kanban-instance.tsx` (the open card and the one-shot `kanban:open`).
