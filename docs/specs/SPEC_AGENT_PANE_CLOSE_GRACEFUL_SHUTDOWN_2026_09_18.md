# SPEC: Closing a pane shuts down every agent in it — gracefully, in order

**Date:** 2026-09-18
**Status:** proposed — design only, nothing implemented yet. Lands with its
implementing PR, not as a docs-only change.
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

## 3. How it works today (verified in code)

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
- `PersistentSubprocessController::stop` honors `graceful` by sending `false`
  to the kill task (the existing graceful branch). The kill task gains a way to
  signal completion (a oneshot or notify) so `shutdown_controller` can await it
  instead of assuming.
- **Turn in progress at close:** closing a pane is the user choosing to stop,
  so the default is to interrupt the turn cleanly (the control protocol's
  interrupt, see `docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`), then
  send EOF. Whether EOF alone ends a stream-json session cleanly mid-turn is
  not yet verified — Phase 0 (§5).

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

**Phase 0 — measure before building (no user-visible change).** In a `task dev`
instance, for a stream-json persistent session: send EOF while idle and while a
turn is running; send the control-protocol interrupt then EOF. Record exit time,
exit code, and whether the session resumes intact afterwards. This decides §4.3.

**Phase 1 — every agent in a closed pane stops, in the right order.**
`ClosePane` saga (§4.1), per-agent order with the existing hard kill (§4.2
steps 1, 4–7), pane × wired to it, `ClosePane` MCP routed through it (#3202).

**Phase 2 — graceful.** `shutdown_controller`, `stop` honoring `graceful`,
completion signal, concurrent members (§4.3, §4.4).

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

## 7. Open questions

1. How does the pinned Claude CLI behave on stdin EOF mid-turn in stream-json
   mode (Phase 0)? Interrupt-then-EOF is the default until measured.
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
