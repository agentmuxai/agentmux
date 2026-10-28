# Spec: always-available agents (an agent runs and is recorded whether or not a pane shows it)

**Status:** active — the operator approved decisions D1 to D10 as recommended (relayed by AgentX, 2026-10-27). Kanban #5823 (spec); the build is #5838 (P1) and #5840 (P2b). P1 is three PRs: A (the park and attach commands, the by-uid lookup, the setting, off) in #5641; B (the close paths park an agent's block: close-pane, tab-close, workspace-delete; a process exit keeps it; deleting the agent deletes it) in #5657; C (agent.open attaches a parked block or focuses the pane that shows the agent, the per-agent desired state, the headless start) in #5661.
**Date:** 2026-10-27
**Author:** AgentX@narko, with Manoz@Area54 (lifecycle), Camper@narko (Swarm) and AgentA@Area54 (srv core) answering questions.
**Verified against:** `agentmux` `main` @ `685c739a4` (2026-10-27), by reading code. Nothing here is built. Claims I could not check are listed in section 10.
**Builds on:** `SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md` (the one state and verdict for "can this agent take a message now"), `SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md`, `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24` (the per-agent lease).

## 1. The ask

The operator, in the card: "agents are always available, regardless of if their conversation window is open. So a leader can run an agent that doesn't necessarily have a pane open, but the conversation and all the info is still recorded, and the agent operates as if it were open. If a user were to open the agent via My Agents, it would see the conversation, whether idle or in progress. This also changes the operation of the swarm, where all created agents appear, instead of only the ones with open panes."

Five things follow:

1. An agent runs without a pane (started by a leader, a message, a Job, or the user).
2. Its conversation is recorded the same way as with a pane.
3. Opening it later (My Agents, Swarm) shows that conversation, idle or mid-turn, and live from then on.
4. Closing the pane does not stop it.
5. The Swarm lists every created agent, with its state, not only those with a pane.

## 2. What exists today

### 2.1 What is already agent-keyed (the good news)

| Thing | Keyed by | Where |
|---|---|---|
| One live process per agent (the lease) | agent uid | `backend/agent_admission.rs` (`acquire`, `HeldAgentLease`), checked by `agent.open` before it writes anything (`server/app_api/agent_open.rs:395`) |
| Held messages for a stopped agent (`not_running`, 24 h) | target uid | `server/jekt_hold.rs`, `jekt_held.rs` |
| Availability (running / not running / limited / signed out) | uid, read via the block | `backend/agent_availability.rs` (Manoz, #5585) |
| Identity token (`X-Agent-Token`) | uid | `backend/storage/agent_tokens.rs` |
| Agent definitions | uid | `agent_def_list`, `listagents` |

### 2.2 What is block-bound (the work)

A "block" is the object behind a pane. Today **the block is the agent's body**: the controller, process, transcript and session all hang off `block_id`, and closing the pane deletes the block.

| Thing | Keyed by block | Where |
|---|---|---|
| Controller (spawn, stdin queue, stdout/stderr readers, term size) | `block_id` | `backend/blockcontroller/mod.rs` `CONTROLLER_REGISTRY` |
| Process tracker (Windows job object / cgroup), Process Broker entry | `block_id` | `release_block_processes` |
| Transcript (the NDJSON the pane replays) | `block_id` in FileStore (`blockfile`) | `persistent/stdout_reader.rs` writes it, srv-side, with no frontend involved |
| Session id for resume (`agent:sessionid`), `session:active_pid`, `agent:last_failure` | block meta | `blockcontroller/core.rs`, `session_recovery.rs` |
| Supervisor state, seat, colour | block meta (`supervisor:state`, ...) | Swarm and Supervisor read them by block |
| Held input from the composer | block meta (`HELD_INPUT_META_KEY`) | `useOfflineInput.ts` |
| Reactive registration (agent -> block) | `AgentRegistration.block_id`, a required field | `backend/reactive/types.rs:491` |
| Swarm rows, selection, fleet actions | block id | `swarm-model.ts` (`agent.tracked-blocks`) |
| `agent.list`, `agent_panes_of`, account moves | iterate tabs -> blocks, filter on `agentId` meta | `server/app_api/agent_io.rs`, `account_episodes/moves.rs` |
| Cleanup on pane close | close-pane saga takes the controller and tears the process tree down | `sagas/close_pane.rs`, `delete_controller` (`blockcontroller/mod.rs:576`) |

### 2.3 What a pane itself owns

Very little. Findings from reading the frontend:

- **History** is fetched from srv (`blockfile:read_range`, a 10,000-line window) on mount, then the live stream is built from `BlockControllerEvent`s. A pane opened on an existing block already replays then tails.
- **Starting and stopping the process** does not happen on pane mount or unmount in the frontend. The kill on close is srv's, in the close-pane saga.
- **Questions and approvals** (AskUserQuestion, tool permission) are answered over the control protocol to the running process. A question with nobody looking sits as `awaiting_answer` in the transcript until someone answers.
- Compaction rows, Supervisor rows, account/quota UI are drawn from srv events and stored status; nothing is only in the pane.
- UI/Pane tools take an optional pane and fall back to the focused window.

**Conclusion of the inventory:** the pane is mostly a view already. The coupling is on the srv side: the block is both the view's anchor and the agent's body, and closing the pane deletes the body.

### 2.4 Nothing headless exists for a persistent agent

`agents/runner.rs` runs a one-shot `claude --print` for the Drone block (task in, result out, no session). Jobs and cron deliver to an agent that is already running; a fire to a stopped agent is missed or left pending. A message to a known stopped agent is held `not_running` and nothing starts it. Containers keep a Docker container alive between turns, but each turn is driven by a block's subprocess controller. There is no agent supervisor (the only `supervisor.rs` is LSP). After an srv restart a controller is rebuilt only when a frontend resyncs the block, so **no pane connected means no agent after a restart**.

## 3. The model

### 3.1 One idea

**An agent has one block. The block is the agent's body and lives as long as the agent does. A pane is a layout node that shows that block (a view). Panes come and go; the block, the process and the transcript do not.**

This is Manoz's "host block" seam, taken one step further: the host block is not a hidden second thing next to the pane. It is the same block the pane shows today.

- **Attach** = insert a layout node for the agent's existing block into a tab (what `agent.open` does when the agent's block already exists, but for any tab, not only "already open in this tab").
- **Detach** = remove the layout node and **park** the block: it stays alive, belongs to no tab's `block_ids`, and its controller keeps running.
- **Stop** = stop the process. The block (with its transcript) stays, shown as stopped, with the session id kept so the next start resumes.
- **Delete agent** = the only thing that deletes the block.

