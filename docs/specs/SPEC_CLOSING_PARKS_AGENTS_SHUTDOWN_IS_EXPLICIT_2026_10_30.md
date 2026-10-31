# Spec: closing a window, tab or agent pane parks the agent; shutdown is a separate, explicit act

Task #11085 (spec), #11086 (build). Written 2026-10-30 against `origin/main` 8564b2612. Owner: AgentX.
**Status:** draft — for the operator's decisions (section 5). Ships with the #11086 build; PR 1 (window close parks, reopen re-attaches) is built on the operator's own ask, the rest waits for their answer.

## 1. The ask

The operator (2026-10-29): "closing a window, tab, or agent pane will not shut it down but will go to pane-less. Shutdown will be a special operation, like /shutdown or an option in the agent pane under my agents."

## 2. What main already does (read from the code, not assumed)

`SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` built most of this. `agents:always_available` has been on by default since #5722 (`backend/always_available.rs`).

| Close | Today | Where |
|---|---|---|
| Agent pane | **Parks.** `park_agent_blocks` runs before teardown; the process keeps running; a note says "Closing a pane doesn't stop it" with an Open button | `sagas/close_pane.rs` `run_inner`; `view/agent/shutdown/keeps-running.ts` |
| Tab | **Parks** each agent block, then tears down the rest | `sagas/delete_tab.rs:149-170` |
| Delete workspace | **Parks** (same helper) | `sagas/delete_workspace.rs` |
| **Window** | **Stops every agent in it** (`Policy::close()` over all block ids), then keeps the layout as a "closed window" | `server/service/session_closed.rs` `keep_workspace`, ~line 236 |
| Last window / quit AgentMux | Every agent is stopped by `Policy::app_exit()`; work claims are kept and agents with desired state `running` resume at the next start | `main.rs:512`, `sagas/agent_teardown.rs` |

Surfaces for ending an agent today:
- `/quit` (alias `/exit`): ends this agent, releases claims and Shell sessions, closes its tab, keeps the conversation (`commands/global/quit.ts`, `sagas/self_quit.rs`). Named "quit", which reads as quitting AgentMux.
- `Stop` tool, action `agents`, and `Agents stop`: an agent or tool can end another agent (15 s veto toast).
- **My agents has no shut-down action** (`AgentCard.tsx`, `AgentPicker.tsx`). A pane-less agent can be opened but not ended from the list.

Cost guards already shipped (`server/agent_lifecycle.rs`): idle stop of an agent with no view after `agents:idle_stop_minutes` (60; 0 = never; per-agent override), and a cap of `agents:max_running` (24) with a Windows commit-limit check. An agent that is `waiting on you` or holds a claimed task is not idle-stopped.

**So the gap is narrower than the ask reads:** (a) closing a window still stops agents, and its confirm still says "This stops N agents"; (b) shutdown has no obvious name or place; (c) a pane-less agent in My agents cannot be shut down, and the list doesn't say it is running with no pane.

## 3. The rule

