# Spec: one availability state for "can this agent take a message now"

**Status:** active — approved by the operator ("go with best recommendations", relayed by the platform lead on 2026-10-27), with D1, D3, D4 as recommended and D2 = yes, so build steps 3 and 4 follow steps 1 and 2. Kanban #4565. The probe of §3.2 and the stored failure's time (§2.1 item 9) shipped first, in #5574; build step 1 (the `agent_availability` module and its golden tests, §6) is in #5585, and build step 2 (the reason as data in refused and held answers) is in #5596.
**Date:** 2026-10-26
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `08565342f` (2026-10-27), by reading code. The probe and the failure's time were built and tested against `8a907cd83`; the rest is not built.
**Builds on:** `SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §3.5 (one read helper) and §5 (held messages), `SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md`, `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md`. Code from #5389, #5419, #5429, #5453, #5520.
**Not the same as:** `TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`, which is the pane's composer gate (may the *user* type now). This spec is about messages from *other senders*.

## 1. Ask

The operator's idea, relayed by AgentY: make "can this agent take a message now" one thing that every sender and receiver path reads, instead of several places that each work it out, and make the jekt path the door for it.

The states AgentY listed: running, not running, idle, limited with a known reset, limited with an unknown reset, signed out.

## 2. What exists today

`agent_state::decide` already gives each pane a state, and `unavailable_of_block` already gives the limited and signed-out cases one reader (step 1 of the unresponsive-agents spec). The delivery code that grew on top of it did not stay on that one reader. Who decides what, on `origin/main`:

| Where | Question it answers | Reads |
|---|---|---|
| `server/jekt_hold.rs` `holding_unavailable` | Hold or refuse a jekt for a limited or signed-out agent | `unavailable_of_block`, then a **private filter**: signed out, or limited *with* a reset |
| `server/jekt_hold.rs` `hold_before_delivery`, `server/jekt_files.rs:422` | The same, at inject and at the file receiver | `holding_unavailable`, then `jekt_cold::hold_if_cold` |
| `server/reactive.rs` (tier cascade, `route_for_absent`) | Is the agent absent here, so hold it `not_running` | registry miss (`lookup_delivery_target`), `defined_here` |
| `server/reactive.rs:742`, `jekt_held.rs` `is_spawn_gate_refusal`, `still_needs_login` | Is the agent signed out | the **spawn gate's error text** (a string match), and a gate re-check on replay |
| `server/jekt_held.rs` replay (`target_unavailable`, `target_cold`) | May a held row go out now | `holding_unavailable` again; `cold_hold` read directly from the controller |
| `backend/cold_hold.rs` | Is a Claude session cold | `ctrl.last_output_ms()`, outside `AgentState` |
| `server/app_api/fleet.rs` `deliver_broadcast_turn` | Refuse an operator broadcast | `holding_unavailable` |
| `crates/mcp/src/tool_helpers.rs` | What the sender is told, and what to advise | **English in `error`**: `strip_prefix("agent unavailable: ")`, `ends_with(" needs to sign in again")`, `starts_with("identity spawn gate")` |
| `backend/jobs/mod.rs:617`, `jobs/policy.rs:130`, `jekt_hold::peer_refused` | Is this refusal recoverable, so retry | `is_spawn_gate_refusal` / `is_unavailable_refusal`: string prefixes |
| `server/http_health.rs` `handle_discovery` | `addressable` | registry presence only |
| `server/app_api/kanban.rs:232-245` | `holder_unavailable` on a card | `current_state`, then `unavailable_of_block` (two controller reads per agent) |
| `server/work_tasks.rs` `renews_lease` | Does a limited holder keep its lease | `unavailable_of_block` plus two constants of its own (30 min, 1 h) |
| `backend/kanban.rs:160` | Is the agent idle, so remind it | `current_state == Idle` |
| `backend/supervisor/watcher.rs:250` | Pause the Supervisor | `unavailable_of_block(...).reason()` |
| `lan_discovery/agent_names.rs`, `fleet_source.rs`, `swarm_remote.rs` | What peers and mobile show | `agent_status_of_block`; carries `{state, since_ms}` and **not the reset** |

### 2.1 Where they disagree or hide a policy

1. **Three meanings of "not running".** No registration here (hold `not_running`, and discovery says `addressable: false`); a controller whose process ended (`AgentState::Stopped` or `Error`); and a spawn-gate refusal that is reported as `needs_login`. A reader has to know which one it is looking at.
2. **"Signed out" has two sources.** The state comes from a stored `Auth` failure; the hold comes also from the gate's refusal text. They are one condition to the sender, and the replay must run the gate again (`still_needs_login`) because the state alone doesn't cover the second.
3. **"Limited, reset unknown" is a policy buried in a filter.** `holding_unavailable` delivers it ("a turn discovers recovery"); the Supervisor pauses for it; the lease logic treats it as far away; the Kanban shows "Out of quota". Each is defensible, none is written down together.
4. **"Idle" means two things.** `AgentState::Idle` is "alive, no turn in flight". The held reason `idle` is "a Claude session quiet for 55 minutes, message routine". The second is read from the controller, not from the state, and has its own module.
5. **The sender's advice comes from parsing English.** The MCP tool picks "after they sign in" or "when the agent can work" by what the sentence ends with. A rewording in `jekt_hold.rs` silently changes it.
6. **A sender can't see it before sending.** `addressable: true` for an agent that is out of quota until Sunday; the state in the feed or LAN beacon has no reset time.
7. **Four places must remember to wake replay** when something frees held mail (`held_jekt_wake().notify_one()` on registration, after failures are cleared, after an account move, and in the limit wake). A fifth state change that forgets leaves mail waiting for the 30 s sweep.
8. **Reads write.** `unavailable_of_block` records into the shared `SinceTracker`; harmless today, but it makes "just look" not pure.
9. **A restart moves "since".** The stored `agent:last_failure` had no time of its own, so after a restart a limited agent's `since_ms` was the moment srv first saw it, and the Kanban and Swarm showed that. It now carries `atMs` (kept when the same failure is hit again). The frontend (`frontend/app/store/agent-unavailable.ts`) also re-derives limited and signed out from the same stored failure, a second reader of the rule in TypeScript.
10. **A real limit can strand a message.** With the agent limited until a known reset, every message is held, urgent ones included, and nothing but a turn, the reset or an account move ends it; if the account was changed out of band, nobody learns it. Built in #5574, see §3.2.

## 3. Proposal

Two layers, both in one module: a **fact** about the agent, and a **verdict** about one message to it. Policy lives only in the second.

### 3.1 The fact: `Availability`

```rust
// backend/agent_availability.rs
pub enum Availability {
    /// Defined here, no live process: never started, ended, or its pane closed.
    NotRunning { exited_with_error: bool },
    /// Alive, no turn in flight. `quiet_for_ms` since its last output, when the controller knows it.
    Idle { quiet_for_ms: Option<u64> },
    /// A turn is in flight (working, or waiting on a question).
    Running { waiting_on_user: bool },
    Limited { resets_at_ms: Option<u64>, since_ms: u64 },
    SignedOut { since_ms: u64 },
    /// Nothing tracks its turns (a terminal pane): the old behaviour, the tiers decide.
    Unknown,
}
pub fn availability_of(ctx: &Ctx, agent: AgentRef) -> Availability;   // one read, no writes
```

- It is computed from what `agent_state::decide` already reads (controller status, `agent:last_failure`, `term:awaiting_user`) plus the registry (for `NotRunning`) and `last_output_ms` (for `quiet_for_ms`). **`AgentState` stays** as the wire value for the fleet feed, LAN and mobile; `Availability` is the delivery-relevant projection, so the wire does not change.
- **Signed out has one source.** A spawn-gate refusal (the agent can't start) records an `Auth` failure on the pane (or a row for the not-started agent), so "signed out" is the fact, and the English `is_spawn_gate_refusal` match survives only at the one place that sets it. Open point D3.
- Limited and signed out carry `since_ms` from the tracker; the tracker is written by the observers that already exist (the fleet source), not by readers.
- Never the account id. `Unavailable::account_id` stays out of anything another host can receive.

### 3.2 The verdict: `verdict(availability, message) -> Verdict`

```rust
pub enum Verdict { Deliver, Wait { reason: HoldReason, until_ms: Option<u64> } }
pub struct MessageKind { urgent: bool, routine: bool, hands_over_work: bool }   // already computed by cold_hold / jekt_cold
```

| Availability | Verdict | Same as today |
|---|---|---|
| `Running` | Deliver (steers or queues) | yes |
| `Idle`, warm | Deliver | yes |
| `Idle`, cold (Claude, quiet > `jekt:cold_hold_minutes`), message routine, not urgent, hands over no work | `Wait { idle }` | yes (`jekt_cold::decide`) |
| `Idle`, cold, otherwise | Deliver | yes |
| `NotRunning` | `Wait { not_running }`, **asked after** the tier cascade found no live candidate | yes |
| `Limited { Some(reset) }` | `Wait { usage_limit, until }` | yes |
| `Limited { None }` | Deliver | yes — see D1 |
| `SignedOut` | `Wait { needs_login }` | yes |
| `Unknown` | Deliver | yes |

`urgent` wakes a cold idle session; it does not deliver into an out-of-quota or signed-out agent, which would only fail the turn and lose the message. Instead, a message that cannot wait (urgent, or naming a task that waits on the agent) to an agent limited until a known reset is held as above **and starts one probe turn**: a small system note on the pane asking for a one-word reply, at most once per 15 minutes per agent (`server/limit_probe.rs`). A clean probe turn clears the stored failure and wakes replay, so the held message goes out at once; a probe that fails leaves the agent limited and the message held. The message never rides the probe.

What a `Wait` becomes is the **caller's** rule, in one place (`jekt_hold`): *held* where `hold_applies` (a full-key host-tier sender, not a cron fire, with an id), *refused* with `agent unavailable:` elsewhere (LAN and cross-channel hop, file transfer, cron), and for an operator broadcast always *refused in its result*, as built in step 2c. That is a map from `Wait`, not new policy.

The tier cascade (cross-channel, LAN, WAN relay, `route_for_absent`) is out of scope: it keeps deciding where an absent agent's message goes, and consults `verdict` where it consults the hold today.

### 3.3 The jekt path is the door ("commandeer")

- **Everything that applies a verdict goes through `server/jekt_hold.rs`:** inject, the file receiver, replay, broadcast refusal, job refusals. `holding_unavailable`, `local_target_unavailable` and `cold_hold`'s stand-alone check become thin callers of `availability_of` + `verdict`, then are deleted.
- **Everything that only wants to *know* asks the same two functions, not the controllers:** the Kanban list and card, `Agents list` and `workers`, `renews_lease`, the Supervisor, the reminder loop (`Idle { .. }`), discovery.
- **MCP is a separate process,** so it reads a route that wraps the same calls: `GET /agentmux/reactive/availability?agent=<name|uid>` (same auth key as `/agentmux/discovery`), answering `{ state, since_ms, resets_at_ms, would: "deliver" | "hold" | "refuse", reason }` for a routine message from a host-tier sender, and never an account. It is a separate route and not a dry run of `inject` on purpose: `inject` reserves a replay id, spends the rate limit (10 per second) and writes the audit log.
- Cross-hop stays as it is: the instance that has the agent is the authority, and a peer's refusal is returned as is (`peer_refused`). It is read from the answer's `reason` field (3.4), not its prefix.

### 3.4 The answer carries the reason as data

A held or refused answer gains `reason` (`not_running` | `needs_login` | `usage_limit` | `idle`; the existing `held_reason` strings), `until_ms` and `retry_when` (`sign_in` | `reset_or_account_move` | `session_active` | `start`). The English `error` stays for readers that show it. Then:

- `tool_helpers.rs` chooses its advice from `retry_when`, and the three string matches go;
- `jobs` decide "recoverable, retry" from `reason`, and `is_unavailable_refusal` / `is_spawn_gate_refusal` go;
- `peer_refused` reads `reason`.

An older MCP or peer that doesn't know `reason` keeps working from `error`. A newer MCP against an older srv falls back to the string match for one release.

### 3.5 Replay uses the same verdict

A held row is ready when `verdict(availability_of(target), row's kind)` is `Deliver`, with the one extra rule that a row held for `idle` waits until the session is warm (`quiet_for_ms` below the limit). `still_needs_login`'s gate re-check stays only as the source of the `Auth` failure (3.1), and `target_unavailable` / `target_cold` are deleted.

### 3.6 One wake

The `SinceTracker` already notices when a pane's state changes. A change **out of** `Limited`, `SignedOut` or `NotRunning` (and a cold session warming) notifies `held_jekt_wake()` from that one place. The four existing notifiers can stay (they are cheap), but a state change that forgets is covered.

## 4. Consumers after the change

| Consumer | Reads | Visible change |
|---|---|---|
| `Message send` answer | `reason`, `until_ms`, `retry_when` | none, apart from robustness |
| `Agents list`, discovery | `availability` per agent + `would` | a sender sees "out of quota until Sun 7 PM, messages are held" before sending; `addressable` keeps its meaning (registered) |
| Kanban list, card | `availability_of` once per agent | none; one read per agent instead of two |
| `Tasks enqueue` with `target_agent` | `would` | the answer says the target is out of quota or signed out and the task will wait; the follow-up "one line naming the task" is held, not lost |
| Lease renewal | `Limited` / `SignedOut` + `since_ms` | none; the 30 min and 1 h constants move next to the verdict table |
| Supervisor pause, reminder loop | `Limited`, `SignedOut`, `Idle` | none |
| LAN beacon and fleet feed | `AgentState` as now, plus optional `resets_at_ms` | peers show "until Sun 7 PM" for a remote agent; old peers ignore the field |

## 5. Compatibility

`AgentState` and its wire names, `held_reason` values, `held_until_ms` / `held_until`, the `agent unavailable:` prefix and `host.addressable` are unchanged. The additions are fields. No setting is added; `jekt:cold_hold_minutes` keeps its meaning.

## 6. Build order

1. **The module and a golden test, no behaviour change.** `agent_availability.rs` with `Availability`, `availability_of`, `verdict`; a table test that, for every availability × message kind × hop, equals what `holding_unavailable`, `jekt_cold::decide` and `renews_lease` answer today. `holding_unavailable` becomes a caller. *Owner: whoever the platform lead names; I can take 1 and 2.*
2. **Move the call sites and the reason field** (3.3 first half, 3.4, 3.5): `jekt_hold`, `jekt_files`, replay, broadcast, jobs, MCP. Delete the string matches and the private filters.
3. **Show it** (3.3 second half, the route, `Agents list`, Kanban, enqueue note, LAN `resets_at_ms`).
4. **One wake** (3.6).

Each PR cites this spec; steps 3 and 4 follow steps 1 and 2 (D2 is decided).

## 7. Tests

- The golden table of 1. is the contract: it fails if a consumer's answer for any availability × kind differs from the table.
- `reason` and `retry_when` round-trip through srv's answer, the MCP text, `jobs`' retry decision and `peer_refused`; an answer with only the old `error` still gives the old behaviour.
- `availability_of` writes nothing: two calls in a row give equal values and do not touch the tracker.
- Replay: a `usage_limit` row is delivered the pass after the reset passes; an `idle` row only once warm; a row for an agent that went `SignedOut` meanwhile waits.
- A guard that nothing outside the module calls `current_state_at` / `unavailable_of_block` for a delivery decision (a grep test in the module's test, like the repo's other source checks).
- The route returns no account id for any state.

## 8. Risks

- **Silent policy change while moving code.** The golden test (1) is first for this reason, and steps 1 and 2 change no verdict.
- **Latency on the inject path.** `availability_of` does what the paths do now (one block read, one controller read; the registry lookup is in memory), once instead of up to three times. Measure in step 2 with the existing inject timing.
- **The reset on LAN.** It tells a peer when an agent will be back, which is no more than the status already shows, and no account id goes with it. The cloud presence record and the relay stay as they are (state only); decide with the platform lead before adding it there.
- **Two sources for "signed out" merging into one** (3.1) touches the spawn gate's failure recording. If that proves invasive, step 2 keeps `still_needs_login` as an input to `Availability::SignedOut` and the merge moves to a later step.

## 9. Decisions

All four are decided. The operator answered "go with best recommendations" (relayed by the platform lead, 2026-10-27), so each below is its recommendation; D1, D3 and D4 had been applied by the platform lead under that standing rule before. Build steps 3 and 4 follow steps 1 and 2.

- **D1 (decided: keep delivering). Limited, reset unknown.** Today a message is delivered (a turn discovers recovery) and may fail. Options: (a) keep, say so in the answer; (b) hold it with a retry every N minutes. *Recommend (a)* for this change; (b) is a separate decision because nothing ends that hold except a probe.
- **D2 (decided: yes). Does the sender see it before sending?** The route and `Agents list` field (3.3) are what the operator's "DRY it" buys in the UI; steps 1 and 2 stand without them. *Recommend yes.*
- **D3 (decided: one source). Signed out from the gate's refusal.** Make it a recorded `Auth` failure (one source, 3.1), or keep two inputs behind one `Availability`. *Recommend one source, after step 2 shows it is not invasive.*
- **D4 (decided). Name.** Keep the wire reason `idle` (it exists), and call the internal case `Cold` in code, so it isn't confused with `AgentState::Idle`.

Agent: Manoz@Area54