### 3.2 Why one block per agent, not a new runtime object

I considered a new `AgentRuntime` keyed by uid with blocks as pure views. It is the cleaner picture and the bigger change: the controller registry, the Process Broker, the process tracker, the transcript, the session meta, the Supervisor and Swarm state, `AgentRegistration.block_id` and the account-episode code are all block-keyed, and each would be re-keyed. With one block per agent, **block meta becomes agent meta for free**, every one of those keeps working unchanged for a headless agent, and the registration's required `block_id` stays satisfied. A block-less runtime can still be the later cleanup; nothing here blocks it.

### 3.3 Rules

1. **At most one block per agent uid** (enforced at creation: look up by uid first, then create). The duplicate-pane class cannot occur by construction, not only by refusal. The lease stays as the cross-instance guard.

   *Why this matters:* the RCA's cause was that at 02:01 a job called `agent.open` for an agent whose old pane was in the layout but not running, and `agent.open` made a second pane instead of reusing it; input typed into the duplicate went nowhere because its spawn was refused and the refusal was not shown. Loap confirms nothing in srv stops a second block for one agent from existing (a frontend `createBlock` with the live pane's meta produced one in a live test); only the spawn is refused. Loap's audit task builds a placement function that every pane-creating path (restore, import, fork, drag, tear-off, picker) calls. **That function is rule 1.** P1 builds it once, as "find the agent's block by uid, else create", and the audit's per-path tests become P1's tests.
