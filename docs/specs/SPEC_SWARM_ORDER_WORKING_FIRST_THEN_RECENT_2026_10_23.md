# Spec — Swarm order: working agents on top, idle agents by most recent activity

**Date:** 2026-10-23
**Author:** Camper@narko
**Type:** Spec
**Status:** implemented — shipped in PR #5376; approved by the operator in conversation, 2026-10-23.
**Requested by:** the operator, 2026-10-23: *"lets also refine the swarm so working agents always leap atop. Also, the agents idle agents should be sorted by time from last active, with most recent higher"*

## 1. Today

The Swarm's agent list has no order of its own.

- `SwarmViewModel.buildTree()` (`frontend/app/view/swarm/swarm-model.ts`) walks `agent.tracked-blocks` in the order srv returns it, then adds any parent block that only a subagent named. It does not sort the agents.
- srv builds that list from the controller registry, a `HashMap` (`get_all_controllers()` → `ProcessBroker::list_agent_panes()`), so the order is whatever the map yields. It shifts when panes open or close, and it has no meaning.
- `swarm-view.tsx` renders it as one flat list keyed by block id (`rowKeys` → `AgentRowHost`), so a reorder moves existing rows rather than recreating them. The rows of other instances follow, in `remoteRows()` order (this machine's other channels, then LAN, then cloud, by machine, by name).
- There is no grouping by tab or host. Remote rows carry a "where" tag instead (`SPEC_SWARM_REFINEMENTS_2026_10_17.md` §5).
- The only sorting in the Swarm is inside a row (subagents, workflows, shells, crons, each newest first). The todo list keeps the agent's own order and stays that way.

## 2. The new order

Two bands, top to bottom. The order applies to the agent rows only; a row's own children are unchanged.

**Band 1, needs the Swarm's attention: working.** An agent is in band 1 when the status the Swarm chip shows for it is `needs-you`, `held` or `working` (`rankAgentStatus` in `frontend/app/store/agent-status.ts`). So:

- a turn in flight is working;
- waiting on a question or approval during a turn is in band 1. It is the agent that most needs a person, so it must not drop below idle ones;
- held (reconnecting, compacting, stopping, rate limited, launching) is in band 1. Its turn is not done and the state is temporary;
- an idle agent the Supervisor stopped for you (`needs-you`, paused) is in band 1.

The chip and the order read the same function, so a row cannot say "working" and sit among the idle ones.

**Band 2, everything else:** `idle`, `interrupted`, `errored`. A stopped or exited agent that is still listed is here too. Remote states map as the chip already maps them: `working` and `waiting` are band 1; `idle`, `stopped` and `error` are band 2.

**Within band 1:** the agent that started working most recently is first (it "leaps atop"). The key is the time the Swarm first saw the agent in band 1 during this stretch, `workingSince`, not its last output. Output arrives about once a second (§3), so ordering by it would shuffle working agents all the time. An agent first seen already working (the Swarm just opened) takes its last-active time as `workingSince`.

**Within band 2:** most recently active first. Agents that never produced output (last-active 0) come last.

**Tie-break, both bands:** name, case-insensitive, then block id. The sort is total, so equal keys never swap between renders.

**Remote agents.** An instance shares no last-activity time, so:

- remote agents in band 1 join the top band, after the local ones (by section order, then name);
- remote agents in band 2 stay after all local rows, in the existing order.

Sharing a last-active time over `swarm.other-instances` would let remote idle agents sort by recency. That is a separate change; it is not in this change.

## 3. "Last active"

`session:last_activity_ms` in the pane's block meta, read by `lastActiveMs()` (`swarm-last-active.ts`). It is the time of the agent's latest output line, flushed about once a second by srv (`session_stats.rs`), and is the same value the row already shows as "3m ago". It does not need a new source. It is 0 until the pane has stats.

An idle agent's value changes only when it produces output (a finished background task, say), so the band-2 order is calm. An agent that just finished its turn has the newest value, so it lands at the top of band 2, directly under the working agents.

## 4. Leaping without moving things under the cursor

**A re-sort waits while the user is using the list.** It holds while any of these is true:

- the pointer is over the list;
- a mouse button is down in it, or a drag is in progress;
- a context menu from a row is open;
- keyboard focus is inside a row (a card or a control in it).

When the last one ends, the re-sort runs after a 400 ms settle. The same 400 ms coalesces a burst of status changes into one move.

**While held:** the rows keep their current order. An agent that appears is added at the end and moves into place at the next re-sort. An agent that leaves is removed at once, since nothing shifts under the cursor but the rows below it, and a leaving row is already moving them.

**What does not hold a re-sort:** which pane has focus (the highlighted row) and which fleet checkboxes are ticked. That selection always follows the agent (§5), and holding on it would freeze the list whenever you work in an agent's pane, the usual case.

**Motion.** A move is a short FLIP slide (about 180 ms, transform only) for rows that change place. It goes through `shouldReduceMotion("transition")` in `frontend/app/motion/motion.ts`; when that returns true, the rows move without a slide. There is no `prefers-reduced-motion` query in the Swarm. Today the policy has both tiers run all motion, so the slide runs everywhere until the policy says otherwise.

## 5. Selection and keyboard focus follow the agent

- Rows are already keyed by block id, and the fleet selection (`pruneSelection`), the focused-pane highlight (`focusedBlockId`) and the expand state are all keyed by block id, never by index. They need no change.
- Moving a DOM node that has focus can drop focus in some engines. The hold in §4 means a re-sort never runs while focus is inside a row; the build must still test that focus survives a move that happens with focus elsewhere in the Swarm (the filter box, the toolbar).
- Tab order follows the new DOM order after a re-sort. Nothing else about keyboard handling changes.

## 6. Grouping

The Swarm is one flat list today, so §2 applies to it whole. When a grouping exists (`Group by lead` in `SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md` §Swarm, or a tab or host grouping later):

- the groups keep their own fixed order (for lead: the operator, then leaders, then workers, then "No seat"), never an activity order;
- §2 applies within each group, band 1 then band 2;
- the hold in §4 covers the whole list, not one group.

## 7. Where it goes

- A pure function `orderSwarmItems(items, workingSince)` in a new `frontend/app/view/swarm/swarm-order.ts`: no store reads, no timers. Local and remote rows are one list of items, told apart by `remote`.
- A small controller, `SwarmOrderController` in the same file, that keeps `workingSince`, runs the 400 ms settle and the hold, and publishes the order `swarm-view.tsx` renders. The view creates it through `useSwarmOrder` (`swarm-order-hold.ts`), which also wires the holds to the list element; the slide is in `swarm-order-motion.ts`. `buildTree()` stays unsorted, so the other callers (`collectClearableRows`, the selection prune) are unaffected.
- No srv change, no setting. The old order was arbitrary, so there is nothing to keep.

## 8. Tests

- **Pure order:** bands by status kind including `needs-you` and `held`; band 2 by last-active with 0 last; tie-break by name then block id; `workingSince` seeding for an agent first seen working; remote `working` and `waiting` join band 1 after the local ones; remote band 2 stays last.
- **Controller (fake timers):** a change waits 400 ms; a burst is one move; each hold condition delays it and release re-sorts; a joining agent is appended while held; a leaving agent goes at once; `workingSince` survives a re-sort and resets when the agent leaves band 1.
- **Render:** the highlighted row and ticked checkboxes stay on the same agent after a move; no move while the pointer is over the list or focus is in a row; focus elsewhere in the Swarm survives a move; with `shouldReduceMotion("transition")` true there is no slide.
- A live check in an isolated dev build: start two agents, run one, watch it leap; finish it and watch it settle at the top of the idle band; hover while another changes and confirm nothing moves.

## 9. Rollout

One frontend-only PR (#5376) with a `minor` changeset. No migration, no flag; it ships in the next release.

## 10. Decisions for the operator

1. **Is an agent waiting on a question, or held, "working" for the top band?** Recommended yes (§2). The alternative is a third band for `needs-you` above working.
2. **Order inside the working band.** Recommended: newest to start working first. The alternative is a fixed order by name, which never moves while agents work, but a new worker then does not "leap atop".
3. **Remote working agents join the top band.** Recommended yes. The alternative keeps all remote rows below the local ones until instances share a last-active time.
4. **The 400 ms settle and the hold on pointer and focus.** Recommended as written; say if you want a different wait or no hold.

Agent: Camper@narko
