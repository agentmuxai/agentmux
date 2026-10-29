# Spec: two buttons on every countdown, and agents stopped, opened and spawned as a batch

**Status:** active: approved by the operator on 2026-10-28 ("yes, good for all 4"). S1 (part 1) is built in #5733, S2 (part 2) in #5735 and S3 (part 3) in #5749; S4 is not.
**Date:** 2026-10-28
**Author:** Manoz@Area54 (agent-pane lifecycle), for AgentY, from the operator's own asks of 2026-10-28.
**Verified against:** `origin/main` @ `bd2aa677f`, by reading code. Not checked: the macOS toast (the launcher has no macOS notify module yet; it is #1267) and a run of any of this.
**Builds on:** `SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md` (the Stop tool and the shutdown toast), `SPEC_SUPER_AGENT_2026_10_25.md` §5 (sensitive operations and their countdown), `SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` (the 12-agent cap).

## 1. What was asked

1. A countdown toast gets two buttons at the bottom. Closing an agent: **Keep running** and **Close now**. Any other countdown action: a cancel and a do-it-now worded for that action. Both the OS toast and the in-app one.
2. Closing many agents at once is one action: one toast ("Closing 5 agents: A, B, C, ..."), one window, one audit entry, with Keep running / Close now and a way to keep just one.
3. Spawning and opening many agents at once, with one approval prompt where one is needed, and a clear answer to the 12-agent cap: refuse up front and say which would fit, never half-do it.

## 2. What exists today

- **An agent's close** (`sagas/pending_shutdown.rs`): `request` opens a 30 s window (15 s for an agent's own quit), tells the user with the pane banner (`ShutdownPendingBanner.tsx`, **Keep running** through `agentshutdownkeep`) and an OS toast (Windows and Linux in the launcher's `notify/`, one **Keep running** or **Keep open** button, `keep_for` in `server/notify_handlers.rs`). There is no way to end the wait early. `expire_now` does it, but only in tests.
- **A sensitive operation's countdown** (`sagas/sensitive_window.rs`, 12 ops in `backend/sensitive_ops.rs`): the pane banner (`SensitiveBanner.tsx`) has **Veto** and **Approve now** (`sensitive.decide`, window-only through `RpcContext::operator_only`). The OS toast has **no button**: any client holding the instance key can send `notify.ack`, so a press is not proof of the person.
- **`Stop agents`** (`fleet_close_agents_with_override` in `app_api/fleet.rs`) takes a list but calls `pending_shutdown::request` once per target: a window, a toast and an audit row each.
- **`Agents spawn`** (`app_api/agent_spawn.rs`) makes one worker per call: at most `MAX_LIVE_WORKERS` = 3 live per leader, one `consent` prompt per call unless the user's own turn asked for it. **`Agents open`** (`app_api/agent_open.rs`) opens one agent per call and asks nothing.
- **The cap** (`server/agent_lifecycle.rs`, #5699): a start over `agents:max_running` (12) or the commit limit stops the idle agent with no view that has been idle longest, or is refused with `capacity: ...`. A start also counts against the sender's starts per minute (`start_rate`).

## 3. Part 1: two buttons on every countdown

**Behaviour.** Each countdown shows a cancel and a do-it-now. Do-it-now changes **when**, never **what**: the action runs at once through the same path it would have run at the deadline, and the audit row says the person chose it.

| Countdown | Cancel | Do it now |
|---|---|---|
| Close a tab or a pane (a `Stop agents`, `Stop pane`, an agent's own quit) | Keep running (Keep open when it is not running) | Close now |
| Stop only (internal) | Keep running | Stop now |
| `upgrade` | Don't upgrade | Upgrade now |
| `quit_agentmux` | Keep open | Close now |
| `widget_install`, `job_template_install` | Don't install | Install now |
| `job_template_approve` | Don't approve | Approve now |
| `desktop_control_grant`, `vault_use`, `vault_always_allow` | Don't allow | Allow now |
| `sender_key_repin` | Don't trust | Trust now |
| `tower_pair` | Don't pair | Pair now |

The sensitive pairs are two new fields on `OpSpec` (`cancel_label`, `now_label`), so a new operation cannot be added without its words. The in-app banner stops saying "Veto" and "Approve now" and uses them.

**Trust.** Skipping the wait removes the person's own protection, so a forged press must not be able to do it. Keep is the safe direction and stays as it is.

- *In the app:* a new `agentshutdownnow` RPC beside `agentshutdownkeep`, and the existing `sensitive.decide`, both `operator_only` (the proven window).
- *On the OS toast:* the launcher presents a proof with each button press. A per-toast token cannot work: srv publishes a notification as a broadcast event, and every agent holds the instance key and can read it. What the launcher has that no agent has is the **host registration secret** it makes for srv at start (`SrvSpawnResult::host_reg_secret`): srv takes it from its environment, so it never reaches an agent's, and the launcher holds it in the same process that shows the toast. The launcher sends it as `proof` in `notify.ack` on a button press; srv compares it in constant time (`host_caller_allowed`). A **Close now** press, and any press on a sensitive operation's toast (the veto and the approve-now), without it is `refused` and the countdown goes on. **Keep running** on an agent's close stays open to any caller, as today, because it only leaves things as they are. That also gives the sensitive toast its buttons. Windows and Linux; macOS follows #1267.

**Touch.**
- srv: `backend/notify/policy.rs` (`NotifyAction` becomes up to two, built from the effect or the op; `ShutdownInfo` carries the labels), `server/notify_handlers.rs` (`notify.ack` handles `now`; `keep_for` becomes a decision by id), `sagas/pending_shutdown.rs` (a production `close_now`, the sibling of `keep`), `sagas/sensitive_window.rs` (toast routes to `veto` and `approve_now`), `backend/sensitive_ops.rs` (the two labels), a new RPC in `rpc_types/commands.rs` and `server/websocket.rs`.
- Launcher: `crates/launcher/src/notify/{mod,windows,linux}.rs` (two action ids in place of the Keep-only `keep_label()`, the proof sent with a press), and the four places `supervisor/` starts or restarts the presenter, which now pass `host_reg_secret`.
- Frontend: `view/agent/shutdown/ShutdownPendingBanner.tsx`, `view/agent/sensitive/SensitiveBanner.tsx`, regenerated `store/rpc-api/generated.ts`.
- MCP: the `Stop` tool text ("a Keep running button" becomes "Keep running and Close now buttons") and its test needle in `tools/stop_tool.rs`.

## 4. Part 2: one window for many agents

A `Stop agents` call is one request over all its local targets.

- **`pending_shutdown`:** a new action `CloseTabs { block_ids, detail }` (the model `ClosePane { block_ids }` already has: `covers` returns every block). One request id, one deadline, one `PendingView` with a `blocks` list (block id and agent name) and, when it settles, an `outcomes` list per block (`shut_down`, `kept_by_user`, `superseded`, `failed`). `Stop agents` answers with the same request id on each target's entry, so a caller that polls per target still works.
- **Toast and banner:** one OS toast, "Closing 5 agents: A, B, C and 2 more in 30 s", with **Keep running** (all) and **Close now** (all). Each target's own pane keeps its banner and its **Keep running** keeps **that** agent only (`keep_one`: the block leaves `covers`; the others and the clock are untouched; when none is left the request is `kept_by_user`). This is the "keep just one". The toast does not list a button per agent.
- **Audit:** one `FleetBulkStop` entry with the target list (`audit_note` names them), and the per-block outcomes added when it settles. Today there is one row per target.
- **Built as:** `pending_shutdown/batch.rs` (`request_batch`, `Action::CloseTabs`). The toast is the request's own group (`batch:<id>`), not one agent's block, so its Keep running (all) and Close now (all) name the request. Only the blocks no request covers yet go in the new one: a block that already has a window joins it as before, and with fewer than two free blocks `Stop agents` opens the ordinary request per block. A batch is never upgraded by a pane close (a pane that holds an agent is refused). The MCP `Stop agents` reads each target's answer from its own entry in `outcomes`.
- **Not changed:** a target on another AgentMux on this machine is asked there (`ask_other_instances`) and has that instance's own window and toast. A batch is per instance. A block that already has an active request joins it as today.

## 5. Part 3: spawn and open many

**Shapes.**
- `Agents spawn` with `workers: [{ name?, model?, effort?, account?, brief }, ...]` (a single worker stays as it is). `Agents open` with `agents: [{ agent, tab_id?, focus? }, ...]` (a single `agent` stays). New srv routes `/api/v1/agent/spawn_many` and `/api/v1/agent/open_many`; the MCP process sends the briefs after srv has launched **all** the workers, so a failed batch has sent nothing and can be rolled back.
- **One approval.** `consent` is asked once for a spawn batch: one question listing every worker (name, model, account, the first line of its brief), "Start them" or "Don't". The user's-own-turn shortcut (`self_quit::gate`) applies to the batch as it does to one. `Agents open` asks nothing today and asks nothing in a batch.
- **All or nothing.** Everything that can be checked is checked before the first agent starts and before the prompt: briefs, names, accounts (`choose_account`), `MAX_LIVE_WORKERS` (3 per leader) against the number asked for, the cap. If a launch still fails, the ones already launched are discarded (`discard_worker`, which exists for one) and the call returns the error. A `spawn` batch is serialised on the leader's lock, as one is.

**The cap.** Agents already running or shown do not use capacity; only the ones that must start do.
- A batch **never stops another agent to make room** (like restart resume). One start may stop an idle agent with no view; a batch of five doing that would be a surprising mass stop.
- Under one hold of the admission lock it reads the room (`agents:max_running` minus running and reserved, and the commit limit) and **reserves** all of it, so a concurrent start cannot take it between the check and the starts. A refusal reserves nothing.
- If the room is short, the whole call is refused: `capacity: 12 agents are running (the limit is 12); room for 2 of the 5 asked: A and B would fit, C, D and E would not. Stop some agents (3 are idle with no view), or ask for fewer.` Nothing starts.
- Neither batch charges the sender's `start_rate` up front: as for one agent, a start that a message triggers is charged when the message is delivered, so charging here too would count a started agent twice.

**Built as:** `app_api/agent_spawn/many.rs` (`spawn_many`: validate everything, take the leader lock, count against `MAX_LIVE_WORKERS`, reserve room with `admit_batch`, one consent, launch each and roll back with `discard_worker`, `release_batch`) and `app_api/agent_open_many.rs` (`open_many`: at most 16, each agent by id, slug or name, a duplicate refused, only agents not already running count as starts, panes it created are closed again if a later one fails). `admit()` lets an agent that holds a reservation through. The MCP `spawn` with `workers` sends the briefs only after srv answers that every worker is open.

**Touch.** `app_api/agent_spawn.rs` (`spawn_many`, one `consent`), `app_api/agent_open.rs` and `agent_attach.rs` (`open_many`, reusing the per-agent lock), `server/agent_lifecycle.rs` (`admit_batch`: room, reservation, the refusal text), `server/start_rate.rs` (take N), `crates/common/src/api_types.rs` (the request types), `crates/mcp/src/tools/agents_tool.rs` and `tool_schemas.rs` (the two list parameters, the briefs sent after the launch).

## 6. Decisions I need from the operator

| | Question | Recommendation |
|---|---|---|
| **D1** | The OS toast's Close now and a sensitive operation's buttons need the launcher's proof (§3). The alternative is no decisive button on the toast: Keep stays, and the toast just opens the window. | **The proof.** The toast is where the person is when the countdown starts; a button that only opens a window is one more click. (Approved 2026-10-28 as "a press token"; built as the launcher's proof because a token in a broadcast event proves nothing, see §3.) |
| **D2** | A batch never stops idle agents to make room (§5). The alternative treats idle agents with no view as room, as one start does. | **Never stop in a batch.** The refusal tells the caller how many are idle, so stopping them first is one call. |
| **D3** | "Keep just one" is the target's own pane banner, not a button per agent on the toast (§4). | **Pane banner.** A toast with five buttons does not fit; the person who wants one agent is looking at it. |
| **D4** | The wording table in §3. | **As written;** the words are one field per operation, so changing any is one line. |

## 7. Phases

| Phase | What | Size |
|---|---|---|
| **S1** | Part 1: the pair on the pane banners and the OS toast, the press token, the labels | M |
| **S2** | Part 2: the batch window, `keep_one`, the one audit entry | M |
| **S3** | Part 3: `spawn_many`, `open_many`, `admit_batch` | M |
| **S4** | Part 4: a seat's lead may be an agent on another machine | S |

Each phase is its own PR with its tests: S1 the labels per op, a forged press refused, Close now runs the action once; S2 five targets one window and one audit row, `keep_one` leaves four, a joined block; S3 a short room refuses whole and reserves nothing, a failed launch discards the earlier ones, one prompt for N workers.

## 8. Part 4: a lead on another machine

Added 2026-10-28 at the operator's word ("keep the leader on external machine possible"). Today a seat's lead must be an agent with a definition on this machine: `Agents set_seat` resolves a lead by slug with `uid_of` (`app_api/agent_seats.rs`: "no agent 'agenty' on this machine") and `agent_seat::validate` refuses a uid that `is_agent` does not know ("lead ... is not an agent on this machine"). So the stewards on Area54, charlie and starpower cannot name AgentY or AgentX on narko.

- **Shape.** The seat stays `{role, qualifier, lead, area}`; a remote lead is the string `slug@host` (a minted uid never holds an `@`). `lead: "agenty@narko"` is accepted by `set_seat` and the window's `agent.setSeat` when the LAN directory lists that agent on that machine (`lan_discovery.get_instances()`: each peer's agent names with its hostname). Only the LAN list applies: a WAN agent name carries no machine name, so it cannot be written `slug@host`. A name no peer reports is refused, and the refusal lists the `slug@host` pairs that are known. A local slug or uid works as before.
- **Checks.** `validate` gets a second predicate, `is_remote_lead`. A remote lead cannot be a cycle: its own leads live on its machine, so `no_cycle` stops at it. An agent cannot lead itself still holds.
- **Shown.** `SeatLead` already allows `name: null` for an agent this machine does not know (`summarize`'s `name_of`); for a remote lead `name_of` answers "AgentY@narko" (the peer's agent name and machine name), so `Agents list` and the Swarm show it, never "no agent". When the peer is offline the stored `slug@host` is shown as it is.
- **Not changed.** Nothing is sent to the lead's machine: the seat is this machine's record of who the agent reports to, as for an operator lead. Seats a peer sends (`sanitize_peer_summary`) keep their bounds.
- **Touch.** `backend/agent_seat.rs` (`validate`, `summarize`), `server/app_api/agent_seats.rs` and `server/agent_handlers/seat.rs` (resolve a lead that is `slug@host` against the peer lists), the `set_seat` text in `crates/mcp/src/tool_schemas.rs`, a test for each of: accepted when known, refused when unknown with the known list, no cycle through it, shown as "AgentY@narko".

## 9. Not covered

The macOS toast (#1267). Closing agents on other machines (a different instance's window decides). Raising `MAX_LIVE_WORKERS` or the cap.
