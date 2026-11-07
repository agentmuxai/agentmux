# Spec: a new agent state beside Working and Idle: Background

**Status:** active — approved 2026-11-07 (operator, relayed by AgentY: "Approve as recommended", D1–D7 as recommended). The name is decided (operator, 2026-11-01: "ah, good lets do Background"). Step 1 of §9 (the shadow detector, no state change) is in the first build PR of #1072; steps 2–5 follow. Task #1071; its build is #1072.
**Date:** 2026-11-01
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `9470cd465` (2026-11-01), by reading code; re-read against `main` @ `730f098a9` (2026-11-07), see §2.1. Nothing here was run. Where I did not read a path to its end, it says so.
**Builds on:** `SPEC_AGENT_AVAILABILITY_EVERY_PATH_ONE_DECISION_2026_10_31.md` (#4565), `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6.2, `SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md`, `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`, `STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md`.

## 1. Ask

The operator (2026-11-01): "lets add another category to working/idle/<anytoher one> the name would represent an agent with work happening in the background like a long-lived". Relayed by AgentY.

An agent's turn has ended, so every surface reads it as **Idle**, but something it started is still running: a long-lived background shell command (a build, a dev server, a test suite), a background subagent, a monitor. It will wake on its own when that finishes, so "Idle" misleads: the operator thinks it is free, a leader thinks it can be reassigned, and an idle-stop or an upgrade can kill the work.

The new state is **Background**: no turn in flight, work in flight.

## 2. What main has now

The facts already exist; nothing joins them into a state.

- **Process tracker** (`backend/process_tracker/`): every agent's process tree is held in a Windows job object or a Linux cgroup (best-effort process-table scan on macOS and without a delegated cgroup). `registry.list_block(block_id)` is what the agent started, **excluding the CLI itself, conhost and MCP servers**. It is provider-independent.
- **`background_work.rs`**: `running()` turns `list_block` into process trees per agent for the upgrade panel, the launcher's dialog and the post-quit note to each agent (`SPEC_UPGRADE_FOR_USERS_AND_AGENTS` §6.2). So an upgrade restart already warns about background work; the Swarm and Agents list do not.
- **Claude's task feed** (`background_task_feed.rs`): `task_started` / `task_updated` / `task_notification` system lines with `task_type` (`local_bash`, `local_agent`, `local_workflow`), `is_backgrounded`, final status. Only backgrounded `local_bash` tasks are written, to `db_background_tasks` (durable, with the wrapper's pid via `pending_background_pids`). Subagents (`local_agent`) are the subagent watcher's: dispatches with a status.
- **Frontend `attachedTask` axis** (`agent-pane-state`, `useAttachedTaskAxis`, the Activity Dock): the pane already treats attached background work as busy for its working indicator. It is renderer-local: the Swarm of a window that has not mounted the pane never sees it (`swarm-display-status.ts` falls back to `running`/`idle`).
- **The one status** (`agent_state::decide` → `AgentState`: working, waiting, idle, stopped, error, limited, signed_out; frontend `rankAgentStatus` → needs-you, held, working, unavailable, interrupted, errored, idle). Idle is decided from "no turn in flight".

### 2.1 Re-read on 2026-11-07, and the neighbours AgentY asked about

`agent_state.rs`, `background_work.rs`, `background_task_feed.rs` and `process_tracker/` have no change between `9470cd465` and `730f098a9` (a diff of those paths is empty), so §2 still describes them. The neighbours:

