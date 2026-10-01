# SPEC: a second pane of an agent that is live in another pane of the same AgentMux must recover, not go silent

**Date:** 2026-10-01
**Status:** active — Phase 0 (an instance-wide open-agent map for My agents, §4.0) shipped in PR #4127; see §11.
Phases 0.5–3 are not yet built. §8's decisions are proposed at their recommended option and await the repo owner.
**Author:** Agent3 (UID `fb3e692d-caf9-48e3-b20a-e659361aa057`)
**Trigger:** Repo owner, 2026-10-01: Korp's pane accepted typed messages and did nothing. Asked for a permanent fix so
that *"if the same conditions arise it recovers"*.
**Researched against:** `agentmuxai/agentmux` `main` @ `85e861af3` (v0.59.2). The incident ran on v0.59.1.
**Extends:** `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` — that spec covers a holder in a *different* instance
(host/LAN/WAN) and gives it a Take over. This one covers the case it calls "degenerate" — the holder is another pane of
the **same** srv process — which in practice has no recovery at all.

**Related:**
- `SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md` — the stop path §4.3 reuses.
- `SPEC_ORPHAN_RECONCILER_CROSS_PLATFORM_LIVENESS_2026_09_20.md` — process liveness; this spec adds *pane* liveness.
- `SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md` — the resume gate the moved session goes through.

Evidence labels: **[verified]** I read the code or the logs myself · **[inferred]** reasoning, not observed. Times are
UTC.

---

## 1. What happened [verified from the v0.59.1 host and srv logs unless noted]

| Time | Event |
|---|---|
| 02:55:30 | Korp (UID `a4bbd01b…`) spawns in pane `aba62569…`, `claude.exe` PID 48444. Takes the agent lease. |
| 02:57:24 | That pane is torn off into a floating window (pool window `floating-b70338…`, x=2501 y=100, 799×698). A floating window is its own renderer, so the pane leaves the main window's in-memory pane registry. |
| (before 07:01) | In the main window's agent picker, **My agents shows Korp as not active**, so clicking it skips the "already open in another pane — switch to it?" prompt and starts a reattach (G0). |
| 02:59:17 | Its last turn ends. It stays idle from here on, and the process and lease stay alive (renewed every few seconds). |
| 03:05:00 | The floating window's always-on-top is switched off. It ends up behind the main window; it is still open at 08:37. |
| 07:01:27 | A second Korp pane, `633f5f68…`, resyncs with `forcerestart` and tries to eager-resume session `81fef94a…`. The spawn-time lease claim is refused: `agent_admission.denied: agent is live in another instance … holder_block: aba62569…, holder_channel: <this channel>`. Falls back to a "lazy" spawn on the next message. |
| 07:01:39, 07:01:44, 07:02:50 | Each typed message: `AgentInput` → the early admission check **passes** → the spawn env is built (identity resolved, `CLAUDE_CONFIG_DIR` injected, account bindings read) → the spawn-time lease claim is refused again. Nothing is surfaced in the pane. |
| 07:03:52 | The pane is moved to another tab; its Claude account is re-bound ("armory bind") and the controller force-restarts — same refusal. |
| 07:03 → 08:21 | The UI's activity tracker lists `633f5f68` as **busy**. No further `AgentInput` reaches srv, although the user keeps typing. |

The user's view: a Korp pane that takes messages and never answers. The real Korp was alive, idle, and hidden behind the
main window.

## 2. Why — six gaps

**G0. My agents can only see the window it is drawn in — the trigger.** [verified] A row's "active" state is
`openDefinitions.has(row.definition_id)` (`frontend/app/view/agent/components/MyAgentsList.tsx:998`). The map comes from
`getOpenDefinitionMap()` (`frontend/app/store/agent-pane-state-store.ts:467`), which walks `slots`, a module-local `Map`
(`:107`) holding the panes registered in **this renderer only**. Every floating/pool window is a separate renderer with
its own `slots`. The file says so itself at `:486`: *"Scope: THIS renderer only … a pane showing the same agent in another
window is not listed here."* So an agent open in a floating window, or in any other window, shows as not active. Clicking
it then skips the fork / "switch to existing" prompt and reattaches, creating the second pane that §1 fails on. The same
blind spot affects the identity bind menu (`frontend/app/view/identity/bind-to-agent-menu.ts:90`, `runningBlockId`) and
the delete sweep (`getOpenBlockIdsForDefinition`, `:492`). The srv already knows every agent pane in the process:
`AgentTrackedBlocks` (`crates/srv/src/server/app_api/agent_io.rs`, `ProcessBroker::list_agent_panes()` over
`get_all_controllers()`). It returns block ids only, and no UI surface uses it for this.

