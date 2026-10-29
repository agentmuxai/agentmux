# The Swarm filter bar: narrow the agent list by name and by what an agent is working on

**Status:** active. Approved by the operator 2026-10-29 (task #10394): all seven questions in §4 as recommended; built in task #10395 by Camper-2@narko.
**Author:** Camper-2@narko, at the operator's request (2026-10-29); Camper reviews it, as the owner of the Swarm.
**Scope:** [`frontend/app/view/swarm/`](../../frontend/app/view/swarm/): [`swarm-agent-list.tsx`](../../frontend/app/view/swarm/swarm-agent-list.tsx) (the one list both sections share), [`swarm-root.tsx`](../../frontend/app/view/swarm/swarm-root.tsx), [`swarm-fleet-toolbar.tsx`](../../frontend/app/view/swarm/swarm-fleet-toolbar.tsx) (Select all), [`swarm-seats.tsx`](../../frontend/app/view/swarm/swarm-seats.tsx) (group by lead), [`swarm-stopped-agents.tsx`](../../frontend/app/view/swarm/swarm-stopped-agents.tsx), and the shared [`FilterInput`](../../frontend/app/element/ui/FilterInput.tsx).

The operator's words (10-29): *"in the swarm, we need a filter bar, filters for both agent names and work descs"*.

## 1. What exists

- **No filter in the Swarm.** The list shows every agent: this instance's agents with a pane (the tree), the agents of other instances (remote rows), and the created agents with no pane (the stopped list). Supervisor and Broadcast are two sections over **one** list (`SwarmAgentList`, rendered once); a section switch changes what sits above it and the rows' density.
- **What a row shows** (`AgentRow`): the agent's name (`agentName`, from the block's meta), the **line** (`node.line.text`: the generated title; if there is none, the previous session's title, a one-liner from the user's last message, or a status phrase such as "Waiting for you"; `store/swarm-line.ts`), the tool in flight, the in-progress todo, and under it the agent's running subagent, workflow, background-shell and cron rows. A remote row shows the name and its instance's line. A stopped agent shows its name only.
- **"Work description" is therefore the line.** The brief's other candidates are folded into it already (goal and last-message preview are what the line falls back to) or are not on a row at all: the row has **no branch or PR and no Kanban task title**, so there is nothing for a filter to match or highlight there. This spec does not add them.
- **The same box exists elsewhere.** `FilterInput` (magnifier, input, clear button, Escape clears) is the shared "type to narrow this list" box (My Agents' filter bar, the Help pane); `matchesEveryWord` (`util/fuzzysearch.ts`) is its matcher: every word, case-insensitive, no ranking, so the list keeps its order.
- **Three things the filter must not break:**
  1. **Selection.** Select all, the per-row checkboxes and Broadcast's recipients live in `model.selectedBlockIdsAtom`. `pruneSelection` drops an agent from it when the agent **leaves the list**; a filtered-out row must not count as having left.
  2. **Order.** The list's order is held while it is in use, so a row never moves under the cursor (`swarm-order-hold.ts`). Settling that order was the top long task in the Swarm on 10-29.
  3. **Grouping.** "Group by lead" builds headings and counts from the order it is given (`groupRowsByLead`).

## 2. The change

1. **One bar at the top of the list**, in both sections, below the Supervisor/Broadcast compartment and above the group-by control. One state for the pane, not one per section. It is shown whenever the list has agents **or the filter is non-empty**: with no agents and no filter the empty state stays as it is and there is no bar, but a restored filter on a list that is now empty keeps its bar (and "No agents match", item 9), so it cannot look broken.
2. **One text box matches a name or a work description.** Typing `name:` or `work:` first narrows that one (`name:lark`, `work:login`); without a prefix both are searched. Words are `matchesEveryWord`: every word must appear, in either field.
3. **What each row is matched on:**
   - **Name:** the name as the row shows it (`agentName`); for a remote or stopped agent, its name.
   - **Work:** the line's text; the in-progress todo; the tool in flight; the visible label of the agent's running subagent, workflow, background-shell and cron rows. A **match in a sub-row makes its agent match**, and the agent's row stays whole.
   - Only what the row's tree node **already holds** is read (`agentName`, `line.text`, `todoRows`, `currentTool`, the sub-row buckets); the filter reads no block meta and fetches nothing, so it adds no subscription of its own to the agents' data. The agent's slug, which the node does not carry, is not matched separately (the name is usually the slug).
   - **Not matched:** provider, status, tokens, the machine's name, the seat. (Status and machine have their own controls.)
4. **Live and cheap.** Case-insensitive substring per word. The box takes every keystroke at once; the list applies the filter **150 ms after the last one** (trailing debounce). Escape clears (the `FilterInput` behaviour); with the box already empty a second Escape gives focus back to the list.
5. **A count and the matches shown.** While a filter is on, the bar says **"7 of 24"** (rows listed now, local + remote + stopped), and the matched text in the name and the line is **highlighted** (a `<mark>`, the theme's highlight colour; text only, never a re-created row).
6. **Sub-rows stay with their agent.** A matching agent shows all its subagent/workflow/shell/cron rows, not only the matching ones.
7. **Group by lead keeps working on what is left.** The filter is applied to the list's order before it is grouped, so a heading's count is the shown rows and a group with none disappears.
8. **Selection and the fleet actions** (decision 3):
   - **Select all** selects `agents.selectableKeys` (this instance's agents, then the remote ones not gone stale; **not** the stopped, no-pane rows, as today) **intersected with the shown rows**, and says so while a filter is on: the button reads "Select all 7 shown".
   - A selected agent the filter hides **stays selected** (it has not left the list; no `pruneSelection`) but **no action reaches it while it is hidden**: Stop, the Supervisor mode change on the selection and Broadcast's send all act on the **selected and shown** agents only. The toolbar's count says so: **"5 selected · 2 hidden by the filter, not included"**, the Stop button's number and its confirm text count only the shown ones ("Stop 3 agents"), and Broadcast says who it will send to by the same count. Clearing the filter brings the hidden ones back into reach, still selected.
9. **No match:** the list says **"No agents match "<text>""** with a Clear button; the stopped list hides itself when none of its agents match.
10. **Kept per pane.** The text is stored in the pane's meta (`swarm:filter`, as Group by lead stores `swarm:groupby`), written 500 ms after the last keystroke, so it survives a restart and follows a pane moved to another tab. The bar and its count are always visible while it is non-empty, so a restored filter cannot look like agents that went missing.
11. **Keyboard:** the pane's **Find in pane** (Ctrl+F / ⌘F, `pane:find`) focuses the box when a Swarm pane has the focus. The Swarm has no other find, so no new shortcut is claimed.
12. **Narrow Swarm.** The bar is one slim row: magnifier, input, clear button. The count sits inside the right end of the box while there is room, and on its own line under it when the pane is narrower than the compact breakpoint (the width at which the rows already drop to their compact layout). The group-by control stays on its own row below.

## 3. How it stays cheap

- **No filter, no cost.** The match memo **returns at once when the query is empty and reads nothing**, so a Swarm with no filter has no new subscription to any row.
- **A filter re-runs on an agent's event but never moves a row under the cursor.** With a filter on, the memo reads the fields the node already holds, so it re-runs when an agent streams; a pass over the listed rows (tens to a few hundred) is microseconds. What would be churn is a row appearing or vanishing by itself, so the order hold's rule applies to **membership**: **a row that stops matching because of the agent's own update stays, shown as normal, until the filter text next changes or is cleared; a row that starts matching appears at once.** A change the person makes (typing, clearing) re-evaluates every row.
- The filter is a **pure function over the order the list already has** (`swarmOrder.order()` → the keys whose row matches), applied in `SwarmAgentList` just before `SwarmSeatList`. It never reaches into `useSwarmAgents`: `model.buildTree()` is not re-run, the held order is not re-settled, and a filter change is never delayed by the order hold (it is the person's own act, not an agent's change).
- Each row's search text is built from fields the row's node already holds; the match runs over the listed rows (tens to a few hundred) per debounced change. Rows the filter hides are not mounted; the rows it shows are built by the existing `AgentRowHost`, so what a keystroke costs is the rows entering or leaving the list.
- A bench in the build (a list of 60 agents, a filter toggled on and off) measures the cost of re-mounting rows against hiding them with a `display: none` class. The build keeps unmounting unless the toggle takes more than one frame, and then switches to hiding (the rows' elements, and a pressed row's identity, survive: #5208).

## 4. Questions for the operator (each with a recommendation)

1. **One box or two fields?** One. *Recommend one box with the optional `name:` / `work:` prefix:* it answers "filters for both" with a single control, costs about twenty lines (split the first word), and is invisible until used. Two fields would take twice the width of a narrow Swarm and make the common case (just find it) two steps.
2. **Keep the filter across a restart?** *Recommend yes,* per pane, because a Swarm left filtered on "codex" is usually left so on purpose, and the always-visible bar and count make a surprise impossible. The alternative (session only) loses it on every restart.
3. **A selected agent the filter hides: kept, dropped, or still acted on?** *Recommend kept, but not acted on while hidden, and counted* ("5 selected · 2 hidden by the filter, not included"). Dropping it silently deselects work the person did while narrowing the list to add more; acting on an agent the person cannot see is how a Stop or a broadcast reaches the wrong agents (Stop is the worse of the two). So every fleet action (Stop, mode, Broadcast) reaches the selected **and shown** agents only, and clearing the filter puts the hidden ones back in reach. The looser choices are to act on hidden selected agents with the count and the Stop confirm saying so ("Stop 7 agents, 2 of them hidden by the filter"), or to drop hidden agents from the selection whenever the filter changes: say so if you prefer one of them.
4. **Does a match in a subagent or background row show its agent?** *Recommend yes:* that is the work the agent is doing, and the agent is what the person acts on.
5. **Are remote and stopped agents filtered too?** *Recommend yes:* the list is one list; a filter that left other machines' agents standing would not be a filter. A stopped agent matches on its name only.
6. **Ctrl/⌘+F to focus it?** *Recommend yes,* through the existing Find in pane command.
7. **Match the branch / PR / Kanban task?** *Recommend not now:* they are not on a row, so a match would show an agent for a reason the screen cannot explain. A later change that puts the task on the row can add it to the matched fields.

## 5. Build outline (#10395)

1. `swarm-filter.ts` (pure): parse the query (prefix, words), `rowMatches(node, query)`, `remoteRowMatches`, `stoppedMatches`, and the highlight split (`splitMatches(text, words)`); tests for all.
2. `SwarmFilterBar` over `FilterInput`, in `SwarmAgentList` above the group-by control; the debounce and the `swarm:filter` meta (read and write through `ctx`).
3. Apply the filter to the order before `SwarmSeatList`; the stopped list and the empty-result note.
4. Highlight in the row's name and line (a shared `Highlight` element).
5. Selection: `selectAll(selectableKeys ∩ shownKeys)`; the fleet actions (Stop, mode, Broadcast) take the selected-and-shown keys; the "N shown" and "N hidden by the filter, not included" wording in `FleetToolbar`, the Stop confirm and Broadcast; `pruneSelection` unchanged (it gets the unfiltered keys).
6. The membership rule (§3): a shown row that stops matching on its own update stays until the filter changes.
7. `pane:find` focuses the box for a Swarm pane.

## 6. Verification

- **Unit:** the query parser (no prefix, `name:`, `work:`, several words, case, an empty or whitespace query matches everything); name and slug matching; the line, the in-progress todo, the tool and a sub-row's label; a sub-row match keeps its agent; remote and stopped rows; the highlight split; an empty query reads nothing; **a shown row whose own update stops it matching stays until the filter text changes, and a row that starts matching appears at once.**
- **Component:** the bar appears with agents and not without; typing shows the count and hides rows after the debounce; Escape clears, a second one leaves the box; "No agents match"; group headings and counts follow the filter; Select all selects only the shown, selectable rows (not the stopped ones) and the wording says so; a hidden selected agent stays selected, is counted as hidden, is **not pruned and not reached** by Stop, the mode change or Broadcast, and the Stop button's number and confirm text count the shown ones only; a restored filter on an empty list keeps its bar; the filter survives a remount through meta; the order hold does not delay a filter change.
- **Perf:** the bench in §3: the tree is not rebuilt, and 60 agents filter in under one frame (16 ms) on the dev machine.
- **Live** in my own task dev build (one at a time on narko): a Swarm with several seeded agents, before and after screenshots of the bar, a narrow pane, a filter with a highlighted match, "No agents match", Group by lead with a filter, Select all with a filter in Broadcast.

## 7. Not in scope

Sorting or saved filters; filtering by status or machine (their controls exist); the Kanban and its own filters; matching the branch, PR or task (question 7); a filter shared between several Swarm panes.