2. **A block with an agent has 0 or 1 views.** (Decision D3 for more.)
3. **Closing a pane, a tab, or a window detaches. It never stops the agent.** The same for deleting a workspace. Stopping is a separate, visible action (Stop; Stop and close).
4. **The agent runs as if open.** Same controller, same permission mode, same account, same transcript, same Kanban claims, same Supervisor.
5. **An agent has a desired state, `running` or `stopped`**, stored on the agent (not the block's layout). Stop sets `stopped`; start, a message that starts it, and open set `running`. After an srv restart, agents whose desired state is `running` are resumed by srv with no frontend (staggered).
6. **`close_on_exit` and the orphan reaper never delete an agent's block.** A process exit makes the agent `stopped` (or `error`), shown as such.
7. **Who started it is recorded** (the sender, the Job, the user) on the agent's record. Actor attribution is unchanged.

### 3.4 The states a viewer sees

Reuse the availability module's states and add two the Swarm needs. For one agent: `stopped`, `starting`, `idle`, `working`, `waiting on you` (a question, approval or sign-in no one answered), `limited` (with reset if known), `signed out`, `error`. `AgentState` and `Availability::Running{waiting_on_user}` already carry most of this; the build adds `starting` and a single event stream carrying the whole set per agent id (`agentruntime:changed:<id>`, in the style of `agentidentities:changed:<id>`).

## 4. Decisions for the operator

Each has a recommendation. "Rec" is what I would build if you say "go with the recommendations".

| # | Decision | Rec |
|---|---|---|
| **D1** | Does closing a pane stop the agent? | **No: detach.** It is the point of the request. Stop is explicit. Cost is handled by D5 and D6, and a status-bar count of running agents. |
| **D2** | Hidden extra block, or the agent's own block parked? | **The agent's own block, parked** (section 3.1). A block-less runtime is a later cleanup if ever. |
| **D3** | May two panes show one agent at once? | **Not in the first build.** Opening an agent that is already shown focuses that pane, even in another tab or window. A second, read-only mirror is a P4 option. One writable view keeps one composer and one set of answers. |
| **D4** | Who may start a stopped agent by sending it a message? Starting costs money and a seat. | **Host-tier senders (agents on this machine, the user, the Kanban) and the operator's own Jobs/cron start it. LAN and WAN peers, and any unverified sender, are still held (`not_running`) until the agent next starts.** Same trust test as the hold today (`hold_applies`), plus the spawn gate and disk admission. |
| **D5** | Idle auto-stop for an agent with no view? | **Yes, after 60 minutes without a turn, a message or a view, per-setting (0 = never), per-agent override.** The agent stays addressable: a message starts it and resumes its session. An agent that is `waiting on you` never auto-stops. |
| **D6** | A cap on live agent processes? | **Yes.** Default derived from the host (to be set with Loap-3's numbers; placeholder 12 on a laptop). At the cap, the least recently active idle agent with no view is stopped to make room; if none can be, the start is held with reason `capacity` and the sender is told. |
| **D7** | After an srv restart, which agents come back? | **Those whose desired state is `running` and that were active within the idle window**, started staggered (a few seconds apart), skipping any that are signed out or limited. Others stay `stopped` and start on first message or open. |
| **D8** | An agent asks something and nobody is watching. | **It waits, and shows `waiting on you`** in the Swarm row, the Kanban "Needs you" column and the status bar, with a desktop notification; opening the agent shows the question. **No auto-answer, no timeout** for a question or approval. A sign-in or limit uses the existing availability states. The sender of a message that started it is told if it is waiting on the user. |
| **D9** | Roll-out. | **Behind a setting** `agents.always_available`, off through P1 and P2, on by default in P3 once the Swarm shows all agents, and the setting removed in P4 after migration. |
| **D10** | Jobs and cron firing at a stopped agent. | **Start it** (D4 covers it). Today it is missed or left pending. A Job that names an agent now means "run it", the same as it would with the pane open. |

## 5. Surfaces

### 5.1 MuxAPI (what an agent or tool can do)

Names are a sketch; the build settles them with the typed-RPC conventions.

| Action | Does | Notes |
|---|---|---|
| `Agents list` | every created agent with its runtime state | today it returns only agents with panes (`agent.list` iterates tabs -> blocks); it moves to definitions joined with the runtime state |
| `Agents start <agent>` | starts it headless (resumes its session), no pane | the leader's use; admission, spawn gate and the lease apply |
| `Message send <agent>` | delivers; if the agent is stopped and D4 allows, **start, then deliver** | replaces the `not_running` hold's "wait" with "start" in `agent_availability` |
| `Agents watch <agent>` | replay the recent transcript, then tail | a read of the same transcript a pane reads; for leaders that supervise a worker without opening it |
| `Agents stop <agent>` | stops the process, sets desired `stopped` | exists today as pane-close; becomes explicit |
| `Agents open <agent>` | attaches a pane (any tab) to the agent's block | no spawn if it is running; if stopped, starts it (as today) |
| `Agents detach <agent>` | removes the pane, keeps the agent running | what Close does |

### 5.2 The Swarm and My Agents

- **The Swarm lists every created agent.** Camper's finding: it lists blocks via `agent.tracked-blocks`, so only agents with a live controller appear. With one block per agent, a parked block still appears in that list; the change is to also list agents that have **no running controller** (stopped agents), as a second source of rows keyed by agent id, and to carry runtime state from a per-agent event, not from the block's controller status.
- Row actions: open (attach, live), message, start, stop.
- Selection, broadcast targets, Supervisor and fleet actions stay block-keyed for running agents and use the agent's block for stopped ones (it exists, parked). No re-keying to agent id is needed, because rule 1 makes the block id a stable per-agent handle.
- **My Agents** shows the runtime state of each agent and opens it the same way.
- The status bar shows how many agents are running and how many are waiting on you.

### 5.3 Opening an agent: replay then tail

Attaching a pane to a running agent's block is what a pane already does on mount: read history from `blockfile`, then follow the block's live events; the snapshot fast-path (`agent:<definitionId>:current`) restores overlay state. **Not yet verified: the seam between the history read and the subscription when a turn is streaming** (a line written between the two could be lost or shown twice). P3 includes a test for it; I will not promise the opening of a mid-turn agent is gap-free before that test passes.

### 5.4 Prompts with nobody watching

| Prompt | Today | With no view |
|---|---|---|
| Tool permission | default permission mode is bypass, so rare | if one is asked, `waiting on you` (D8) |
| AskUserQuestion | `awaiting_answer` in the transcript | `waiting on you`, answerable from the opened agent (or later, inline in the Swarm row) |
| Sign-in / limit | availability states, held messages | unchanged |
| Vault request, handoff, "Needs you" | surfaced by the pane / Kanban | surfaced by the Kanban and Swarm, not only the pane |
| ACP agents | auto-approve | unchanged |

## 6. Cross-cutting

| Subsystem | Effect | Change |
|---|---|---|
| Supervisor, seat, colour | block meta | none: block meta is agent meta (rule 1) |
| Kanban claims and leases | alive = leased | none; a headless agent claims and heartbeats like any other |
| Account switch / quota moves | iterate open blocks by `agentId` | works on a parked block; a **stopped** agent is relinked without a restart (check in P2) |
| Compaction, "Continuing in" | events in the transcript | none |
| Jobs / cron | deliver to a running agent | start the agent (D10) |
| Containers | one Docker container per agent, driven by the block's subprocess controller | works unchanged; D5's idle stop also stops the container (decide in P4) |
| LAN / WAN delivery | held while the agent is not running | unchanged for peers (D4); local starts follow D4 |
| Mobile viewer and feed | lists agents with state | shows `stopped` agents; QTest checks the states |
| `AgentRegistration.block_id` is required | a headless agent still has a block | **none**: this is why section 3.2 prefers the parked block over a block-less runtime |
| MCP calls made by a headless agent that pass its own block (Pane tools, `Agents spawn`) | `resolve_tab_id_for_block` errors "not found in any tab" for a parked block (`agent_spawn.rs:142`, `pane.rs` lookups) | fall back to a no-tab path; the leader's tab is then an explicit argument |
| Security | token is per uid; attribution is the actor | unchanged; the starter is recorded (rule 7). New danger: a message starting a paid session (D4) |

## 7. Lifecycle and cost

- **Idle stop** (D5) and **cap** (D6), with the numbers set from Loap-3's measurements (resident memory and commit charge per agent process on narko; pending).
- **Restart resume** (D7), staggered, with eager resume reusing `persistent/eager_resume.rs`, which today runs only when a frontend resyncs a block. srv runs the resync itself (all its arguments come from `AppState`).
- **Visibility of spend:** the status bar's running count, and the Swarm's state column, so closing a pane no longer being a stop is not a surprise bill.
- **Wake loops:** agent A messages stopped agent B, B replies to stopped A, and so on. The cap and the idle stop bound it; the start itself is rate-limited per sender (P2).

## 8. Migration

Today every pane with an agent already has exactly one block, so **an existing layout migrates by doing nothing**: each such block is that agent's block, shown. What changes is its behaviour when the pane closes (detach, not delete).

- Blocks of agents that were closed before the upgrade are gone, as today. The first open after the upgrade creates the agent's block and resumes the session the way `agent.open` does now.
- Two old blocks for one agent (possible before the guard): keep the one with a live controller, else the most recently active; leave the other as an ordinary pane until closed.
- The tab-close and workspace-delete sagas detach agent blocks instead of deleting them. Delete agent is the only delete.
- The orphan reaper (a block in a tab's `block_ids` but in no pane for 90 s is closed if it is an agent block) must not scan a parked block; a block in **no** tab's `block_ids` is not scanned, so park it there.

## 9. Phases

| Phase | What | Owner | Size |
|---|---|---|---|
| **P0** | Inventory and this spec | AgentX | done on approval |
| **P1** | The agent's block outlives its pane. Detach/attach/park; look up the agent's block by uid; close-pane, tab-close, workspace-delete and `close_on_exit` detach; reaper and `resolve_tab_id_for_block` fixes; one block per agent at creation; desired state; the setting (off). Tests: close keeps the process and transcript; reopening shows both; a second open focuses | Manoz (lifecycle, RPC), AgentA reviews the srv seam | L |
| **P2** | Headless start (`agent.start`), start-on-message (availability verdict `NotRunning` becomes "start, then deliver", D4), Jobs/cron start, the `Agents` start/stop/watch/detach actions, `waiting on you` state and per-agent runtime event, sender rate limit | Manoz (message side), AgentA (API, event) | L |
| **P3** | The Swarm lists every agent and open attaches with replay then tail; stopped/headless/needs-you states in the pane (Start button, a question to answer on attach); My Agents shows runtime state; the mid-turn attach test; setting on by default | Camper (Swarm), AgentX (agent pane) | L |
| **P4** | Lifecycle policy (idle stop, cap, restart resume staggered), status bar count, old-layout cleanup, mobile states, docs, setting removed | Manoz/AgentA (policy), Loap-3 (numbers, memory), QTest (mobile), AgentY's docs lane | M |

Build #5824 is split along these phases; each phase is its own PR.

## 10. Risks, and what I did not verify

**Risks**

1. **Cost.** Closing a pane no longer stops the agent. Mitigated by D5, D6, the status bar, and the setting.
2. **Memory on a laptop** (narko's commit charge is tight): the cap is the control; numbers pending.
3. **Mid-turn attach** may lose or repeat a line (5.3). Test before claiming.
4. **Everything that assumed a block is in a tab** (Manoz's list: `find_agent_block`, `resolve_tab_id_for_block` callers, the reaper). Found by grep, not by running.
5. **Surprise starts:** a message waking a paid agent. D4 limits who.
6. **Surprise stops on restart:** an agent that was running and is not resumed (D7) looks lost; the Swarm shows `stopped`, and the first message starts it.
7. **The "one block per agent" rule is new.** Anything that creates agent blocks (import, fork, restore, drag) must go through the lookup. Fork and import create new agents, so new blocks; restore of a saved layout attaches.

**Not verified**

- The history-read to subscription seam (5.3).
- Settled by reading: tab-close and workspace-delete do not go through the close-pane saga. `delete_tab` and `delete_workspace` run `agent_teardown::run_many(Policy::close())` over every block, then the reducer's `DeleteTab`, whose persist step (`wcore::delete_tab_inner`) deletes the block rows; `close_pane` runs the same teardown and then `DeleteBlock`; `close_on_exit` runs the `delete_block` saga. P1 parks agent blocks before the teardown in all four.
- Whether anything on the frontend renders from `tab.blockids` and would miss a parked block (Camper and I read the Swarm and pane code, not every consumer).
- The `agent_admission` capacity logic, if any, that the cap (D6) could reuse (asked of AgentA).
- That account switching of a **stopped** agent relinks without a restart (read, not run).
- Numbering: Kanban tasks #5417/#5419 (the duplicate-pane RCA and its placement-function audit, Loap) are not the same numbers as the merged PRs #5417 (mouse side buttons) and #5419 (the usage-limit hold). The RCA is AgentY's, uncommitted in AgentY's clone, and I have read only Loap's summary of it. The guard on `main` is the per-agent lease plus `agent.open` returning the existing pane in the same tab; Loap's #5580 (open) adds the early admission check and the "Open in another pane" row.

## 11. Gaps found along the way

- At boot `persist.rs` loads a block into the reducer's state only if a tab's `block_ids` holds it, and skips (logs) any other as an orphan. A parked block is in no tab, so it carries block meta `agent:parked` and boot loads it with an empty tab; resync after a dropped event keeps it parked. Without this a parked block would vanish from the reducer on the next srv start and every command on it would fail.

- The Swarm and `agent.list` enumerate agents through tabs and blocks, so any agent not in a layout is invisible to them.
- `AgentRegistration.block_id` has no "none" and `resolve_tab_id_for_block` has no tabless fallback.
- Held messages are keyed by agent uid, but nothing starts the agent they wait for.