A side effect seen in §1: the muxbus registration for Korp moved to the refused pane (`DiscoverAgents` listed Korp at
`633f5f68`, not the live floating pane). So agent-to-agent messages for Korp were probably routed to the pane that can't
run. [inferred from the registry listing; the routing was not exercised]

**G1. The early check is blind to its own process.** [verified] `check_before_spawn` asks the lease store for
`live_holder_other_than(uid, own_boot_id)` (`registry/leases.rs:469`), which by construction ignores a holder with this
srv's boot id. That suits other instances, but a holder in *this* process passes the check. So the early check, which
exists to stop shared writes before a doomed spawn (single-live-instance spec I9), lets them happen. Each refused message
re-ran `build_persistent_spawn_env`, and at 07:03:52 a re-bind of the agent's account.

**G2. The refusal isn't shown.** [verified for the log; inferred for the mechanism] `run_agent_turn` has a
`surface_refusal` path (`server/agent_handlers/input.rs`) that persists an error frame and `agent:last_failure`. But no
frame, failure, or `persist_last_failure` line appears for `633f5f68` in the srv log after any of the four refusals. The
refusal from the spawn-time claim, rewritten as `held_elsewhere_error` in `segments.rs:73`, doesn't reach that path. The
pane got neither an error row nor its message back. Phase 0 pins down exactly where it is dropped.

**G3. The pane stays "busy".** [verified from the activity log; inferred cause] The frontend keeps `633f5f68` in
`busyCount` after the refusals. With the pane marked busy, later sends sit queued in the composer waiting for a turn that
never started. That matches "no `AgentInput` after 07:02:50".

**G4. No same-process recovery action.** [verified] `live_elsewhere` + Take over (`failure/takeover.ts`,
`POST /api/v1/agent/takeover`) only works for a holder in another instance, through `locate_holder` → `request_release`
over HTTP. With the holder in the same process, the only option is the message's own advice: *"Close it there, or
switch to it"*. That presumes the user can find "there". Here it was a floating window hidden behind the main window.

**G5. Nothing ever lets go.** [verified] The holder was healthy: alive, renewing, not fenced. Every existing mechanism is
correct to leave it alone. Nothing weighs *"idle for four hours in a window the user can't see"* against *"the user is
typing to this agent in the pane in front of them"*.

## 3. Requirements

- **R0 (one instance-wide view of open agents):** My agents, and every other "is this agent open?" consumer, reflects
  every agent pane in this AgentMux instance: all tabs, all windows, floating and pool windows included. The source of
  truth is the srv controller registry, not a renderer's in-memory pane list. A row for an agent open anywhere shows as
  active, and says where ("open in a floating window", "open in Tab 3"). Clicking it offers **Switch to it**, which
  focuses that window and pane, instead of starting a reattach.
- **R1 (no doomed shared writes):** a send to a pane whose agent is held by another pane of the same process is refused
  *before* `build_persistent_spawn_env` or any identity/account write. Same guarantee as I9, extended to this process.
- **R2 (never silent):** every admission or held-elsewhere refusal on the send path does all of the following:
  - persists a classified failure that the pane's recovery row reads;
  - clears the pane's turn-active/busy state;
  - returns the unsent message to the composer instead of discarding it.
- **R3 (recover by default):** when the holder pane is **idle**, a send in another pane of the same agent moves the
  agent to the pane the user is typing in, automatically, and delivers the message. The newest explicit user intent
  wins over an idle holder. Nothing is lost: the moved pane resumes the same session (§4.3).
