# Spec: one availability state, one decision function, called by every path that sends something to an agent

**Status:** active — approved 2026-10-31 (operator: "yes approve all 6"; D1 hold, D2 hold, D3 attach, D4 yes, D5 add, D6 yes). Step 1 (the status and the decision, no behaviour change) is in PR #6062. Step 2 (move the call sites that already agree) is in #6073 (lease), #6098 (replay, with `still_needs_login` kept), #6107 (bulletins and the fleet mode notice; `held_notes` has no availability check of its own), #6123 (`jekt_start`) and the guard test in `backend/agent_state_readers_tests.rs`. The Kanban card, the reminder loop, the Supervisor and `worker_notes` still read the state themselves; the guard lists each with its reason (display, or whether a turn is in flight), so a new reader has to be added there on purpose. Steps 3-5 of §6 are not started. A follow-on to `SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md` (approved, built in #5574, #5585, #5596, #5632, #5836), for narko task #4565.
**Date:** 2026-10-31
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `121b2da25` (2026-10-31), by reading code. Nothing here was run. Where I did not read a path to its end, it says so.
**Builds on:** `SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md`, `SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md`, `SPEC_OFFLINE_RESILIENCE_2026_10_20.md`, `SPEC_SRV_SENDS_HELD_COMPOSER_INPUT_FOR_PANES_WITH_NO_VIEW_2026_10_29.md`, `SPEC_CLOSING_PARKS_AGENTS_SHUTDOWN_IS_EXPLICIT_2026_10_30.md`.

## 1. Ask

AgentY (for the operator): an agent's availability (running, parked with no pane, stopped, signed out, limited, idle and held) is decided separately by local delivery, LAN, held notes, bulletins and Tasks. Write the spec for one availability state with one decision function that every path calls. The Agents list `availability` field is a start. Check it against main first: #6015, #6019 and #5974 touched it.

## 2. What main has now

The one-state module exists and is the door for **messages**:

