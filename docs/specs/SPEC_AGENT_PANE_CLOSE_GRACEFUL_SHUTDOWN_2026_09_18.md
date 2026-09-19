# SPEC: Closing a pane shuts down every agent in it — gracefully, in order

**Date:** 2026-09-18
**Status:** active — Phase 0 measured (§5.1), Phase 1 implemented in #3402 (§5.2);
Phase 2 implemented for the Claude persistent controller (§9.9); Phase 3 not
started.
**Author:** AgentA@Area54
**Related:** `docs/specs/SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md`
(the continuity guarantee this spec protects),
`docs/specs/persistent-process-mode.md` (the persistent controller this spec
changes), `docs/specs/SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md` (already
names the backend half of the stack-orphan bug and tracks it as issue #3202),
`docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` ("tracked ⇒
dies with the pane" — preserved here),
`docs/specs/TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md` (the
in-turn "backgrounding" family — a different concern, see §6),
`docs/specs/SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md` (app-level
shutdown — out of scope here, see §6),
`docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md` (why every agent tab in
a pane is now mounted at once — relevant to §4.7).
**Tracks:** the frontend half of the bug has no issue yet; the backend half is
#3202.

---

## 0. The ask

> when agent panes are closed, agents are to gracefully shut down, saving their
> state so that when they are opened elsewhere they can be picked up [...] lets
> audit that entire flow

> is that graceful shutdown ordered well, that is, if multiple agent tabs are
> open in a pane and the pane is closed

The audit (§2, §3) found that neither half holds today: closing a pane that
holds several agent tabs shuts down only the visible one, and the one it does
shut down is hard-killed after its records are already deleted.

## 1. The guarantee this spec establishes

**Closing a pane stops every agent in it. Each one is stopped gracefully, its
resumable state is saved, and only then are its records removed — so reopening
that agent anywhere resumes the same conversation.**

Corollaries:

- Closing one tab (its own ×) and closing the whole pane (the pane's ×) end in
  the same state for every agent they remove. Today they don't (§3.1).
- Nothing is deleted before the process it describes has stopped. The codebase
  already states this rule in one place — `wcore::delete_tab_inner`: "Kill
  shell processes FIRST — must happen before DB cleanup"
  (`agentmux-srv/src/backend/wcore/tab.rs:102`) — and breaks it in two others
  (§3.3).
- An agent block that is in a tab but in no pane is a bug, not a state. It is
  found and shut down (§4.9), never left running.

## 2. The incident (2026-09-18, live instance, verified)

Two agents, AgentA (block `949f8ab0…`) and Manoz (block `4a291ef5…`), were tabs
in the same pane, AgentA visible. The user closed the pane at 22:35:05 UTC.
Evidence, all from the running instance's own data:

| Source | What it shows |
|---|---|
| Host log, `[BUG-TRACE] onNodeDelete` | `DeleteBlock` sent for `949f8ab0` at 22:35:05. Never for `4a291ef5`. |
| srv log | For `949f8ab0` only: `[process-tracker] dropped tracker on pane close`, `persistent process kill requested force:true`, then `session_recovery: compare-and-clear active pid failed … not found`. Manoz's last log line is 22:33:59; nothing at close. |
| `objects.db` (read-only) | `4a291ef5` still in `db_block`, still in Tab 1's `blockids`, in **no** `db_layout` tree. `949f8ab0` gone. |
| `objects.db` → `db_agents` | Manoz, AgentA and Posa all `status = running`, `ended_at = 0`. Posa's block no longer exists. `session_id` empty for all three. |
| `tasklist` | Manoz's `claude.exe` (pid 19176 = the block's `session:active_pid`) still running. |
| `FleetList` | Manoz `addressable: false, block_id: null` (unregistered from messaging), yet still listed in Swarm (still in `CONTROLLER_REGISTRY`). |

Swarm was reporting the truth: the controller was still registered. The bug is
that nothing ever removed it.

## 3. How it worked before Phase 1 (verified in code)

This section records the code as the audit found it. §5.2 lists what Phase 1
changed.

### 3.1 Close paths

| Action | Path | Backend gets | Every agent stopped? |
|---|---|---|---|
| A tab's own × | `closeBlockInStack` (`frontend/layout/lib/layoutStack.ts:162`) removes that member from the stack, then deletes its block; last member falls through to the pane path | `DeleteBlock` for that tab | Yes, for that tab |
| **The pane's ×** | `closeNode` (`frontend/layout/lib/layoutMagnify.ts:41-76`) removes the whole leaf, then calls `onNodeDelete(nodeToDelete.data)` **once**; `onNodeDelete` (`frontend/app/tab/tabcontent.tsx:55-65`) deletes `data.blockId` only | `DeleteBlock` for the **visible tab only** | **No — every other tab is orphaned** |
| Close a window tab | `delete_tab` saga (`agentmux-srv/src/sagas/delete_tab.rs`) | `CloseTab` | Yes, loops over the tab's `block_ids` (sequentially, hard kill) |
| `ClosePane` MCP tool | `agentmux-srv/src/server/app_api/pane.rs:416-481` → `delete_block` saga, whose `LayoutDeleteNodeByBlock` step removes the whole leaf (`find_node_id_by_block`, `agentmux-srv/src/backend/layout/mod.rs:145`) | one block | **No** — issue #3202 |

Issue #3202 describes the `ClosePane` case and states the frontend path is
fine. That is true of a tab's own × and not of the pane's ×.

### 3.2 Persistent controller stop is always a hard kill

- `PersistentSubprocessController::stop(&self, _graceful: bool, …)` ignores its
  argument and calls `stop_process(true)`
  (`agentmux-srv/src/backend/blockcontroller/persistent.rs:4546`).
- The kill task's force branch is `child.kill()`; its graceful branch already
  exists — drop `stdin_tx` (EOF), clear queued sends, wait for exit up to 5s,
  then kill (`persistent.rs:4132-4174`). Its only production caller is the
  deferred restart for a settings change (`persistent.rs:3305`).
- `stop_process` only sends on a channel and returns; no caller can await the
  process actually exiting (`persistent.rs:4404-4429`).
- **Even an honored graceful flag would be cut short:** `delete_controller`
  (`agentmux-srv/src/backend/blockcontroller/mod.rs:348`) calls `stop()` and
  then, on the next line, drops the block's process tracker. On Windows that
  closes a Job Object with kill-on-close, killing the whole process tree
  immediately (see `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` for
  that contract).

### 3.3 Records are deleted before the process is stopped

`delete_block::run` (`agentmux-srv/src/sagas/delete_block.rs:74-147`) runs, in
order: reducer `DeleteBlock` (block record and meta gone) → `LayoutDeleteNodeByBlock`
→ queue a frontend `DeleteNode` action → **then** `delete_controller`.
`delete_tab` does the same (records, then `delete_controller` per block,
`delete_tab.rs:145-150`). Consequence seen in the incident: the controller's
post-exit cleanup tries to clear `session:active_pid` on a block that no longer
exists.

### 3.4 What is saved, and where

- The session id is persisted **continuously** as the CLI reports it, not at
  shutdown: block meta `agent:sessionid`, the local instance row via
  `sync_instance_session_id` (`agentmux-srv/src/backend/blockcontroller/core.rs`),
  and the shared registry record. The shared registry is what reopen reads
  (`agentmux-srv/src/server/app_api/agent_open.rs:524-545`).
- The CLI writes its own transcript as it goes. A turn still running at close
  is cut off.
- Nothing ever sets the local instance row to `stopped` / `ended_at`. The only
  production writer of that row is `sync_instance_session_id`
  (`core.rs:197`); `status: "stopped"` is only written when an agent is first
  defined.
- `list_active()` on the shared registry
  (`agentmux-srv/src/registry/store.rs:338`) means "not forgotten" (not
  retired/tombstoned), not "running". Marking an agent stopped must not retire
  its registry record, or reopen stops finding its session.

### 3.5 The close confirmation is bypassed and too narrow

`useAgentCloseConfirm` (`frontend/app/view/agent/hooks/useAgentCloseConfirm.ts`)
replaces `onClose` on the agent's own `nodeModel`. That object is a spread copy
made by `PaneLeafChrome` (`frontend/app/tab/pane-leaf-chrome.tsx:181`, or `:238`
for kept-alive tabs, which agent panes use since
`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md`); the
pane header's × calls `onClose` on the pane's `nodeModel`
(`frontend/app/block/blockframe.tsx:251-256`). The replacement is never reached.
Even when it was, it counted only the visible tab's tracked processes and
ignored other tabs and a turn in progress. (Verified by reading the code, not
yet reproduced live.)

### 3.6 Reopen can collide with a still-running orphan

- `agent.open` checks for a live controller elsewhere and, if one exists,
  seeds no session id — the agent starts fresh (continuity lost, but
  announced) (`agent_open.rs:524-545`).
- The My Agents picker's reattach (`handleReattach`,
  `frontend/app/view/agent/components/AgentPicker.tsx:390-437`) passes
  `continueSessionId: row.session_id` with no such check. Reopening an orphan
  from there starts a second `claude --resume` on the same session.

### 3.7 Nothing reaps an orphaned block

The existing prunes run in the other direction — they drop layout references to
blocks that no longer exist (`prune_dangling_block_refs` /
`prune_dangling_stack_members`, `agentmux-srv/src/backend/layout/mod.rs:189,281`;
the frontend's equivalent in `frontend/layout/lib/layoutPersistence.ts`).
Nothing handles a block that exists in a tab but in no layout.

## 4. Design

### 4.1 One backend operation closes a whole pane

Add a backend `ClosePane` saga that takes the tab and **every** block in the
pane's stack, and runs §4.2 for each. The pane's × calls it with the leaf's full
`blockStack` (or `[blockId]` when there is no stack). The `ClosePane` MCP tool
routes through the same saga, closing issue #3202 at the same time.

Do **not** fix the frontend by sending one `DeleteBlock` per tab. Each
`delete_block` run queues a frontend `DeleteNode` for a leaf that is already
gone; that is the source of the `Cannot apply eventbus layout action
DeleteNode, could not find leaf node with blockId …` errors already seen in
logs. The new saga prunes the layout once, after all members are handled.

A tab's own × keeps using `closeBlockInStack`, but its backend step moves onto
the same per-agent shutdown (§4.2) so both paths are identical per agent.

### 4.2 Per-agent shutdown order

For each agent block, strictly in this order:

1. **Stop routing new input to it.** Unregister it from messaging (jekt/muxbus
   delivery) and refuse new `AgentInput` for that block, so nothing can start a
   turn mid-shutdown.
2. **Ask the process to stop.** See §4.3 for what "gracefully" means while a
   turn is running.
3. **Wait for it to exit**, up to the grace period (reuse the existing 5s).
4. **Force-kill** if it hasn't exited.
5. **Only now drop the process tracker** (the Job Object / cgroup / process
   group), so anything the agent itself started dies with it.
6. **Save final state:** clear `session:active_pid` on the block (which still
   exists), set the local instance row `status = stopped` and `ended_at`, keep
   the session id in the shared registry record. Never retire that record.
7. **Delete the block record**, remove it from the tab, forget it in the
   Process Broker (which tells Swarm).

After all members: prune the layout leaf once.

### 4.3 Graceful stop mechanics

- Add an async `shutdown_controller(block_id, grace)` in
  `agentmux-srv/src/backend/blockcontroller/mod.rs` that does steps 1–5 and
  **returns when the process has actually exited**. `delete_controller` stays
  as the immediate, forced variant for paths that need it.
- ~~`PersistentSubprocessController::stop` honors `graceful`~~ — superseded
  by §9.3: every existing `stop(true, …)` caller relies on an immediate stop,
  so graceful stopping gets its own entry point, `Controller::shutdown`. The kill task gains a way to
  signal completion (a oneshot or notify) so `shutdown_controller` can await it
  instead of assuming.
- **Turn in progress at close:** closing a pane is the user choosing to stop,
  so interrupt the turn (the control protocol's interrupt, see
  `docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`), wait for the
  turn's `result` event, then send EOF. Phase 0 (§5.1) showed EOF alone does
  **not** end a running turn: the CLI finishes the turn first, so with a 5s
  grace every mid-turn close would end in a force-kill.
- **Idle at close:** EOF alone; the CLI exits cleanly in under half a second.
- **Exit code after an interrupt is 1** (the turn's result is
  `error_during_execution`). The controller's exit handling must treat that as
  the requested stop, not a crash — no failure banner, no crash-restart.

### 4.4 Several agents in one pane

Shut members down **concurrently** with one shared deadline (the grace period
plus a small margin), not one after another. Five tabs must not mean 25
seconds. The layout prune and the confirmation (§4.6) cover the pane as a
whole.

### 4.5 The UI does not wait, the backend guarantees

Keep removing the pane from the UI immediately; a 5s wait before the pane
disappears would feel broken. The backend saga owns completion:

- A failed or rejected close is reported to the user, not swallowed (today
  `onClose` runs `closeNode` through `fireAndForget`, `frontend/util/util.ts:87`,
  which catches the error, logs it to the console, and shows the user nothing).
- If the saga cannot complete, the reaper (§4.9) finishes the job.

### 4.6 One confirmation for the whole pane

Replace the `nodeModel.onClose` monkey-patch with a check in the pane-level
close handler. It asks the backend, for every member, whether a turn is active
or tracked processes are running, and shows one confirmation listing the
agents affected. If nothing is running, no prompt.

### 4.7 Reopen is safe

- The picker's reattach path gets the same live-elsewhere check `agent.open`
  already has. If the agent is live elsewhere, offer to switch to it instead of
  starting a second process on the same session.
- With §4.1 and §4.9 in place, "live elsewhere" should only ever mean a real,
  visible pane.

### 4.8 Sub-blocks

An agent block's sub-blocks (e.g. the agent shell drawer's `subblockids`) are
torn down inside the same ordered shutdown, before the parent's records are
deleted — not before step 1 and not after step 7. The existing child cleanup at
`agentmux-srv/src/server/mod.rs:1636` calls `delete_controller` on a child
block id and is the place to align.

### 4.9 Orphan reaper

A backstop, not the fix. Find agent blocks that are in a tab's `blockids` but in
no layout leaf or stack, and run §4.2 on them. Run at startup and periodically.
It must **not** reap:

- a block just created with `pane.open { skip_placement: true }`
  (`agentmux-srv/src/server/app_api/mod.rs:173`), which sits in the tab unplaced
  until the frontend pushes it onto a stack — require a minimum age;
- sub-blocks (their parent is a block, not a tab);
- blocks in floating/torn-off windows, or anything mid-drag (the frontend prune
  already skips while a drag is in flight);
- blocks of tabs whose layout has not loaded yet.

The first reaper run also cleans up orphans left by the bug today.

## 5. Phases and tests

**Phase 0 — measure before building (no user-visible change).** For a
stream-json persistent session: send EOF while idle and while a turn is
running; send the control-protocol interrupt then EOF. Record exit time, exit
code, and whether the session resumes intact afterwards. This decides §4.3.

### 5.1 Phase 0 results (2026-09-18)

Measured against the pinned CLI (`@anthropic-ai/claude-code` `bin/claude.exe`
from the 0.56.6 instance), launched with the production flags
(`--input-format stream-json --output-format stream-json --verbose
--include-partial-messages --permission-prompt-tool stdio
--dangerously-skip-permissions`, `--model haiku`), after an `initialize`
control request. The mid-turn cases asked the model to run `sleep 20` via Bash
and acted 2s after the `tool_use` event. Times are from the action.

| Case | What happened | Exit | Resume afterwards |
|---|---|---|---|
| EOF while idle | exits | 0, after 0.4s | — |
| EOF mid-turn | **keeps running**: finishes the tool call and the turn (`result: success`), then exits | 0, after **22.8s** | intact — the model recalls the command |
| Interrupt, EOF 1.5s later | interrupt acked in 13ms; `result: error_during_execution` just after the EOF | 1, after 1.9s | intact |
| Interrupt, EOF 6s later | `result: error_during_execution` 2.3s after the interrupt, with stdin still open; exit after EOF | 1, 0.5s after EOF | — |

So the interrupt on its own ends the turn in about 2s, EOF then exits in about
0.5s, and neither loses the session. §4.3 is written from this.

**Phase 1 — every agent in a closed pane stops, in the right order.**
`ClosePane` saga (§4.1), per-agent order with the existing hard kill (§4.2
steps 1, 4–7), pane × wired to it, `ClosePane` MCP routed through it (#3202).

### 5.2 Phase 1 as implemented

- `agentmux-srv/src/sagas/close_pane.rs`: `close_pane::run(block_ids)`. Per
  block, `shutdown_before_delete` (mark closing, unregister from messaging,
  `delete_controller`, clear `session:active_pid`, instance row `stopped` +
  `ended_at`), then `DeleteBlock`. The leaf is deleted, and one frontend
  `delete` queued, only when no member of it survives — so one id from a
  tab's own × closes that tab and keeps the pane.
- `delete_block` runs the same `shutdown_before_delete` before its reducer
  dispatch, so a tab's × and the pane's × stop an agent identically.
- A closing block is refused by `resync_controller`
  (`blockcontroller::mark_closing`), so nothing can respawn it between the
  stop and the record delete.
- Frontend: `onNodeDelete` (`frontend/app/tab/tabcontent.tsx`) sends
  `ObjectService.ClosePane(effectiveStack(data))`.
- `ClosePane` MCP (`agentmux-srv/src/server/app_api/pane.rs`) expands the
  target to its leaf's members (`backend::layout::find_leaf_containing_block`).
- **Not yet reordered: `delete_tab`.** It still deletes records before
  stopping controllers. Stopping first there needs care: its reducer step can
  refuse (last-tab guard), and stopping every agent in a tab that then stays
  open would be worse than today. It moves with Phase 2.

**Phase 2 — graceful, and the same chain for every close.** An agent is
interrupted, allowed to exit, and only then has its process tree killed and
its records deleted. All four ways of closing an agent (a pane tab's ×, a
pane's ×, closing a window tab, closing a window) run that one chain, with
the agents stopped concurrently under one deadline. Detailed design: §9.

**Phase 3 — the edges.** Pane-level confirmation (§4.6), reopen guard (§4.7),
reaper (§4.9), user-visible failure (§4.5).

Tests (each must fail on today's code):

- Closing a pane with stack `[A, B, C]` stops all three controllers and removes
  all three blocks; none left in `tab.blockids`; layout pruned once; no
  `DeleteNode`-not-found error.
- The same through the `ClosePane` MCP tool on any single member (#3202).
- Ordering: the controller is gone and `session:active_pid` cleared **before**
  the block record is deleted; the process tracker is dropped only after the
  process exits.
- Graceful: a process that exits on EOF within the grace period is never
  force-killed; one that doesn't is killed at the deadline; three members finish
  within one grace period, not three.
- The local instance row reads `stopped` with `ended_at` set; the shared
  registry record keeps its session id and is not retired; reopening resumes it.
- Picker reattach refuses to start a second process for an agent that is live
  elsewhere.
- Reaper: reaps an unplaced block past the minimum age; leaves a fresh
  `skip_placement` block, a sub-block and a floating-window block alone.

Note for local verification: most test suites under `frontend/app/view/agent/`
don't load on a machine whose Node is older than the repo's pin
(`ERR_INVALID_ARG_VALUE … @solid-refresh`, recorded in
`TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md` §3.2). CI is the
authoritative run.

## 6. Not in scope

- **Keeping an agent running with no pane.** This spec keeps the existing rule:
  a closed pane's agent stops ("tracked ⇒ dies with the pane",
  `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`). A headless or
  detached-agent feature would be its own spec.
- **In-turn backgrounding** of long tool calls (keeping an agent available while
  a dev server runs) — `TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`.
  Unaffected.
- **App and OS shutdown** — `SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md`.
  The per-agent shutdown here is a natural building block for it, but wiring
  app exit to it is that spec's work.
- **Making pane tabs reducer commands.** The reducer stores each leaf's
  `block_stack` but has no command that changes it: every add, switch and
  remove is a frontend edit followed by a whole-tree `LayoutSetTree` push
  (`frontend/layout/lib/layoutStack.ts`, header comment). So "a block exists"
  and "a block is in a pane" are two separate writes that can come apart —
  another way to produce the orphan this spec's reaper backstops. Stack
  commands in the reducer (push / set-active / remove, and create-and-place as
  one step) would make that invariant enforceable in one place. A follow-up
  spec, not a phase of this one.
- **Using the interrupt for the Stop button.** Phase 2 makes the controller
  able to interrupt a turn without killing the process. Stop / Esc
  (`persistent.rs` `send_input`, SIGINT) could then stop a turn and keep the
  process warm instead of hard-killing it. Worth doing; a separate change,
  because it alters what Stop means to the user.

## 7. Open questions

1. ~~How does the pinned Claude CLI behave on stdin EOF mid-turn in
   stream-json mode?~~ Answered in §5.1: it finishes the turn. Interrupt first.
   Other providers (Codex via app-server, the ACP providers) are not measured.
   §9.3 requires that measurement before their graceful path lands; the
   deadline is their backstop meanwhile.
5. A turn blocked on a permission prompt (the CLI's `can_use_tool` request
   over `--permission-prompt-tool stdio`) has an open request AgentMux owes an
   answer to. Does the interrupt cancel it, or must the shutdown deny it
   first? Measure alongside §9.3's other-provider runs.
2. Why is the local instance row's `session_id` empty for every agent in the
   incident instance, when `sync_instance_session_id` looks the row up by block
   id and the row's `last_block_id` matches? Reopen reads the shared registry,
   so continuity is not known to depend on it — but
   `SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md` §3.1 says the
   recent-sessions list prefers the local row. Needs a check before Phase 1
   relies on either.
3. Should a pane close during an active turn wait for the turn to finish instead
   of interrupting it? This spec says interrupt (the user asked to close); the
   confirmation in §4.6 is where a user who wants the turn to finish can cancel.
4. Is 5s the right grace period for several concurrent members?

## 8. The incident's orphan

Separately from the fix: the orphaned Manoz process from §2 can be stopped by
pid, and its block removed, by hand. The Phase 3 reaper would do the same
automatically.

## 9. Phase 2 in detail

### 9.1 What Phase 1 left

After Phase 1, every close path stops every agent it should, but:

- The stop is still `delete_controller`: a hard kill, followed on the next line
  by dropping the process tracker, which kills the whole tree
  (`blockcontroller/mod.rs`). Nothing waits for the process to exit.
- A turn in progress is cut off mid-write. Phase 0 (§5.1) showed the CLI keeps
  the session resumable even so, but the turn's last output is lost and any
  tool it was running dies mid-step.
- Closing a window tab (`delete_tab` saga → `wcore::delete_tab_inner`,
  `agentmux-srv/src/backend/wcore/tab.rs:100`) and closing a window
  (`delete_workspace` saga, which relies on the same `delete_tab_inner`,
  `agentmux-srv/src/sagas/delete_workspace.rs:136-145`) never go through
  `shutdown_before_delete`. The reducer drops the tab and its blocks first,
  then the persist step hard-kills and deletes rows. No respawn guard, no
  messaging unregister, no final state.
- Agents are stopped one after another. With a hard kill that costs nothing;
  with a grace period it would cost 5s per agent.

### 9.2 One shutdown, used by every close

```
shutdown_agents(block_ids, deadline) -> Vec<StopOutcome>
```

in `agentmux-srv/src/sagas/close_pane.rs` (renamed `agent_shutdown.rs` if it
outgrows panes). For each block, **concurrently**:

1. Mark closing (refuses `resync_controller`) and unregister from messaging —
   unchanged from Phase 1.
2. Remove the controller from `CONTROLLER_REGISTRY` so no new input reaches it.
3. `ctrl.shutdown(deadline).await` (§9.3) — returns when the process has
   exited, whether on its own or by force at the deadline.
4. Only then drop the process tracker, killing anything the agent left behind.
5. Save final state (Phase 1's `save_final_state`).

It resolves when every block has finished step 5. Records are deleted by the
caller after that, exactly as Phase 1 does. `delete_controller` stays as the
immediate variant for the paths that must not wait (spawn rollback in
`server/mod.rs`, crash cleanup).

Callers:

| Close | Saga | Blocks passed |
|---|---|---|
| A pane tab's × | `delete_block` | that block |
| A pane's × | `close_pane` | every member |
| A window tab | `delete_tab` | every block in `tab.block_ids` |
| A window | `delete_workspace` | every block in every tab of the workspace |

One deadline per close, not per agent: `deadline = now + GRACE` with
`GRACE = 5s` (the existing constant in the kill task). Ten agents in a window
take about 5s, not 50s.

### 9.3 `Controller::shutdown`

Add to the `Controller` trait (`blockcontroller/mod.rs:206`):

```rust
/// Stop gracefully: end any turn in progress, ask the process to exit, and
/// force-kill at `deadline`. Resolves once the process has actually exited
/// (or there was none). Does NOT drop the process tracker — the caller does
/// that after this resolves.
fn shutdown(&self, deadline: Instant) -> BoxFuture<'static, StopOutcome>;
```

`StopOutcome` is one of `NotRunning`, `Exited { code }` or `Killed`. It is
logged, and the `Killed` count feeds §9.8.

The default implementation calls `stop(true, STATUS_DONE)` and resolves
immediately. That is today's behavior, kept for shell, cmd, subprocess and
tsunami controllers. Each has its own story (a PTY's shell exits on SIGHUP;
a per-turn subprocess has no idle process), and none is an agent session
whose state matters here.

Implemented for:

**`PersistentSubprocessController`** (Claude stream-json), using §5.1:

1. Nothing spawned yet (lazy spawn) → `NotRunning`.
2. Clear `pending_send_messages` so no queued message starts a new turn.
3. **If a turn is active** (`health_monitor.is_active_turn()`,
   `persistent.rs:993`):
   - set `stop_requested` on the controller before sending anything (§9.4);
   - write the control-protocol interrupt
     (`{"type":"control_request","request_id":…,"request":{"subtype":"interrupt"}}`)
     to stdin;
   - wait for the turn's `result` frame, but no later than `deadline − 1s`.
     The stdout reader already recognises result frames (`persistent.rs`,
     `is_result_frame`); it signals a `Notify` the shutdown waits on.
   
   Measured: about 2s.
4. Send the graceful stop through `kill_tx`, carrying the deadline instead of
   today's `bool`: `enum KillRequest { Force, Graceful { deadline } }`. The
   graceful branch (`persistent.rs` kill task) already drops `stdin_tx` for
   EOF and then waits; it waits until `deadline` instead of a fixed 5s, then
   kills. Measured: 0.4–0.5s to exit after EOF.
5. The kill task's cleanup ends by publishing the exit on a
   `watch::Sender<Option<StopOutcome>>` held in the controller. `shutdown`
   awaits that — the completion signal §3.2 found missing.

**`stop(graceful, …)` is left as it is, an immediate stop.** §4.3 proposed
making it honor its flag, but every caller passes `true` and relies on the stop
being immediate. That includes:
- the hung-process watchdog (`watchdog.rs:64`, `:82`);
- controller replacement (`blockcontroller/mod.rs:222`, `:307`);
- `delete_controller` (`mod.rs:375`);
- the app-server controller's own paths.

Flipping the flag's meaning would quietly slow every one of them. `shutdown`
is the only graceful entry point. A later cleanup can rename or remove the
unused flag.

**ACP controllers** (`acp.rs:658`): if a turn is active, send ACP
`session/cancel`, then the `shutdown` request and `exit` notification it
already sends, then wait for exit until the deadline, then kill. **App-server
controllers** (`app_server_controller.rs:484`): if a turn is active, send the
app-server's turn interrupt, close stdin, wait until the deadline, then kill.

Neither provider family's exit behavior has been measured (§7 Q1). Before
either lands, a §5.1-style measurement must be run for it. Until then, the
deadline guarantees they are never worse than today: at worst they are
killed 5s later than now.

### 9.4 A requested stop is not a failure

Phase 0: an interrupted turn ends in `result` with `is_error: true`,
`subtype: error_during_execution`, and the process then exits with code 1.
Today the stdout reader classifies any `is_error` result as a failure,
persists `agent:last_failure` and publishes `EVENT_AGENT_FAILURE`
(`persistent.rs`, the `is_error_result && !hold_back_for_resume_retry` arm).
It also feeds the stale-`--resume` retry machine. Neither must happen for a
stop we asked for:

- The shutdown sets `stop_requested` (with the spawn generation) **before**
  writing the interrupt. The result arm skips failure classification and the
  resume-retry capture for that generation while the flag is set.
- The exit is already handled correctly: a stop requested through `kill_tx`
  exits through the kill arm, which records `StopRequested` and never
  classifies the exit code. That holds as long as the shutdown takes the
  `kill_tx` path after the interrupt, which step 4 does.

Without this, every mid-turn close would record a failure on its way out, and
it would be sent to the notification sound service and to any other open
view of that block.

### 9.5 Closing a window tab and a window

`delete_tab` becomes:

1. Pre-check as today, plus the reducer's own last-tab guard
   (`workspace.tab_ids.len() <= 1` and `!force`, `reducer/tab.rs:114`),
   checked **before** anything is stopped. Refusing after stopping every agent
   would leave a tab full of stopped agents — the reason Phase 1 left this
   path alone.
2. To keep that check true until the dispatch, hold a per-workspace close lock
   (an async mutex keyed by workspace id) from the check through the
   `DeleteTab` dispatch. Two concurrent closes of the last two tabs then
   serialise, and the second is refused before it stops anything.
3. `shutdown_agents(tab.block_ids, deadline)`, then `DeleteTab`.
4. `delete_tab_inner`'s `delete_controller` loop stays as the backstop; it
   finds nothing left to stop.

`delete_workspace` (window close) runs `shutdown_agents` over every block of
every tab once, up front, with one deadline, and then its per-tab `DeleteTab
{ force: true }` dispatches as today. It has no last-tab guard to respect
(`force: true`).

**The last window is special.** Closing it is the user quitting. On that
close, `window_close.rs` saves the restore-on-relaunch snapshot and then
deletes the workspace. That snapshot must be taken **before** the agents are
stopped, as now, so it records the open tabs. Whether the launcher waits for
srv to finish a 5s shutdown on quit is
`SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md`'s question, not this
spec's. If it doesn't wait, those agents are hard-killed exactly as today, and
their sessions still resume (§5.1).

### 9.6 What the user sees

- **Pane ×:** the pane disappears at once, as now (§4.5). The backend finishes
  within the deadline.
- **Window tab ×:** the tab strip hides the tab optimistically on confirm, as
  now (`frontend/app/tab/tabbar.tsx`, `handleClose`); `CloseTab` now takes up
  to about 5s to resolve instead of milliseconds. The hide already survives an
  in-flight RPC; no UI change is needed. Its `holdRevealGate` has its own 800ms
  cap and does not depend on the RPC.
- **Window close:** the window closes at once. The saga completes in the
  background.
- **A closed-pane agent reopened within the grace window.**
  `agent.open`'s live-elsewhere guard (`agent_open.rs:524`) treats a block as
  live when `get_controller(block).is_some()`. Step 2 of §9.2 removes the
  controller before the process has exited. Unchanged, the guard would
  therefore call a still-exiting agent "not live" and seed its session id,
  and two processes would `--resume` one session for up to 5s.
  
  Instead, `shutdown_agents` registers each closing block's completion (a
  `watch::Receiver<Option<StopOutcome>>` keyed by block id, next to
  `mark_closing`). The guard waits on it, bounded by the deadline, before
  deciding. By the time it decides, the old process is gone and the reopen
  resumes the session. The picker's reattach path (§4.7) uses the same wait.

### 9.7 Tests

Each must fail on the code after Phase 1:

- **Idle persistent agent:** `shutdown` closes stdin and the process exits
  with `Exited`; it is never force-killed; the tracker is dropped only after
  exit. Use a stub CLI script that exits on EOF.
- **Mid-turn persistent agent:** the stub echoes `result
  error_during_execution` on interrupt. The interrupt is written before EOF,
  the result is awaited, the process exits within the deadline, no
  `agent:last_failure` is written, and no `EVENT_AGENT_FAILURE` is published.
- **Stub that ignores both:** killed at the deadline; the outcome is `Killed`.
- **Concurrency:** a window tab with three agents whose stub takes 3s to exit
  each finishes in about 3s, not 9s. Assert under 5s.
- **Last-tab guard:** closing the last window tab refuses **without** stopping
  any agent in it. Two concurrent closes of the last two tabs stop exactly
  one tab's agents.
- **Window close** stops every agent in every tab of the workspace.
- **`stop(true, …)` stays immediate:** the watchdog's stop of a hung
  persistent agent still kills at once and does not wait for a grace period.
- **Reopen during the grace window** resumes the session instead of
  starting fresh.

### 9.8 Watching it in production

Log one line per closed agent: block, controller type, how it ended
(`NotRunning` / `Exited` / `Killed`), and elapsed ms. If `Killed` is common
for a provider, that provider's shutdown needs work, or the grace period is
too short (§7 Q4). `muxlog srv grep agent_shutdown` is enough; no metrics
system is needed.

### 9.9 As implemented

- `Controller::shutdown`, `StopOutcome` and `SHUTDOWN_GRACE` are in
  `agentmux-srv/src/backend/blockcontroller/mod.rs`. The default is today's
  immediate stop.
- The persistent (Claude stream-json) controller implements it as §9.3
  describes (`persistent.rs`, `fn shutdown`):
  - the kill channel carries `KillRequest::{Force, Graceful(deadline)}`;
  - the kill arm records `stop_exit` the moment the process ends;
  - `shutdown_generation` makes the stdout reader treat the interrupted turn's
    `is_error` result as an ordinary end of turn (§9.4).
- `shutdown_agents` (`agentmux-srv/src/sagas/close_pane.rs`) is used by
  `delete_block`, `close_pane`, `delete_tab` (under `workspace_close_lock`)
  and `delete_workspace`. It logs one `agent_shutdown` line per block.
- **ACP and app-server controllers keep the default immediate stop.** §9.3
  requires a measurement of each provider's exit behavior before either gets a
  graceful path; neither has been run yet.
- The reopen guard (`agent_open.rs`) waits on `wait_closing_stopped` for this
  agent's closing blocks before deciding whether it is live elsewhere. The
  picker's reattach (§4.7) is Phase 3 and does not wait yet.
- Tests:
  - `persistent::shutdown_tests` runs a node stub that behaves as §5.1
    measured: a mid-turn close is interrupted and exits with no failure event,
    an idle close exits on EOF, and a stub that ignores both is killed at the
    deadline. The tests skip, with a note, if `node` isn't on PATH.
  - `close_pane::tests` covers concurrency, the last-tab guard refusing
    without stopping anything, window-tab close, and the closing-wait signal.