- **R4 (busy holder is protected):** when the holder is mid-turn, the send is refused (R2) with two actions:
  - **Show it:** focus the holder's pane and raise its window, bringing a floating window to the front;
  - **Move here:** a confirmed move, the same-process twin of Take over.
- **R5 (orphans recover with no user action):** a holder whose pane is unreachable — its block is in no layout of any
  live window — is stopped and its lease released when another pane claims the agent. A periodic sweep also stops one
  that has been unreachable for a long time.
- **R6 (one session, never two):** at no point do two processes run `--resume` on one session. The move is stop → wait
  for exit → release lease → spawn, never overlapped.

## 4. Design

### 4.0 Instance-wide open-agent map (G0 → R0)

**srv:** extend `AgentTrackedBlocks` (or add `ListOpenAgentPanes` beside it) to return one entry per agent pane:
`{ block_id, agent_id (definition id, from block meta "agentId"), agent_name, window_label, tab_id, tab_name,
floating: bool, live: bool (controller has a process), turn_active }`, from `ProcessBroker::list_agent_panes()` joined
with block meta and the layout→window mapping. Publish one broadcast event, `agentpanes:changed`, whenever a controller
registers or unregisters, a block moves between tabs or windows (tear-off, redock, tab move), or a window opens or closes.
Every renderer receives it, unlike today's per-renderer `subscribeToPaneLifecycle`.

**frontend:** reimplement `useOpenDefinitionMap()` (`AgentPicker.tsx`) on that query, refetching on
`agentpanes:changed` and on `agents:changed`. Keep `getOpenDefinitionMap()` only as an instant first paint for the local
renderer, merged under the srv result, never instead of it. `getOpenBlockIdsForDefinition` and the bind menu move to
the same source. The row's active label and its "Switch to it" target come from the entry's `window_label` / `tab_id`.
Switching calls the existing window-focus IPC, then focuses the block.

**Ordering:** the srv answer wins. A renderer that hasn't received `agentpanes:changed` yet still sees a pane its own
`slots` holds, so a just-opened local pane never flickers to "not active".

This alone prevents §1: the click would have offered "Switch to it" and never created the second pane. §4.1–§4.4 make
the system recover when a second pane is created anyway, through a race, a restore path, `OpenAgent` from an agent, or
an older client.

### 4.1 One in-process holder lookup, used early (G1 → R1)

Add `in_process_holder(uid) -> Option<InProcessHolder { block_id, turn_active, reachable }>` beside
`agent_held_by_other_pane` (`persistent/lifecycle.rs:52`), from the controller registry. `check_admission` calls it
**first**, before the lease-store, WAN, LAN and compat checks, and before `run_agent_turn` builds the spawn env. A hit
returns a typed `AdmissionOutcome::HeldByLocalPane(holder)`, not a free-text `Err`. The text-matching
`is_held_elsewhere_error` stays for the old paths only.

### 4.2 Decide: move, refuse, or reap

| Holder state | Action | User sees |
|---|---|---|
| unreachable (§4.4) | reap: stop it, release, spawn here | message goes through; a one-line note: "Korp was still running in a closed pane; moved here" |
| reachable, idle | **auto-move** (§4.3), then deliver | message goes through; a note in both panes: "Korp moved to the pane in Tab 2" / "Korp moved here from a floating window" |
| reachable, mid-turn | refuse (R2) | failure row `open_in_other_pane` with **Show it** and **Move here** (two-step confirm, as Take over) |

"Idle" means no turn active, no queued prompt, and no running background shell owned by the pane. A holder running a
background command counts as busy.

### 4.3 The move (R3, R4, R6)

Same process, so no HTTP:
1. Mark the holder block **handing over** (`hold_after_handover`, already used by Take over) so nothing re-spawns it.
2. Stop the holder through the graceful close path (`request_stop`, `SHUTDOWN_GRACE`), then `wait_closing_stopped`. The
   lease drops with its `HeldAgentLease`.
3. The holder pane stays open and becomes a plain **"moved to …"** stub with a "Bring it back" button (a move in the
   other direction). It is not closed: closing the user's pane for them is out of bounds.