1. **Closing never ends an agent.** Pane, tab, workspace and window close detach (park). The process, transcript, claims and crons are untouched.
2. **Ending an agent is its own act**, with one meaning wherever it is offered: stop the process, set desired state `stopped` (so a restart doesn't bring it back), release its claims and Shell sessions, keep the conversation and the agent in My agents. It never deletes the agent.
3. **Quitting AgentMux is not a shutdown of agents.** The app's exit still winds processes down, but each agent's desired state is left alone, so the next start resumes the ones that were running (today's behaviour, now stated).

## 4. Changes (#11086)

**4.1 Window close parks.** In `keep_workspace`, run `park_agent_blocks` over the window's agent blocks before the teardown; tear down only what isn't parked (terminals, browsers, drawers, history readers). The closed window is still listed and its contents kept.
- On `TakeClosedWindow`, a parked block whose `agent:parkedFromTab` is a tab of that workspace is attached back into that tab, so the reopened window shows its agents where they were. Parked blocks leave `tab.blockids` and have no layout leaf, so the build must either keep the leaf in the closed window's snapshot or re-add one on reopen (I have not verified how the layout tree treats a removed leaf; this is the first thing the build checks).
- Container agents: parking skips teardown, so the container keeps running like a native agent.
- The closed-windows list shows how many agents kept running.

**4.2 Window close confirm.** With always-available on nothing stops, so `window:confirmcloseworking` no longer asks. Closing shows the same one-line note as a pane close ("N agents keep running without a window. Open them from My agents"). With `agents:always_available` off the old confirm stays and its text stays true.

**4.3 `/shutdown`.** New primary name for what `/quit` does; `/quit` and `/exit` stay as aliases so nothing breaks. Description: "Shut this agent down (stops it; the conversation is kept)". Same saga (`self_quit`), plus: desired state is set `stopped` before teardown.

**4.4 Shut down from My agents and the pane menu.** A "Shut down" action on every running agent row (pane-less or not) in My agents and in the agent pane's header menu. It calls the existing stop path with the `quit` policy. It confirms (naming what ends) only when the agent is mid-turn, holds a claimed task, or has crons targeting it; an idle agent shuts down with one click and an undo-free toast, since the conversation is kept. A stopped row offers Start/Open, as now.

**4.5 Pane-less is visible.** A running agent with no pane shows a "No pane" tag (the shared Tag primitive, #10928) in My agents and the Swarm row, with Open as the primary action and Shut down beside it. The status bar's running count (D5/D6 in the always-available spec) is the cost signal; I did not find where it is rendered and the build should confirm it counts pane-less agents.

**4.6 Shells, background tasks, crons.** Shut down uses `Policy::quit`: claims released, Shell sessions stopped; background tasks (dev servers) keep running unless the caller passes `stop_background`; crons are kept and reported in the toast, as `/quit` does today. Closing leaves all of these as they are.

**4.7 Docs and copy.** `keeps-running-text`, the window-close text and the agent-pane close prompts say "closing keeps it running; Shut down ends it" and no longer use "stop" for a close. Docs article on closing vs shutting down.

## 5. Decisions for the operator (recommendation first)

| # | Decision | Rec |
|---|---|---|
| D1 | Window close parks agents. | **Yes.** |
| D2 | Reopening a closed window re-attaches its parked agents to their old tabs. | **Yes.** The alternative (reopen without the agents) loses the layout. |
| D3 | Window-close confirm. | **Gone while always-available is on; kept when it is off.** |
| D4 | Name of the explicit act. | **`/shutdown`, with `/quit` and `/exit` as aliases.** |
| D5 | Where Shut down appears. | **My agents rows, the agent pane header menu, `/shutdown`, and the existing `Stop`/`Agents stop` for other agents.** |
| D6 | Confirm before Shut down. | **Only when the agent is mid-turn, holds a claim, or has crons; otherwise one click.** |
| D7 | Shut down sets desired state `stopped`, so a restart doesn't resume it. | **Yes.** |
| D8 | Quitting AgentMux leaves agents' desired state alone, so running agents resume at next launch. | **Yes (today's behaviour).** |
| D9 | Shut down keeps background tasks (dev servers) running by default. | **Yes, with a "stop these too" checkbox in the confirm.** |

## 6. Phases and tests

One PR (#11086), as the change is small once 4.1 is understood:
1. `keep_workspace` parks; `TakeClosedWindow` re-attaches. Tests: closing a window with a running agent leaves the process and transcript; reopening shows the agent in its tab; two blocks for one agent keep one; a terminal in the window is still torn down.
2. Confirm text and note; settings-off path unchanged.
3. `/shutdown`, the My agents and pane-menu action, the "No pane" tag, desired-state write. Tests: shutdown sets `stopped`; a restart does not resume it; a shut-down agent is still in My agents.

Owner: Manoz (agent-pane lifecycle) for 4.1; Agent1 or Camper-2 for the UI parts (Tag primitive); AgentX reviews the seam. Manoz is out of quota on Area54 until its host has a fresh account; Camper or Agent1 can take the whole PR meanwhile.

## 7. Risks and what I did not verify

- **Window reopen after parking** (4.1): whether the saved layout leaf survives parking is unverified; it decides whether re-attach is a one-liner or needs a layout write.
- **Cost surprise:** closing a window no longer frees memory; idle stop (60 min) and the cap (24) are the controls. A window with 15 agents closed on a laptop keeps 15 processes for up to an hour.
- **Quit semantic:** if the operator wants "quit AgentMux" to also stop agents for good (not resume), that is D8 the other way and a one-line change.
- Not run: nothing here was run live; all of it is read from `origin/main` 8564b2612.
