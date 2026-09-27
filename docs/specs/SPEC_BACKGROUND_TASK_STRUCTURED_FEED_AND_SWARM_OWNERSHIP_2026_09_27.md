# Spec: Background tasks from the CLI's structured task feed, owned in Swarm

> **Tracked family — canonical status: [`TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`](TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md) (issue #3338).**
> This document is accurate as of its own date. Parts may be superseded; check the tracking doc before acting on it.

**Date:** 2026-09-27
**Author:** AgentO
**Status:** active — Phase 1 (§2) implemented by #3953. Phases 2 and 3 (§3, §4) implemented by the PR that follows it (Swarm ownership). Phase 4 (§5) is not started; its bashwrap packaging prerequisite is #3952.
**Builds on:** `SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md` (registry list RPC + invalidation event), `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` (pid capture), `STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md` (one registry that the dock, `attachedTask` and Swarm all read)

## 1. Problem

Found root-causing an agent pane whose Activity Dock showed 11 "Bash" rows stuck `running` for 12+ hours (installed build 0.57.4, macOS). All 11 were `run_in_background` commands that **subagents** had launched and that had finished within five minutes.

How the registry (`db_background_tasks`) and the dock learn about a background task today:

- **Start:** the renderer recognises an accepted launch by the tool result text `Command running in background with ID:` (`tool-adapter.ts` `isAcceptedBackgroundLaunch`) and pushes `docknodestatus`, which creates the row.
- **End:** the renderer finds a `<task-notification>` **user message** in the same pane (`parseTaskNotification`) and pushes the completion.

Both are text matches, and on current Claude CLIs both are wrong in opposite directions:

| Launched by | Result shape (`tool_use_result`) | Recognised as background? | `<task-notification>` user message in the pane? | Outcome |
|---|---|---|---|---|
| The agent itself | `{stdout: "", …, backgroundTaskId: "b…"}`; the text is only in `message.content` | **No** (the renderer reads the empty `stdout`) | Yes | Never shown as running |
| A subagent | `null`; the text is in `message.content` | Yes | **No** (it goes to the subagent) | Running forever |

Meanwhile the CLI already emits a **structured task feed** on the parent agent's stdout, subagent-owned tasks included, and nothing in AgentMux reads it:

```json
{"type":"system","subtype":"task_started","task_id":"b6p74mfn6","tool_use_id":"toolu_…","description":"Background wait for agents","task_type":"local_bash","is_backgrounded":true,"owned_by_subagent":true}
{"type":"system","subtype":"task_updated","task_id":"b6p74mfn6","patch":{"status":"completed","end_time":1790434791}}
{"type":"system","subtype":"task_notification","task_id":"b6p74mfn6","tool_use_id":"toolu_…","status":"completed","summary":"Background command \"Background wait for agents\" completed (exit code 0)"}
```

Measured in one agent's record (all sessions): 2,553 `task_started`, 1,335 `task_updated`, 2,555 `task_notification`, 13,769 `task_progress`. `task_type` is `local_bash`, `local_agent` or `local_workflow`. `task_updated` patches are `{status, end_time}` (`completed`, `failed`, `killed`) or `{is_backgrounded: true}` when a running command is moved to the background. All 11 stuck tasks have their start and their end in the parent stream: 10 end with `task_notification`, the 11th (a watch loop) with `task_updated {status: completed}`. Replaying that session's 3,046 recorded task lines through Phase 1's `TaskFeed` yields 33 background-task rows, all terminal (31 done, 1 error, 1 stopped), the 11 among them.

A subagent's own stream lines carry `parent_tool_use_id` = the `tool_use_id` of the Agent call that spawned it, which is also the `toolUseId` in that subagent's `agent-<id>.meta.json`. That is the join key from a background task to the subagent that owns it, at any spawn depth.

## 2. Phase 1 — the registry follows the CLI's task feed (implemented)

### 2.1 srv reads the feed

`agentmux-srv/src/backend/background_task_feed.rs`, called for every parsed stdout line in the persistent controller's reader (`blockcontroller/persistent/spawn.rs`):

- `task_started` with `task_type: local_bash`: remembered per stream by `task_id`. If `is_backgrounded`, `background_task_observe(tool_use_id, block, description)`, so the row's label is the CLI's description rather than "Bash".
- `task_updated {is_backgrounded: true}`: observes the remembered task, with its original start time.
- `task_updated {status}` / `task_notification`: completes the row (`completed` → `done`, `failed` → `error`, anything else → `stopped`), only while it is still `running`, so the first end time stands and subscribers hear it once. `task_updated` names only the `task_id`; the remembered start resolves it. `task_notification` carries its own `tool_use_id`, which also closes a row that started before this reader (an srv restart).
- Every change publishes `background-task-updated`, now shared from this module by `websocket.rs`.

The renderer's existing `docknodestatus`/completion pushes are unchanged; every write is the same idempotent store call, so whichever arrives first wins.

### 2.2 The renderer recognises structured launches

`isAcceptedBackgroundLaunch` accepts a non-empty `result.backgroundTaskId` as well as the text prefix.

### 2.3 The dock takes a finished status from the registry

`applyRegistryOutcomes` (`activity/background-adapter.ts`): a transcript-derived dock row still showing `running` takes the registry row's terminal status and end time. That closes subagent-owned rows, whose text notification never reaches the pane.

### 2.4 Not covered by Phase 1

- Non-persistent controllers (one-shot subprocess, containers) keep today's renderer-only path.
- Tasks the CLI never reports ending (a CLI that dies mid-task, `kill -9`). See Phase 4.

## 3. Phase 2 — ownership (implemented)

- Store the owner on the row: new nullable `db_background_tasks.owner_tool_use_id`, from the `parent_tool_use_id` of the `assistant` line that issued the Bash call. srv sees that line immediately before `task_started`; keep a small per-stream `tool_use_id → parent_tool_use_id` map in the same reader.
- `ActiveSubagent` gains `tool_use_id` (from `meta.json`'s `toolUseId`, which `subagent_watcher::completion::tool_use_id_for` already reads).
- `COMMAND_LIST_BACKGROUND_TASKS` accepts an empty `blockid` meaning "every block" (backed by a variant of `background_task_list_running` that also returns recently ended rows), so Swarm can list the fleet in one call, the way it lists subagents and shells.

## 4. Phase 3 — Swarm shows background work under its owner (implemented)

- Swarm's model fetches the fleet list and subscribes to `background-task-updated` without a scope, like its other buckets.
- Per agent row: a background task whose `owner_tool_use_id` matches a subagent's `tool_use_id` renders nested under that subagent; the rest render at the agent level. Nesting follows the subagent tree at every depth.
- Each row shows the description, elapsed time and real status (`running`, then `done`/`error`/`stopped` for the same retention window the dock uses). A finished subagent keeps its still-running tasks visible under it; they are not closed just because their owner ended.
- `swarm-longrunning.ts` keeps showing long-running **foreground** calls; background calls come from the registry, deduplicated by id (registry id = tool_use_id).
- Consistent with `SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md` §3.3 (Swarm reads the same feed, no duplicated "Running in background" text) and `TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md` §3.4 (the four "what's running" subsystems stay separate; this only feeds one of them into Swarm).

### 4.1 As built

- The fleet list is `COMMAND_LIST_BACKGROUND_TASKS` with an empty `blockid`: running rows plus rows that ended in the last 60 s (`FLEET_ENDED_WINDOW_MS`, longer than the longest finished-row retention).
- `SubAgent.tool_use_id` is read from `meta.json` when the subagent is first seen and retried on later transcript changes, because the CLI can write the sidecar after the transcript.
- A task is nested only under a **solo** subagent row that is actually shown. A task owned by a workflow member, or by a retired subagent row, renders in the agent's Background bucket instead, so nothing is hidden. Nesting under workflow members is left for later.
- Nested rows show whether or not the subagent row is expanded.
- `background_task_describe` also replaces the renderer's generic "Bash" label with the CLI's description when the renderer created the row first.

## 5. Phase 4 — backstops for tasks the CLI never reports ending

- srv consumes `agentmux-bashwrap`'s own `tool_chunk` events with a broker observer (`Broker::add_observer`, the pattern `notify/router.rs` uses): `pid` → the existing pending-pid path, `terminal` → complete the row with the real exit code. This removes the renderer round-trip that today only works while the pane is mounted.
- A periodic liveness sweep over `running` rows with a pid (`reactive::registry::pid_alive`), guarded against pid reuse by comparing the process start time with `started_at_ms`. A dead pid with no reported end becomes `stopped`. A real long-running `task dev` stays visible because its pid is alive.
- Prerequisite on macOS and Linux: packaged builds must ship `agentmux-bashwrap` (fixed separately, with `scripts/check-bundled-tools.sh`).

## 6. Tests

- Phase 1: `background_task_feed.rs` unit tests over real event shapes (parse, subagent-owned start then completion, foreground calls never get a row, moved-to-background then ended by `task_id`, completion of a row started before the reader). `tool-adapter.test.ts` (structured `backgroundTaskId`) and `background-adapter.test.ts` (`applyRegistryOutcomes`).
- Later phases: owner attribution from a recorded stream fixture; Swarm grouping; sweep with a dead pid and with a reused pid.

## 7. Open questions

- Whether `task_progress` (a subagent's current step, tokens, tool count) should also feed Swarm's subagent rows. Out of scope here.
- `local_workflow` tasks: not registry material today; revisit with the Workflow rows in Swarm.