4. The claiming pane spawns with `--resume <holder's session>`, through the resume gate as usual. The queued message is
   delivered as its first turn.

If any step fails, the claimant gets the R2 refusal with the real reason, and the holder is left as it was before step 1
(clear the handover hold).

### 4.4 Pane reachability (R5)

A holder block is **reachable** when its block object exists, and it is a leaf in the layout of some tab of some window
the host reports alive, including floating/pool windows. Main, floating and pool windows all report their layout through
`UpdateObject layout`, and the host already knows which window labels are alive. Expose that as one srv query
(`reachable_blocks()`), refreshed when a layout or window changes.

- **On claim:** an unreachable in-process holder is reaped (§4.2 row 1).
- **Sweep:** every 5 minutes, a persistent controller whose block has been unreachable for ≥ 15 minutes is stopped
  gracefully, logged as `agent_admission.reaped_unreachable_pane`. A grace period, because tear-off and redock
  briefly leave a block in no layout.

### 4.5 Surfacing and un-sticking (G2, G3 → R2)

- New failure class `open_in_other_pane` (beside `live_elsewhere`) in `agents/failure.rs` and the frontend
  `AgentFailure["code"]`. Its payload has `holder_block_id`, `holder_window_label`, `holder_tab_name` and
  `holder_turn_active`, so the row can say where the holder is ("open in a floating window") and what Show it targets.
- `run_agent_turn` routes `HeldByLocalPane` and the spawn-time held-elsewhere error through `surface_refusal`. Both must
  publish turn-inactive, so the frontend activity tracker drops the pane from `busyCount`.
- The frontend returns the refused message to the composer, marked "not sent". It is not left as a pending bubble.
- **Show it** calls the existing window-focus IPC for the holder's window label (`FocusWindow`; for a floating window,
  raise and briefly set always-on-top), then focuses the block.

## 5. Edge cases

- **Both panes idle, user types in both at once:** the per-agent `AGENT_OPEN_LOCKS` mutex serializes the claims. The
  second becomes a move back, which is allowed, and each move posts a note.
- **Holder is a sub-agent or fork pane:** sub-agents run under the parent's process and take no lease, so this does not
  apply. Fork panes have their own UID and do not collide.
- **Holder in a hidden tab of the main window:** reachable, so it auto-moves when idle. That's intended: "hidden" is
  exactly the case the user can't see.
- **Agent mid-tear-off:** the block is out of the layout for under a second. The 15-minute sweep grace and the "on claim"
  check (a user typing into a pane mid-drag is not possible) cover it. [inferred]
- **Different instance holds it:** unchanged. The single-live-instance spec's `live_elsewhere` + Take over applies.

## 6. Observability

`agent_admission.local_holder` (outcome = move|refuse|reap, holder block, idle, reachable),
`agent_admission.moved` (from, to, session, duration_ms), `agent_admission.reaped_unreachable_pane`. A refusal
always logs the `persist_last_failure` it produced, which is the line missing in §1.

## 7. Tests (each written to fail first)

0. Instance-wide map: an agent pane registered in a second renderer (floating window) appears in the main
   window's `useOpenDefinitionMap()` with `floating: true`. Today it doesn't (G0). It disappears when that pane closes,
   and it follows the pane through tear-off and redock without a gap.
0b. My agents row: an agent open only in a floating window renders active, and clicking it shows Switch to it, not a
   reattach.
1. Early check sees an in-process holder: two controllers, one UID. `check_admission` from the second returns
   `HeldByLocalPane` and `build_persistent_spawn_env` is not called (spy).
2. Refusal is surfaced: a mid-turn holder. The send returns `open_in_other_pane`, persists `agent:last_failure`, and
   publishes turn-inactive. Today this fails (G2).
3. Idle-holder auto-move: the holder stops, the lease moves, the claimant spawns with `--resume <same sid>`, the message
   is delivered, and the holder pane shows the moved stub.