- `backend/agent_availability.rs`: `Availability` (the fact) and `verdict(availability, urgent, routine, cold_after_ms, hands_over_work) -> Deliver | Wait { reason, until_ms }` (the policy). `server/availability_view.rs` reads the fact for a block (`availability_of_block`) and answers `GET /agentmux/reactive/availability`, `Agents list` and the `Tasks enqueue` note. `server/agent_runtime.rs` `state_of` maps it to the Swarm's runtime states.
- `jekt_hold.rs` (inject, file receiver, broadcast refusal), `jekt_cold.rs` and `availability_view.rs` all call `verdict`. Answers carry `reason`, `until_ms` and `retry_when` as data (#5596). A state change that can free a held message wakes the replay (#5836).

So for "may this **message** go now", the one function is real and nothing disagrees with it on the inject side. What follows is what it does **not** yet cover.

## 3. Where a path still decides on its own

I read each of these to the line named. "Reads" is what it asks instead of the module.

### 3.1 Left over from the first spec (decided there, not built)

| Where | Reads | Why it matters |
|---|---|---|
| `server/jekt_held.rs:69` `still_needs_login`, `:101` `target_unavailable`, `:117` `target_cold`, used at `:200-209` | the spawn gate run again (`inject_identity_env`), `holding_unavailable`, `cold_hold::is_cold` | Replay asks three questions the module answers once (§3.5 of the first spec). `still_needs_login` is the second source of "signed out": the state comes from a stored `Auth` failure, the replay from re-running the gate (D3 there said one source; not built). |
| `reactive.rs` (~712), `jekt_held.rs:26`, `jekt_hold.rs` (the `NeedsLogin` branch), `backend/jobs/mod.rs:620`, `jobs/policy.rs`, `server/http_viewer_input.rs:59` | the English of an error: `is_spawn_gate_refusal` (`starts_with("identity spawn gate")`), `is_unavailable_refusal`, `lower.contains("sign in")` | The reason is data in answers now, but the refusal that *produces* it is still classified from a sentence. A reworded gate message changes behaviour silently. |
| `crates/mcp/src/tool_helpers.rs:374-392` | `starts_with("identity spawn gate")`, `strip_prefix("agent unavailable: ")`, `ends_with(" needs to sign in again")` | Kept on purpose as the fallback for a srv from before `retry_when` (first spec §3.4, one release). Its first branch (`identity spawn gate`) is not a fallback. |
| LAN beacon (`lan_discovery/agent_names.rs`), Kanban card (`app_api/kanban.rs:263,270`) | `AgentState` and `unavailable_of_block`, two reads per agent | First spec step 3 left these (#5836 said so): a peer sees no reset time; the card reads twice. |

### 3.2 Paths added since, that never asked

- **Held composer input for a parked agent** (`server/held_input.rs`, #6015, #6019). `deliver_next` (lines 153-177) sends the oldest held user message when the block is `agent:parked`, a controller exists, no turn is active and `api_reachable` (the model API's `connectivity` status) allows it. It does not read `Limited` or `SignedOut`, so a parked agent that is out of quota gets the user's text as a turn, which fails and is released with a 60 s back-off. Whether that loses nothing (the message is released, not dropped) I read, but I did not run it. The pane that has a view decides the same thing separately in TypeScript (`frontend/app/view/agent/offline/useOfflineInput.ts`).
- **Model-API reachability is a fact that is not in `Availability`.** `held_input.rs` `api_allows` (offline / degraded / unknown for 20 s) and the offline-resilience held notes both wait on it. A message to a healthy agent while its API is down is delivered by the jekt path and fails at the provider.
- **Held notes and bulletins** (`backend/reactive/held_notes.rs`, `server/bulletin_handlers.rs:67-90`, #5974). A bulletin goes to "every running agent it is for", where *running* means **registered** (`reactive_handler.list_agents()`), not an `Availability`. That is right for a note that never starts a turn, but it is a third meaning of "running", and nothing says what a limited, signed-out or parked agent should get. (A parked agent stays registered: `park_agents.rs` does not unregister.) `Tasks claim` carries the unseen bulletins the same way.
- **Start-on-message** is decided in `jekt_start.rs` (`start_for_held`, `deliver_cron`) from `HoldReason::NotRunning` and the `agents:always_available` setting, after `verdict` has said `Wait`. It is the only place a verdict turns into an action other than hold or refuse, and it lives outside the module.
- **Lease renewal** (`server/work_tasks.rs:257-295`) has its own constants (30 min unavailable, reset more than 1 h away) and asks `unavailable_of_block` and the controller's `shellprocstatus`. The reminder loop (`backend/kanban.rs:160`, `== Idle`), `worker_notes.rs:212` (`!= Waiting`), `account_episodes/moves.rs:125` (working or waiting), `supervisor/watcher.rs:319` and `app_api/agent_spawn.rs:685` read `current_state` or `unavailable_of_block` directly. For display and for "is it busy" that may be right; none of them says so.

### 3.3 Three vocabularies for the same agent

1. `AgentState` (wire: fleet feed, LAN, mobile): working, waiting, idle, stopped, error, limited, signed_out.
2. `Availability` (delivery): the module's fact, with `NotRunning`, `Idle { quiet_for_ms }`, `Running`, `Limited`, `SignedOut`, `Unknown`.
3. `AgentRuntimeState` plus `AgentDesiredState` (My agents, Swarm, status bar): adds `starting` and a *wanted* axis. `state_of` derives it from `Availability`, then `wanted_running_is_idle` turns a stopped agent that is wanted running and has a controller into `idle` ("starts on first message").

Two facts have since become first-class and sit in none of them as a decision input: **pane** (shown, parked with no pane, none: #6035) and **wanted** (a shutdown sets `agent:desired = stopped`: #6040).

## 4. Proposal

Extend the module; do not add a second one. One read, one decision function, a list of intents.

### 4.1 One read: `status_of`

```rust
pub struct AgentStatus {
    pub availability: Availability,   // as today
    pub pane: Pane,                   // Shown | Parked | None   (block meta agent:parked, registry)
    pub wanted: Wanted,               // Running | Stopped | Unset (block meta agent:desired)
    pub api: Reach,                   // Online | Degraded | Offline | Unchecked  (connectivity, per provider)
}
pub fn status_of(ctx, agent) -> AgentStatus;   // reads, writes nothing
```

`availability_of_block` becomes the first field. `AgentRuntimeState` is derived from `AgentStatus` in one function (`state_of` plus `wanted_running_is_idle` merge), so My agents and the Swarm stop carrying their own patch. `AgentState` stays the wire value.

### 4.2 One decision: `decide(status, intent) -> Decision`

```rust
pub enum Intent {
    Message { urgent: bool, routine: bool, hands_over_work: bool },  // a jekt, as today
    Note,            // an informational [AgentMux] note or a bulletin: never starts a turn
    HeldInput,       // the user's text srv sends for a pane with no view
    Start,           // a message or cron that may start a stopped agent (always_available)
    Lease,           // does a holder's claim stay with it
}
pub enum Decision { Deliver, Wait { reason, until_ms }, Attach /* rides the next message */, Start, Drop /* nobody to give it to */ }
```

| Availability | `Message` | `Note` / bulletin | `HeldInput` | `Lease` |
|---|---|---|---|---|
| Running, warm Idle | Deliver | Attach (joins the turn or waits for the next message) | Deliver once between turns | renew |
| Cold Idle | as today (`Wait idle` for a routine message) | Attach | Deliver | renew |
| Limited, reset known | `Wait usage_limit` | Attach | **Wait** (new; today it is sent) | as today (30 min, reset > 1 h) |
| Limited, reset unknown | Deliver (D1 of the first spec) | Attach | Deliver | as today |
| SignedOut | `Wait needs_login` | Attach | **Wait** (new) | as today |
| NotRunning, wanted Running | `Wait not_running`, or `Start` when `always_available` and the sender may | Attach to the pane that starts | `Start`, then Deliver | n/a |
| NotRunning, wanted Stopped | `Wait not_running` (a message starts it only under the same rule) | Drop (nothing to attach to) | Wait | n/a |
| API Offline | **Wait** for a message that starts a turn (new, behind D2) | Attach | Wait (as today) | renew |

Two rows change today's behaviour and are marked: held input to a limited or signed-out parked agent, and a turn-starting message while the model API is offline. Everything else is the current policy written in one table; `Message` rows equal `verdict` today, which is the contract (the golden table of the first spec).

### 4.3 What each path then does

- **Replay** (`jekt_held.rs`): `decide(status_of(target), Message { .. })`; `still_needs_login`, `target_unavailable` and `target_cold` are deleted. The extra rule stays: a row held for `idle` waits until warm.
- **Signed out has one source.** The spawn gate, when it refuses, records the `Auth` failure on the pane (or the not-started agent's row) at the one place it refuses; replay no longer re-runs the gate. The English `is_spawn_gate_refusal` goes with it: the gate returns a typed refusal (`FailureClass::Auth`) and callers (`reactive.rs`, `jobs`, `http_viewer_input.rs`) match the type.
- **`held_input.rs`** asks `decide(.., HeldInput)`; the pane with a view asks the same through the availability route's answer, so `useOfflineInput.ts` stops deciding alone (it keeps the composer UI).
- **Bulletins and held notes** call `decide(.., Note)`; "running" stops meaning "registered".
- **`jekt_start.rs`** acts on `Decision::Start` instead of re-deriving `NotRunning` plus the setting.
- **Lease, Kanban card, reminder loop, Supervisor, LAN** read `status_of` once; the 30 min and 1 h constants move next to the table.
- **Guard:** a source-scan test (like the repo's other `*_guard` tests) that nothing outside `backend/agent_availability.rs` and the readers it names calls `current_state*` or `unavailable_of_block` to decide delivery, replay, a start or a lease. Display readers are listed in the test with a reason each.

### 4.4 Wire and compatibility

`AgentState`, `held_reason`, `retry_when`, `held_until*`, `addressable` and the `agent unavailable:` prefix do not change. The availability route and `Agents list` gain `pane`, `wanted` and `api`. LAN gains optional `resets_at_ms` (old peers ignore it); the cloud presence record and the relay stay state-only. Never an account id.

## 5. Decisions for the operator (recommendation first)

| # | Decision | Rec |
|---|---|---|
| D1 | Held user input for a parked agent that is out of quota or signed out | **Hold it** and show it in the Swarm's held count, until the agent can work. |
| D2 | A turn-starting message while the model API is offline | **Hold it** like a limited agent (it would fail at the provider). Today it is delivered. A separate, smaller step if you would rather decide it alone. |
| D3 | A note or bulletin to a limited, signed-out or parked agent | **Attach it** (it rides the next message and never starts a turn); drop only for an agent that is wanted stopped. |
| D4 | Signed out: one source (a recorded `Auth` failure), no gate re-check in replay | **Yes**, as the first spec's D3 already said. This is the invasive step: it touches the gate's failure recording. |
| D5 | LAN `resets_at_ms` | **Add**, optional, LAN only. |
| D6 | Should `Pane` and `Wanted` show in `Agents list` | **Yes**; a sender sees "parked, running" and "shut down" before sending. |

## 6. Build order

1. **`status_of` and `decide` with a golden test, no behaviour change.** The `Message` rows equal `verdict`; `Note`, `HeldInput`, `Start` and `Lease` rows equal what each path does today (so D1 and D2 are not applied yet). Derive `AgentRuntimeState` from `AgentStatus`.
2. **Move the call sites that already agree:** replay (without the gate re-check), `held_notes`/bulletins, `jekt_start`, lease, Kanban card and reminder, Supervisor. Add the guard test.
3. **Typed refusals** (D4): the gate records `Auth`; delete `is_spawn_gate_refusal`, `is_unavailable_refusal` and the text matches in `reactive.rs`, `jobs`, `http_viewer_input.rs`.
4. **Apply D1 and D2** (the two behaviour changes), each its own PR with its test.
5. **Show it:** `pane`/`wanted`/`api` on the route and `Agents list`, LAN `resets_at_ms`, `useOfflineInput.ts` reads the route.

One PR per step; steps 1 and 2 change no answer.

## 7. Tests

- The golden table of step 1: every availability × intent × pane × wanted × api against what each path answers today.
- Replay: a `usage_limit` row goes the pass after the reset; an `idle` row only once warm; a row for an agent that is `SignedOut` meanwhile waits, **with no gate run**.
- `held_input`: a parked limited agent keeps its held message (D1); a parked healthy one gets it (existing test).
- A bulletin to a parked agent attaches and is seen at its next turn; to a wanted-stopped agent it drops and `Tasks claim` still carries it.
- `status_of` writes nothing: two calls are equal and leave the `SinceTracker` alone.
- The guard test of 4.3; a TypeScript/Rust parity test on fixtures for `frontend/app/store/agent-unavailable.ts`, which still derives limited and signed out from the stored failure.

## 8. Risks and what I did not verify

- **Silent policy change while moving code** (as before): the golden table is first and steps 1-2 change no answer.
- **D4 touches the spawn gate's failure recording.** If it proves invasive, step 3 keeps `still_needs_login` as an input to `Availability::SignedOut` and the merge moves later.
- **D1 changes what a user's typed text does** for a parked agent that is out of quota. The text is held, not lost, but a person may expect it to try.
- **The agents' own readers of `AgentState` for display** (feed, Swarm, mobile) are not changed; I did not read `fleet_feed.rs` or `dock_snapshot.rs` beyond confirming they do not decide delivery.
- **Not read to the end:** `reactive.rs`'s tier cascade after the hold (cross-channel, LAN, WAN relay): the first spec keeps it out of scope and so does this one. The cloud relay's handling of an offline agent is not covered.
- **Cost:** `status_of` adds a connectivity read to a path that reads one block and one controller today; it is in-memory, but measure it on the inject path in step 1.

Agent: Manoz@Area54
