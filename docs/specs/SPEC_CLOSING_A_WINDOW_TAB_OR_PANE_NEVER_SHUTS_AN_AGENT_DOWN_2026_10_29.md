# Spec: closing a window, a tab or a pane never shuts an agent down; shutdown is its own act

**Status:** draft — for the operator's decision on §9. Kanban #11085 (spec); the build is #11086 (after the spec), both on narko's board.
**Date:** 2026-10-29
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `e42ecf34b` (2026-10-29), by reading code. Nothing here is built. What I did not trace is listed in §8.
**Builds on:** `SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` (an agent runs and is recorded with or without a pane; rule 3.3 already says "closing a pane, a tab, or a window detaches"), `SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md` (the Stop tool and its 15 s toast), `SPEC_AGENT_SELF_QUIT_2026_09_24.md` (`/quit`), `SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md` (the teardown), `SPEC_WORKSPACES_2026_10_23.md` (closed windows).

## 1. The ask

The operator, 2026-10-29: "closing a window, tab, or agent pane will not shut it down but will go to pane-less. Shutdown will be a special operation, like /shutdown or an option in the agent pane under my agents."

## 2. The finding in short

**Most of this is already built.** With `agents:always_available` on (the default since #5722), closing a **pane**, a **tab** or deleting a **workspace** parks each agent's block: its process keeps running, its transcript and session stay, and a one-line toast says so, with an Open button (`sagas/park_agents.rs`, `frontend/app/view/agent/shutdown/keeps-running.ts`: "Stop is the only way to end an agent"). What is left:

1. **Closing a window still stops its agents.** `keep_workspace` runs `Policy::close()` over every block of the window (`server/service/session_closed.rs:236-245`), and the close warning says "This stops N agents" (`frontend/app/window/window-close.ts`). This is the one place the operator's sentence is not true today.
2. **Shutdown has no user-facing name.** An agent can end itself (`/quit`, `/exit`), an agent can end another (the `Stop` tool, with its 15 s toast), and the Swarm has a bulk stop; there is no `/shutdown`, and My Agents has no "Shut down".
3. **Quit and the last window** end every process (the app is exiting), and only agents with no pane are resumed by srv at the next start; an agent kept in a closed window would be neither.
4. **The words and the settings** still describe the old behaviour: "This stops N agents", `window:confirmcloseworking`, "Closing X in 15 s".
5. **The guard against forgetting** exists in part (idle stop after 60 min, a cap of 24, a running count in the status bar) but does not apply to an agent kept in a closed window, and nothing tells the person at the moment they close.

## 3. What happens today

| Event | Agents | Terminals and other panes | Where it is |
|---|---|---|---|
| Close a **pane** (X) | parked: process, session, transcript kept; the pane goes | closed | `park_agents.rs:113`, `close_pane.rs:157` |
| Close a **tab** | parked, same | closed | `delete_tab.rs:149` |
| Delete a **workspace** | parked, same | closed | `delete_workspace.rs:128` |
| Close a **window** | **stopped** (`Policy::close`), blocks and layout kept as a closed window; reopening resumes them | its PTYs stop; layout kept | `session_closed.rs:217-249`, `window-close.ts` |
| Remove a window from its workspace | stopped, then the window's tabs and panes are deleted ("The agents stay in My Agents") | closed | `window-close.ts` (`removeWarningText`); the srv path was not traced |
| **Quit** (tray Quit, last window with the tray on) | every agent closed gracefully (`Policy::app_exit`), shells and containers stopped, claims kept; the **parked** ones that were running are recorded and resumed 90 s after the next start, 3 s apart; panes in windows come back with their windows | PTYs stop | `agent_teardown.rs:109-114, 266-302`, `restart_resume.rs` |
| **Last window** closed | with the tray on (the default) and `run_in_background` off (the default), closing the last window quits | | `crates/launcher/src/background_config.rs` (defaults) |
| Agent's process exits | block kept, shown stopped or error | | always-available rule 6 |
| `/quit`, `/exit` (the agent, in its own pane) | ends gracefully (waits up to 30 s for the turn), releases its claims, removes its own tab | | `commands/global/quit.ts`, `sagas/self_quit.rs` |
| `Stop` tool, `agents` (an agent ends another) | a 15 s window with a toast and Keep running; then `self_quit` with origin `ExternalStop`: ends and removes the tab. `Stop` `pane` refuses an agent pane | | `pending_shutdown.rs`, the Stop spec §3.1-3.2 |
| Swarm bulk stop (the person) | stops now, no window | | `fleet.bulk-stop` |

What keeps an unattended agent bounded, from `server/agent_lifecycle.rs`: an agent with **no view** (its block parked) that has been idle `agents:idle_stop_minutes` (default 60, 0 = never; `agent:idle_stop_minutes` per agent) is stopped, unless it waits on the user or holds a claimed task; `agents:max_running` (default 24) caps live processes, and a start is also refused over `agents:commit_limit_pct` (85) of memory. The status bar shows the running and waiting counts (`runtimeCounts`).

## 4. The rule

**Closing a window, a tab or a pane detaches. It never ends an agent. Shut down is the only way to end one** (and the app quitting, which ends every process and brings back the agents that were running).

The closing of a *terminal* (a shell, a browser, an editor) is not changed: a shell without a view has nothing to give, so it still ends, with the process-naming confirmation that exists (`SPEC_PANE_CLOSE_CONFIRM_NAMES_PROCESSES_2026_09_23.md`).

## 5. What closing does, per event and per state of the agent

"Keeps running" below is the parked agent of the always-available spec. The state is the one the Swarm already shows.

| The agent is | Close a pane / tab | Close a window (new) | Notes |
|---|---|---|---|
| **idle** | parked; process idle; costs memory (about 0.5 GB), no tokens | same; the agent stays where the window left it (§6) | idle stop after 60 min applies |
| **working** (a turn in flight) | the turn continues; its result is recorded in the transcript; no prompt | same | the old "Close this window? N are working" dialog goes (§7) |
| **waiting on the user** (a question, an approval) | it keeps waiting and shows `waiting on you` in the Swarm, the Kanban "Needs you" and the status bar, with a notification; closing answers nothing | same | no auto-answer, no timeout (always-available D8) |
| **limited / signed out** | parked; it shows that state | same | unchanged |
| **stopped** (no process) | nothing to keep: the pane goes; its block is parked as stopped | same | |

Background tasks the agent started (`run_in_background`, dev servers) and its `Shell()` sessions are the agent's: they keep running with it and end with its shutdown (§7). A container agent's container stays up while the agent is kept and stops with the agent's shutdown or its idle stop (§8: confirm that the idle stop stops the container).

## 6. Closing a window: where the agents go

Today the window's contents are kept as a closed window and the agents are stopped. Two ways to keep them running:

- **A. Keep each agent's block in its tab of the closed window**, with its controller alive: only the window is gone, the tab and layout record stay, reopening the window shows the agents live. The block is in a tab, not parked.
- **B. Park the blocks** (as a tab close does) and remember where each was (`agent:parkedFromTab`, which exists since #5768), then re-attach on reopen. A park removes the pane from the tab's layout, so a reopened window loses the arrangement of its agent panes.

**Recommendation: A.** The point of a closed window is to come back exactly as it was (`SPEC_WORKSPACES_2026_10_23.md`), and A keeps the layout untouched. What A needs, found by reading:

1. `keep_workspace` runs the close teardown over the window's blocks **except the agent blocks** (`park_agents::is_parkable`, `extras_to_close` for the old two-blocks-per-agent layouts), and lifts nothing it did not take. Terminals and the rest stop as today.
2. **"No view" must include a block in a closed window.** The idle sweep and the cap treat an agent as viewless when its block is parked; a block in a closed window's tab must count as viewless too, or it never idles out (§3, `agent_lifecycle.rs`).
3. **Open must work on such a block.** `agent.open` focuses the pane that shows the agent. For a block in a closed window, Open offers **Reopen the window** (the usual path) or **Move here** (the block goes to the active tab; the closed window loses that pane, as with the existing Move here).
4. **Restart.** A closed window is not reopened by anything at startup, so its agents would be neither window-resumed nor parked-resumed. `record_running_at_exit` must record the agents kept in closed windows with the parked ones, and `restart_resume` resume them the same way (staggered, idle-windowed).
5. **Readers that walk tabs** (`agent.list`, the Swarm's tracked blocks, `resolve_tab_id_for_block`) already see the tabs of a closed workspace; `resolve_tab_id_for_block` for a spawning agent returns that closed window's tab, so its workers would open in a tab nobody sees. Use the agent's team tab (`team_tab`, #5769) or the active tab, as for a parked leader.

**Remove window** (take the window out of its workspace and delete its tabs) is a delete of the window's contents: it follows the tab-close rule (the agents are parked, not stopped). Its confirmation text changes the same way.

## 7. Shutdown: the one act that ends an agent

**Names.** `/shutdown` in the agent's pane; **Shut down** in the agent's menu in My Agents and in the Swarm row; the `Stop` tool, `agents`, for an agent that ends another. `/quit` and `/exit` stay as aliases of `/shutdown` (they exist, and Claude Code users type `/exit` by habit).

**What it does** is what `/quit` and `ExternalStop` do now (`Policy::quit`): interrupt the turn and let the process exit (force-kill at the deadline; `/quit` waits up to 30 s for a turn to end), release the agent's work claims, stop its `Shell()` sessions and its container, remove its pane and set its desired state to `stopped`, so a restart does not resume it. The agent, its conversation and its session stay: Open resumes the session. Background tasks end with it (the Stop spec's D5); `agent:stopkeepsbackground` keeps meaning "the internal stop that keeps the pane" and is not touched.

**Who asks, and how much friction.**

| Who | Confirmation |
|---|---|
| The person, from My Agents or a Swarm row | none for an idle or stopped agent; one confirm naming the claimed tasks and the working turn for a working one. Immediate: it is their own act, so no 15 s window (same as the Swarm's bulk stop today) |
| The agent itself (`/shutdown`, `/quit`) | as `/quit` today |
| An agent ending another (`Stop` `agents`) | the 15 s window with the toast and Keep running, as built (#5375, #5397). Only the strings change: "Shutting down X in 15 s", "Keep running" |

`Stop` `pane` keeps refusing an agent pane and says to use `agents`. Closing a pane is not a shutdown, so the Stop spec's line "`Stop` `agents` closes the agent's tab: the same as the user closing the tab" is no longer true as worded; the behaviour (it ends the agent) stays and the words say "shuts down".

**The old close dialogs.** `window:confirmcloseworking` asked "Close this window? N agents are working. This stops them". That warning is false now. Recommendation: the window close shows the same one-line toast as a pane close ("3 agents keep running. Open"), and the setting is retired the way `agents:always_available` will be (kept one release, no effect on agents, then removed). `tab:confirmclose` (default off) stays for terminals.

## 8. Where a kept agent shows, and getting a pane back

- **The Swarm** lists every created agent with its state (the always-available spec's P3); a kept agent has a row; a kept-in-closed-window agent's row says "in the closed window <name>".
- **My Agents** shows the state chip and **Open**; the status bar shows "N running, M waiting on you".
- **Open** attaches the agent: for a parked agent (`AttachBlock`), for an agent in a closed window as in §6.3. A pane that opens on a working agent replays the transcript and tails it; the mid-turn open is the always-available spec's unverified item (§5.3 there), and this spec does not move that.
- **Waiting on you** is the case that must not be silent: it has the notification, the Swarm state and the Kanban column.

## 9. Quit, the last window, and the cost of what is left running

**Quit** must end every process (srv exits). It records what was running and the next start brings it back: panes in windows with their windows, parked agents and (new, §6.4) agents kept in closed windows through `restart_resume`. "Not shut down" holds across a quit in the sense that the agent is back without a person doing anything, not that it had no gap.

**Last window.** Today, with the tray on and `run_in_background` off (both the defaults), closing the last window quits AgentMux, which ends every kept agent until the next launch. That contradicts "closing will not shut it down". Options: (a) leave it: closing the last window quits, the agents resume at the next launch; (b) when any agent is running or waiting on the user, closing the last window **keeps AgentMux alive in the tray** (the existing background mode) with a one-time explanation, and **Quit** from the tray is the explicit end, naming the running agents. **Recommendation: (b).**

**Cost.** An idle kept agent costs memory (about 0.5 GB, Loap-3's measure on narko), not tokens; a **working** agent costs tokens whether or not a pane shows it, which is the real new exposure: a turn the person closed the window on keeps going, as does a Job or a message that wakes the agent. What bounds it, and what is added:

| Guard | Today | Change |
|---|---|---|
| Idle stop (60 min, per agent override) | for a parked block | also for a block kept in a closed window (§6.2) |
| Cap (`agents:max_running`, 24) and the memory headroom | yes | none |
| Running count in the status bar | yes | none |
| Said at the moment of closing | a one-line toast for a pane | the same line for a tab and a window, with the count: "3 agents keep running (2 idle, 1 working). Idle ones stop after 60 min." |
| Said when the person goes away | nothing | the tray's Quit lists the running agents; with (b) above, the first time the last window closes with agents running |
| A forgotten working agent | nothing | none now; if the operator wants one, a "working for more than N hours with no pane" notification is a small addition (D7) |

## 10. Build (for #11086), if approved

One PR per row, in this order, each under the setting `agents:always_available` as today.

1. **Window close keeps agents** (§6): `keep_workspace` skips agent blocks; the closed-window record notes them; "no view" includes them; Open on such an agent; restart record. Tests: closing a window with an idle, a working and a waiting agent leaves all three running and their blocks in the closed window's tabs; reopening shows them live; the idle sweep stops an idle one after the window; a restart resumes them; a terminal in the window still stops.
2. **Remove window parks** and the dialogs: the new toast for a window and a tab close with the count; the old warning and `window:confirmcloseworking` retired.
3. **Shutdown named**: `/shutdown` with `/quit` and `/exit` as aliases; **Shut down** in My Agents and the Swarm row; the Stop strings. Tests: the three entry points end the agent, set desired `stopped`, release claims, and a restart does not resume it; the Stop 15 s window is unchanged.
4. **Quit and the last window** (D5 decides): the tray stays with agents running; Quit names them.

## 11. Risks and what I did not verify

1. **A working agent no one watches** keeps spending (§9). The window-close dialog was the only brake and it goes. The status bar count and the toast are the replacement; the operator may want more (D7).
2. **Option A leaves agents in tabs of a window nobody sees.** Anything that assumes "a block in a tab is shown" can misjudge them (the orphan reaper, `agent.open`'s focus-the-pane path, the spawn tab lookup). Found by reading, not by running; the build's first PR carries the tests.
3. **Not traced:** the srv path of **Remove window** (§3); whether the idle stop also stops a container agent's container (`agent_lifecycle.rs` was read for the sweep's rules, not for what it tears down); how the launcher decides "last window" on macOS and Linux (only the defaults were read); the mid-turn Open seam (the always-available spec's open item).
4. **Quit leaves a gap**: a kept agent is down between the quit and the resume (90 s after launch plus 3 s per agent). That is stated, not removed.
5. **Old layouts** with two blocks for one agent: `extras_to_close` already handles them for pane and tab close; window close must use it too.

## 12. Decisions for the operator

1. **Where do a closed window's agents go?** (A) stay in the closed window's tabs, running, and come back with the window; (B) are parked and lose their place in the layout. **Recommend A** (§6).
2. **Is `/shutdown` the name, with `/quit` and `/exit` kept as aliases?** And **Shut down** under the agent's menu in My Agents and in the Swarm row. **Recommend yes.**
3. **Friction on the person's own shutdown.** None for an idle agent; one confirm for a working one or one holding claimed tasks; no 15 s window (that stays for an agent ending another). **Recommend as written.**
4. **The close dialogs.** Replace "Close this window? N agents are working" with the one-line "keeps running" toast and retire `window:confirmcloseworking` for agents. **Recommend yes.** The alternative is to keep a warning for a working agent.
5. **The last window.** (a) it quits and the agents resume at the next launch, or (b) AgentMux stays in the tray while agents run, and Quit is the explicit end. **Recommend (b).**
6. **Terminals.** Closing a terminal pane still ends its shell (with today's confirmation). The alternative is a pane-less shell, which I do not recommend: nothing could show it. **Recommend as is.**
7. **A guard beyond the status bar and the toast.** None now; add a notification for an agent that has worked for N hours with no pane only if you want one. **Recommend none for now**, and look at the running count after a week of use.
8. **Idle stop for kept-in-window agents.** The same 60 min as a parked one. **Recommend yes.**