4. No overlap (R6): instrument spawn and exit. The holder's exit is observed before the claimant's spawn.
5. Unreachable holder is reaped on claim; a reachable busy one is not.
6. Sweep grace: a block out of the layout for 10 s (a simulated tear-off) is not reaped. At 15 min it is.
7. Frontend: a refused send returns its text to the composer, and the pane leaves `busyCount`.
8. Live check (each phase): reproduce §1 on a test agent. Open it, tear it off, hide the floating window, open the
   same agent in a second pane, type. Expect the message to be answered in the second pane and the floating one to show
   the moved stub.

## 8. Decisions (proposed at the recommended option)

1. **Auto-move an idle holder (R3), vs. always ask.** Recommended: auto-move. The user typing into a pane is an explicit
   choice of where the agent should be; an idle holder has nothing to lose; and "ask" is what failed here, because the
   question pointed at a window the user couldn't find. Alternative: refuse with Show it / Move here even when idle.
2. **The holder pane after a move: stub, vs. close.** Recommended: stub with "Bring it back". Never close a pane the user
   didn't close.
3. **Sweep threshold.** Recommended: 15 minutes unreachable, checked every 5. Long enough to cover a slow tear-off or a
   window-restore after sleep, and short enough that a lost pane doesn't hold an agent for hours.

## 9. Not in this spec

- The blank `identity_id` warning on the second pane, and the 07:03:52 account re-bind. R1 stops the re-bind from
  happening for a refused pane; the blank identity is a separate issue.
- Floating windows that open partly off-screen (this one was 799 px wide at x=2501 on a 3072 px display).

## 10. Delivery order (one PR each, each ending with the §7.8 live check)

- **Phase 0:** the instance-wide open-agent map and Switch to it in My agents (§4.0, R0). Removes the trigger.
- **Phase 0.5:** find exactly where the spawn-time refusal is dropped (G2) and why the pane stays busy (G3); fix both. R2
  only. It ends the "nothing happens" symptom for any second pane that still gets created.
- **Phase 1:** in-process holder lookup in the early check (§4.1, R1), and the `open_in_other_pane` class with Show it
  (§4.5).
- **Phase 2:** the move: the auto-move for an idle holder, and Move here for a busy one (§4.2, §4.3, R3, R4, R6).
- **Phase 3:** pane reachability, reap-on-claim and the sweep (§4.4, R5).

## 11. Phase 0 as built

- **srv:** `agent.open-panes` (`COMMAND_AGENT_OPEN_PANES`, `crates/srv/src/server/app_api/agent_io.rs`). Same source as
  `agent.tracked-blocks` (`ProcessBroker::list_agent_panes()`). For each agent pane it returns
  `AgentOpenPane { block_id, agent_id, tab_id, tab_name, window_ids }`. `window_ids` are the `Window` oids on the pane's
  workspace. A floating window gets its own `Window` through `CreateWindow` with the tear-off workspace, so it is covered.
  Unit-tested by `open_panes_tests`, which includes the §1 layout: an agent in a floating workspace with its own window.
- **No new event.** Refresh rides `processbroker:tracked-blocks-changed`. That event is already published, with no
  scope, on every `register_controller` and on `release_block_processes` (pane close), so it reaches every renderer.
- **frontend:** `useInstanceOpenDefinitions()` (`AgentPicker.tsx`) merges srv's list with the local map
  (`frontend/app/view/agent/open-agent-panes.ts`, unit-tested). A pane in this window wins over one elsewhere, and the
  local map wins for a pane srv hasn't reported yet. My agents uses it:
  - the active badge and the "already open" prompt say where the pane is ("open in another window", or "open in another
    pane (Tab 3)");
  - **Switch to existing** goes through `revealBlock`, which already raises another window via `block.reveal`.
- **Deliberately unchanged:** `useOpenDefinitionMap` stays window-local for the fork tab strip, whose pills switch within
  the window. The identity bind menu (`bind-to-agent-menu.ts`) also stays window-local: its "running" target gets a
  `cmd:env` patch and a forced restart, and re-pointing that at a pane in another window changes what binding does. It is
  left for a follow-up with its own review.
- **Not live-verified in this PR:** the floating-window case is covered by the srv unit test and the merge tests. The §7.8
  live check (tear off an agent, open My agents in the main window) is still to do on a dev build.