- **Presence `state`** (`work_facts/live_phrase.rs`: `working`, `blocked`, `waiting`, `done`, kept in the block's `term:live_phrase_state`). It is what the **agent says about its own work**, set with the `Presence` `status` line, and it never feeds `agent_state::decide`. Background is what **srv sees running**. They stay separate: a Swarm row can read "Background" with the agent's own line "waiting: the release build" beside it. Background does not set or clear the agent's declared state, and an agent need not declare anything for the state to appear.
- **`Agents workers`** (`WorkerSummary.state` in `server/app_api/agent_spawn.rs`) is `agent_state::decide`'s value, so it reports `background` with no extra code, but the field's doc line lists the seven states and must gain the eighth. A leader waiting on a worker that reads Background learns it is not free, not that it is stuck.
- **The Activity Dock's rows** (the five-word label from #6191, over `background_task_feed` and the renderer's `attachedTask` axis) are detail per item; none of them says "the agent is idle but busy". They stay the detail view: Background is the one word on the row, the dock is what it points at.
- **Camper-2's 60 s rule (#6712)** is about the **person's** input in a browser pane: after 60 s without a click or key the agent may retake the pane (`server/browser_retake.rs`, the host's `last_input` answer). It says nothing about an agent being idle and does not use `agent_state`. No conflict and nothing to share; the spec records it so nobody reads "idle 60 s" as an agent state. One consequence: a Background agent that wants a pane the person left alone is still a retake case, by the same rule.
- **`agent_availability::availability`** is an exhaustive `match` on `AgentState` (`backend/agent_availability.rs`): adding `Background` fails to compile until it is given an arm, which is what §7's "no new variant: `Idle` with no quiet time" row does. The compiler enforces that every reader decides.

## 3. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| D1 | What counts | **Running work only**: a declared background shell task, a process tree an agent left behind, a background subagent, a monitor. |
| D2 | Pending wake-ups (`ScheduleWakeup`, `Schedule` cron, a `/loop`) | **Do not make the state Background.** Nothing is running; show them as a note on the row ("wakes 14:30"). They would make a long-idle agent read as busy for days. Revisit after v1. |
| D3 | Rank | Below **unavailable**: a limited or signed-out agent must still read as unable to take a message even with a dev server running. Above interrupted, errored, idle. |
| D4 | Messages to a Background agent | **Delivered, never held as cold.** A dev server can keep the state for 12 hours with the model session cold; holding a routine message that long is worse than a cache miss. It maps to Idle with no quiet time in `verdict`. |
| D5 | Idle stop | **A Background agent is not stopped to make room.** Stopping kills its process tree. |
| D6 | LAN, viewer and mobile feeds | Emit `idle` to peers and clients that do not advertise the state, plus a `background` count, until they do. |
| D7 | Ship order | **Measure first**: v1 runs the detector in shadow for a week (logs "would be Background"), because a daemon that outlives its command (sccache, a git fsmonitor, a build server) must not make every agent Background. Then switch the state on. |

## 4. The state

`AgentState::Background` (serde `background`), decided in `agent_state::decide` from one new input, `background: u32` (how many items run):

- A turn in flight is still `working` / `waiting` (it wins, as `limited` does not).
- No turn, not limited / signed out, `background > 0` → `Background`. With none → `Idle` as today.
- `since_ms` is when it became Background: the turn's end, or the first item starting after it.

Frontend: `AgentStatusKind` gains `background`, ranked as D3 says, with a fact `background?: boolean` that counts only while nothing is `live` (as `unavailable` does). Look: the existing state styles and the shared palette (#6151), a single steady accent between Working and Idle (not animated: nothing needs the operator), label **Background**.

## 5. Detection

The set is kept per block, srv-side, so every window sees the same answer.

| Source | What it sees | Reliability | v1 |
|---|---|---|---|
| **Declared background shell** (Claude `run_in_background`): `db_background_tasks` rows `running` | the feed's `task_started` with `is_backgrounded`; pid from bashwrap | High for Claude. Rows have stuck `running` before (#2518), so a row counts only while its pid is in the tracker (or, with no pid, for a bounded time); one that is not is marked stopped, which also fixes the stuck dock row | yes |
| **Process left behind** (any provider): `list_block` trees whose root was started under a bashwrap `exec` and outlived the call, past a grace period (30 s), minus a short image denylist (sccache, git fsmonitor, build daemons) | the tracker | High on Windows and Linux with a cgroup; medium on macOS | yes, behind D7's shadow run |
| **Background subagent**: feed `local_agent` with `is_backgrounded`, cross-checked by the subagent watcher's running dispatch | the feed, the watcher | High for Claude; a subagent the parent's turn outlived is the watcher's "abandoned" case, not Background | yes |
| **Monitor / workflow** (`local_workflow`, the Monitor tool) | the feed's `task_type` | **Not verified:** I did not read what task type Monitor reports. Treated as a background task if it reports `is_backgrounded` | verify in the build |
| **Wake-ups** (`ScheduleWakeup`, `CronCreate`, `Schedule`) | srv knows its own `Schedule` jobs; the CLI's native ones are in-session | D2: a note, not the state | note only |
| **Other providers** | tracker only (no structured feed) | as the tracker | the tracker source |

Where it is read: `fleet_source` observes state per block on its own cadence; the tracker read is cached per block for a few seconds, the feed writes are events. A change publishes the usual state change.

## 6. Where it shows

- **Swarm row**: the chip says Background; the line carries "cargo test, 12 min" for the oldest item (+N). The first signal the Swarm row has for it that does not depend on a mounted pane.
- **Pane header / working row**: Background replaces the pane's busy-but-no-turn look; the Activity Dock stays the detail.
- **My agents, Tower, Agents list, App health, status-bar counts**: the state value `background`; `Agents list` adds `background: [{ label, kind, started_at_ms }]` (at most 5). `App` health counts it beside working and idle.
- **Mobile and LAN peers**: per D6.

## 7. Interactions

| Reader | Today (Idle) | With Background |
|---|---|---|
| `verdict` (message hold) | `Idle` with quiet time can be cold-held | `Idle`, no quiet time: delivered (D4) |
| `agent_lifecycle::evictable` / idle stop | stops an Idle parked agent | not stopped (D5) |
| Kanban reminder loop | reminds an Idle agent | not reminded: it will wake itself |
| `worker_notes` busy check | unaffected | unaffected (a note may be sent) |
| Account move / failover | waits for the turn to end; restarts the CLI | would kill the tree: warn first, using `background_work` |
| Upgrade restart, quit | `background_work` warns already | one source: `background_work` reads the same per-block set, so the panel and the state cannot disagree |
| Availability (`Availability`) | `Idle` | no new variant: Background is `Idle { quiet_for_ms: None }` for the message decision; the `availability` match is exhaustive, so the build cannot miss it |
| `Agents workers` (`WorkerSummary.state`) | seven states | eighth value `background`; the field doc is updated |
| Presence `state` (`working`/`blocked`/`waiting`/`done`) | the agent's own words | unaffected; shown beside the state, never merged into it |
| #4565 guard test (`agent_state_readers_tests.rs`) | readers listed | new readers go through `status_of` / `decide` |

## 8. Shape

- `AgentState::Background`; `decide(inputs.background)`; `AgentStatus { state, since_ms }` unchanged.
- `backend/background_set.rs`: the per-block set (the sources above, one `Item { kind, label, started_at_ms, pid }`), the one answer for `decide`, `background_work`, `Agents list` and the Swarm.
- RPC: `AgentRuntime.state` gains the value; `Agents list` gains `background`. MCP `Agents` and `App` pass it through. Generated types regenerate (`gen-rpc-stubs`).
- Frontend: `agent-status.ts` (kind and rank), `swarm-display-status.ts`, the chip and its colours, `swarm-row-notes.tsx` (the detail).

## 9. Build, in order

1. `background_set` and the tracker / feed readers, **shadow only**: logs and a counter for would-be Background per agent. Tests with fake trees and feed lines.
2. Review one week of the shadow log; set the denylist and the grace period from it.
3. The state: `decide`, the interactions in §7, the guard test.
4. Surfaces: Swarm chip and detail, `Agents list`, App health, pane.
5. D6: feeds for peers and mobile.

## 10. Not verified / open

- That `list_block` excludes every MCP server and tool helper of the CLIs AgentMux runs (its doc says MCP servers and conhost): a stray one would make every agent Background. The shadow run measures it.
- What Monitor reports in the task feed (§5).
- Whether `agent_lifecycle`'s idle stop already spares an agent with tracked processes elsewhere; I read `evictable` (state == Idle) and nothing else.
- How an older mobile client or LAN peer renders an unknown state string (D6 avoids finding out).
