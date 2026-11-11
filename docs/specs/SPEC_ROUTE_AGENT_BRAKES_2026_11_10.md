# Route agent brakes: a spend cap, budgeted compaction and loop detection

**Status:** active. Approved by the operator on 2026-11-10 ("yes proceed with best recommendations", relayed by AgentY), with every decision in section 10 taken as recommended. Author: Agent5@narko, 2026-11-10. Task #46807 (spec), build #46808. Parent: [SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md](SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md) sections 9 and 11, which this replaces where they differ (the cap and its default). It also folds in #27459 P5 (spend on the card and lists, and the cap) and #31427 (the degenerate thinking loop).

## 1. What happened, and what it says

Glim (Claude Code on DeepSeek V4.1 Flash through OpenRouter) stopped on 2026-11-10 with `402 This request would exceed your available credits`. Its result frame: 890 turns, 13.8 h of API time, `total_cost_usd` 798.50, about 720M cache-read and 58M input tokens, a 70-minute turn and a 2.7-hour turn, a context grown to about 1M tokens and never compacted, 49 identical failing `Tasks` calls in one stretch, four self-review pushes on one PR. The Swarm showed it idle (#42786, fixed). Nothing capped it.

Three facts from the code (all on main at 4c0536c37) shape the design:

1. **The 798.50 is the CLI's estimate, not a bill.** `total_cost_usd` is priced with Claude's rates. For a route agent srv already replaces it with the provider's own charge (`model_gateway/spend.rs rewrite_result_total`, #30143), but the replacement needs the gateway to have seen a charge in that session. At the route's table prices, the tokens quoted above cost on the order of tens of dollars, not hundreds (720M cache-read at $0.003/M is about $2; 58M input at $0.15/M is about $9). So the credits may have been low to begin with, and the real spend is unknown until the gateway's figure for that session is read. This matters for the default cap: it has to be set from the provider's number, which is what the gateway records.
2. **The context ran to 1M because srv told the CLI to.** `model_gateway/launch.rs env_for` appends `[1m]` to the model id when the model's context is 1M or more (`LONG_CONTEXT`), which makes Claude Code take a 1M window and auto-compact near 967K. That is right for a model that really reads 1M, and wrong for the bill: every turn re-reads the whole context.
3. **The gateway is the one place that sees every request of a route agent**, with the agent, the route, the model and the usage of each response (`server/model_gateway_handlers.rs`, `Metered::settle` calls `spend::record`). It is the right place for a hard stop. But its day table is in memory (`spend.rs` `table()`): it starts again with srv, so a restart gives the agent a fresh day.

## 2. The three brakes, in one line each

| Brake | What it stops | Where it lives |
|---|---|---|
| **Spend cap** | an agent or a route spending more than a day's budget | the gateway (hard), with a stop and a card in the pane |
| **Budgeted compaction** | a route agent re-reading a 1M-token context every turn | the launch environment (`CLAUDE_CODE_AUTO_COMPACT_WINDOW`), with a srv backstop |
| **Loop detection** | the same failing call, or the same thought, repeated | one detector module on the harness's stream |

They are independent: each can ship alone, in this order (section 8).

## 3. Spend cap

### 3.1 What is counted, and where it is kept

- **The figure** is the one the gateway already computes per response: the provider's own charge when the response carries one, else the route's price table (`spend::record`). Claude login agents never go through the gateway and have no cap here (they have plan limits).
- **Persisted.** The day's spend per agent is written to the history store (a small `agent_spend_day` table keyed by agent id and UTC day, updated when a response settles, kept 14 days), so an srv restart does not reset it. `spend::today` keeps reading the in-memory table; the table is loaded from the store at start and written through on `record`.
- **Per route.** The route's day is the sum of its agents' days, computed from the same rows (a `route` column).

### 3.2 The caps

| Cap | Default | Set by | Notes |
|---|---|---|---|
| **Per agent per day** | **$20** for an agent on a route (the task's example; section 9 of the parent spec said $5, see D1) | the agent's definition setting `agent:spend_cap_usd`; `0` or `none` means no cap | The agent's own default is stored when it is first put on a route. |
| **Per route per day** | **$100** (mirror of the OpenRouter key's own limit) | the route's setting in Settings; an agent through `Agents update` | The sum over all agents on that route. |

- **The OpenRouter key limit is the hard stop; the in-app cap is the one that explains itself.** The key limit (set by the person in OpenRouter) cuts every user of the key, ends in an opaque 402, and counts spend we do not see. The in-app cap stops one agent first, says why in the pane and to its leader, and can be lifted. The Vault entry's notes keep recommending a key limit (parent spec section 9); the app does not rely on it.
- **A leader can raise or reset a cap, and a person can.** No operator-only gate (the workspace rule): `Agents update` with `spend_cap_usd` on a worker the caller leads, or on itself only downward; the pane card's buttons call the same API. **Every raise and reset is written to the audit log** with who, from and to.

### 3.3 What happens at 50% and at 100%

- **At 50%** of the agent's cap (once a day): one message to the agent's leader, **not urgent** (so an idle leader is not woken for it): "Glim is at $10 of its $20 today on OpenRouter (DeepSeek V4.1 Flash)." The leader is the worker's recorded lead (`agent_seat::worker_of`, and the seat's `lead` for a leader that is not a spawner). An agent with no leader gets the card only. Sent through the reactive handler (`inject_message`, priority normal).
- **At 80%** the agent's own pane gets a card line ("80% of today's cap"), as the parent spec says; it does not message anyone.
- **At 100%:**
  1. The gateway **refuses the agent's next request** with `402` and a body in the shape the harness already understands from the provider's own out-of-credit answer: "spend cap: this agent has used $20.00 of its $20.00 for today (UTC). A person or its leader can raise the cap." The 402 is the same status Glim's real credit limit returned, after which Claude Code ended the turn and did not retry; the build confirms that on a live refusal before relying on it (section 9, to test).
  2. srv **ends the turn in flight** (the `interrupt` control request the persistent harness already takes, `blockcontroller/persistent/mod.rs interrupt_control_request_line`), so a long turn does not sit retrying.
  3. The agent is marked **stopped: spend cap**: a pane meta key `agent:spend_stopped` (`{at, cap, spent, route, model}`), read by the pane card, the Swarm and the Agents list, and a status of `blocked`, "Stopped: spend cap ($20.00 today)". The process stays up; the next day (UTC midnight) or a raise clears it by itself.
  4. The leader gets one message (the same hold-safe kind as at 50%).
- **A raise** (the card, a leader, a person) writes the new cap, clears `agent:spend_stopped` and lets the gateway answer again; the agent's next turn is whatever the person or leader sends it. **Reset today** zeroes the agent's day rows (audited), for a spend that was wrong.
- **Route cap at 100%:** every agent on the route is refused the same way, with "the route's cap of $100 for today is used", and each leader is told once.

### 3.4 The card and the lists (this is #27459 P5)

- **The pane card** `AgentSpendCard`, above the composer, built like the model-undone card (`AgentModelUndoneCard`): a meta key, a small reader (`spend-stopped.ts`), the kit's `Button` and `tip()`. Text: "Stopped: this agent used $20.00 of its $20.00 today." Buttons: **Raise to $40**, **Reset today**, **Dismiss** (the stop stays; only the card goes).
- **The agent card, the Swarm row and `Agents list`/`conversations`** get one chip and one line: "$4.20 of $20 today" on a route agent, nothing on a login agent. The Swarm card belongs to Camper and the lifecycle to Manoz: the build names both in its PRs and agrees with them where the chip sits (section 8).
- **The agent's own usage line** already shows the session cost; it shows today's figure and the cap beside it.

## 4. Budgeted compaction for route models

### 4.1 The change

`env_for` (`model_gateway/launch.rs`) sets, for an agent on a route, **`CLAUDE_CODE_AUTO_COMPACT_WINDOW`** to the route's compaction budget: **300K tokens by default**, per route and per model (a route entry's `compact_budget`), never above the model's context. The CLI reads that variable (`agent_context_usage.rs` header; the CLI reports the effective point as `autoCompactThreshold`), so Claude Code compacts when the context passes the budget, with its own summarizer, as it does today at the window's end. **Claude login agents keep the 1M window** and are not touched: the variable is set only on a route.

The `[1m]` marker stays (the model really reads 1M; the budget is a cost choice, not a capability one).

### 4.2 Interaction with Claude Code's own auto-compact

- **There is one auto-compactor: Claude Code's.** The budget moves its trigger; srv does not run a second summarizer. After a compaction the CLI's boundary moves (`lastCompactionBoundaryAt`), which the pane already shows.
- **The srv backstop.** srv knows each response's input size from the gateway. If a route agent's context is more than **1.5 times its budget** at a turn boundary (the CLI's auto-compact is off, failed or ignored the variable), srv sends the agent `/compact` the way the pane's Compact button does, once, between turns, never mid-turn, and records why in the audit log. If it fails twice, it raises the spend card's warning ("context is 480K; compaction did not run") and tells the leader.
- **The keep list.** The compaction the agent runs itself (`Agents compact` with `keep`) is unchanged. For the automatic one, the harness's own summarizer decides what to keep; the spec for a better keep list ("the current task number and attempt, the branch and PR, decisions made") is a follow-up that needs the summarizer's instructions to be settable, which they are not today (D3).
- **The cost of compacting** is one request over a context at the budget (300K at $0.15 per million input is a few cents on the Flash model), far below re-reading a 1M one on every turn.

### 4.3 Why 300K

At a cache read of $0.003 per million (DeepSeek V4.1 Flash) a 300K context costs about a tenth of a cent a turn when the prompt cache holds, and about $0.045 when it does not; a 1M context costs about $0.15 uncached. Over the 890 turns above, an uncached 1M context is about $130 against $40 at 300K. The default is a guess to be set from the first measured week (D2).

## 5. Loop detection

One module, `agents/loop_guard.rs`, fed from the persistent harness's stream (`stdout_reader.rs`), which already sees every frame. It holds, per agent and per turn, two detectors.

### 5.1 Repeated failing call

- **What it sees:** a `tool_use` block (`id`, `name`, `input`) and the `tool_result` that answers it (`tool_use_id`, `is_error`). Key: `(name, hash of the normalized input)`.
- **Rule:** the same key failing **5 times** in one turn (`agents:loop_failing_calls`, default 5). Failures between other calls count; a success of that key resets it.
- **Action:** end the turn (the same `interrupt` request), add an error row to the transcript, "repeated failing call: Tasks x5 (the same arguments)", and send the leader one message: "Glim repeated a failing call 5 times (Tasks claim, missing attempt) and its turn was ended." The agent can be sent a new message and goes on; it is a brake, not a stop.
- **Glim's 49 identical failing Tasks calls** would have ended at the fifth.

### 5.2 Degenerate thinking (#31427)

- **What it sees:** the thinking and text deltas of a turn.
- **Rule:** a normalized line (lowercased, whitespace and punctuation collapsed) of at least 3 characters repeated **20 times in a row**, or a window of the last 4 KB whose compressed size is under 5% of its length. Both are cheap and need no model. The numbers are starting points (D4).
- **Action:** the same as 5.1: end the turn, error row "stuck: repeating the same thought", message to the leader.
- #31427's own task text was not found in the repo's docs when this was written; the build reads it first and adds its cases to the detector's tests.

### 5.3 The missing `attempt` (task item c)

The `Tasks` actions that act on a claimed task refuse a call without `attempt`. Today the refusal does not say what to pass, and a weak model repeats the call. srv knows the lease holder's attempt, so the refusal becomes: "attempt is required: you hold #46807 as attempt 1; pass id=46807 attempt=1" (and when the caller holds more than one task, it lists them). It does **not** default the attempt silently: the argument exists to reject a stale holder, and a default would defeat it (D5).

## 6. What the three brakes do together, on Glim's day

| Hour | Before | With the brakes |
|---|---|---|
| Context passes 300K | keeps growing to ~1M | the CLI compacts at 300K |
| The 5th identical failing `Tasks` call | 49 calls | the turn ends at the 5th, the leader is told |
| 50% of $20 | nothing | the leader is told once |
| 100% of $20 | nothing, until the key's own limit said 402 | the gateway refuses, the turn ends, the pane says why, the leader is told |
| srv restarts | the day starts again | the day is kept |

## 7. Edge cases

| Case | Behaviour |
|---|---|
| **A response with no usage** (an interrupted stream) | priced by the route's table from what was read, as today; a request that returned nothing costs nothing |
| **A cap raised mid-turn** | the gateway reads the cap at each request, so the next request after the raise goes through |
| **Two agents on one definition** (a template made twice) | each agent id has its own day and cap |
| **The day rolls over at UTC midnight** | `agent:spend_stopped` clears by itself on the first request of the new day; a cap that was raised stays raised |
| **A cap of 0 / none** | no cap and no 50% message; the route cap still applies |
| **A leader's own route agent, or an agent with no leader** | the card and the lists; no message (nobody to send it to) |
| **Spend counted but the key limit hits first** | the provider's 402 reaches the harness as it does today; the pane card says "the route's key refused" when the body is a credit refusal and the in-app spend is under the cap (so the person is pointed to OpenRouter, not to the in-app cap) |
| **A container agent** | the same: its harness reaches the gateway through the same route, and the loop detector runs in srv on the stream |

## 8. Build plan and who is asked

| PR | What | Effort (agent-days, my estimate) |
|---|---|---|
| **A. Spend cap** | the persisted day table; the two caps and their settings; the gateway refusal; the interrupt and `agent:spend_stopped`; the 50% and 100% messages; the pane card; the chip on the agent card, Swarm row and lists; the audit entries; `Agents update spend_cap_usd`. Includes #27459 P5. | 4 to 5 |
| **B. Budgeted compaction** | `CLAUDE_CODE_AUTO_COMPACT_WINDOW` in `env_for`, per route and model; the backstop; the check that the CLI honours the variable on a route model | 1 to 2 |
| **C. Loop detection** | `loop_guard.rs`, both detectors, the error row, the leader message; the `attempt` hint | 2 to 3 |

The operator asked for the cheapest first, since they stop the bleeding: build order is **B (compaction window), then C (loop detector), then A (spend cap)**. A is the one that would have stopped the day; B and C need no UI of their own beyond an error row. Every PR follows the usual gates; A's card and chip get the macOS and Linux look checks.

**Coordination:** Manoz owns the agent-pane lifecycle (where `agent:spend_stopped` and the interrupt sit); Camper owns the Swarm card (where the chip sits); Lazo's muxreview needs nothing. I tell both when A's PR is opened, before it merges.

## 9. To test in the build (not assumed)

- That Claude Code ends the turn on a gateway `402` with this body and does not retry (Glim's real one did; ours must).
- That `CLAUDE_CODE_AUTO_COMPACT_WINDOW` below the model's window moves the CLI's `autoCompactThreshold` on a route model with the `[1m]` marker (read back through `get_context_usage`).
- That an `interrupt` control request ends a turn that is waiting on a retry.
- The real spend of a route session against the gateway's figure (the first measured week sets the defaults in D1 and D2).

## 10. Decisions (all taken as recommended, 2026-11-10)

| # | Question | Decided |
|---|---|---|
| D1 | **The per-agent daily default** for an agent on a route: $20 (this task) or $5 (the parent spec's D6) | $20 for a worker doing a day's work; a leader can lower it per agent. Revisit after a week of real figures |
| D2 | **The compaction budget** for a 1M-context route model | 300K, set per route and model |
| D3 | **A better keep list** for automatic compaction (needs the summarizer's instructions to be settable) | Later; not in B |
| D4 | **The loop thresholds**: 5 failing identical calls; 20 repeated thoughts | As written; every number is a setting |
| D5 | **`attempt` missing:** a clearer refusal, or default it for the lease holder | The refusal (it keeps the stale-holder guard) |
| D6 | **The 100% stop refuses at the gateway** (hard) rather than only interrupting the turn | Yes: the gateway is the only place that cannot be bypassed |
| D7 | **The route cap** default | $100 a day, mirroring the key's own limit; set per route in Settings |

## 11. What I did not do

I did not run a route agent against a refusing gateway or against the compaction variable (section 9 lists both as tests). I did not read #31427's task text or Glim's transcript; the real cost of that session is unknown (section 1, fact 1). I did not read the Swarm card or the lifecycle code beyond where they would take a chip and a state, and I took the agent's leader from `agent_seat::worker_of` without testing a leader that is on another host. The effort figures are guesses.

## 12. As built

- **B, the compaction window (first PR).** `catalog.rs` `COMPACT_BUDGET` (300K) and `Model::compact_budget()`; `launch.rs env_for` sets `CLAUDE_CODE_AUTO_COMPACT_WINDOW` for a model whose window is over the budget. An existing route agent takes it at its next launch. The srv-sent `/compact` backstop of section 4.2 is not in this PR: the stdout reader that sees the turn end has no `AppState`, which starting a turn needs, so it is filed as its own task; the existing turn-end hint (`context_reading::take_turn_end_hint`) is the nudge meanwhile.
- **C, the loop guard (#6933).** `agents/loop_guard.rs` reads the harness's stdout frames in the reader: five identical failing calls, or a thought (finished or still streaming) that repeats, end the turn with the control-protocol `interrupt`; the agent is handed a held note and its leader gets one message (`persistent/loop_stop.rs`). `Tasks` with no `attempt` answers with the attempt the agent holds. Not built: the transcript row for a guard-ended turn (#47375), fencing the interrupt to the reader's process generation (#47379), blocked tasks in the attempt hint (#47380).
- **A1, the spend cap, backend (this PR).** `storage/spend_day.rs` keeps the day per agent, route and UTC day (`db_agent_spend_day`, 14 days); `model_gateway/spend.rs` writes each change through a writer thread and reads the last two days at start, and also keeps the highest alert level sent so a restart does not repeat a message. `model_gateway/cap.rs`: the agent's cap is the definition content `agent:spend_cap_usd` (absent: $20; `none` or `0`: no cap), the route's the setting `model:spend_cap_usd:<route>` (absent: $100). The gateway checks both at every request and answers `402` with type `billing_error`. `server/spend_watch.rs` acts off the stream's task: 50% a non-urgent note to the leader (`worker_notes::tell_leader`); 80% pane meta `agent:spend_warning`; 100% (or a route's cap) pane meta `agent:spend_stopped`, a `SIGINT` to the pane's controller, and a note to the leader, each once a day per agent. A raised cap, a reset, or the first response of a new day takes the stop off. `Agents update` takes `spend_cap_usd` and `reset_spend_today`: the agent's leader may set and reset; an agent may only lower its own. The audit row records the cap before and after. Differences from the plan above: the default $20 is not written when an agent is put on a route (absent means the default, so a changed default reaches existing agents); the `blocked` status line is not set. Not in this PR: the pane card, the chips on the agent card, Swarm row and Agents list, and the window's Raise / Reset buttons (A2); that Claude Code ends the turn on this 402 is not yet seen live.

- **A2, the pane card and chips (#6974, #6988).** The stop card and the 80% line above the composer (`AgentSpendCard`), a chip on the Swarm card (`SpendChip`), `agent.spend` (set and reset, the window's), and the `spend` object in `Agents list`. A stop or warning from an earlier UTC day is taken off by the minute refresh and ignored by the pane's readers (re-read at each UTC midnight); an agent that left its route loses its chip, stop and warning. Not built: a returning capped agent gets its stop card back (#47419).
- **D, the brakes on a vendor-key harness (#47381, the DeepSeek Harness `dsh`, ACP).** `LoopGuard::observe_acp` reads the ACP stream (`tool_call`, `tool_call_update` with `status` `failed` or `completed`, `agent_thought_chunk`, `agent_message_chunk`; the answer to a `session/prompt` ends the turn) with the thresholds of C, and the ACP reader (`acp_spawn.rs`) ends the turn with `session/cancel`; the held note and the message to the leader are the same code as Claude's (`blockcontroller/loop_stop.rs`). Context: `dsh` sends `usage_update` with `used` and `size` (seen: 664K of 1M) and ACP has no knob for a budget, so the leader is told, held and without waking it, at 300K and at 600K. Spend: `dsh` sends no cost and no per-call tokens (its prompt answer carries no `usage`), and its key goes straight to DeepSeek, so there is nothing to meter and no gateway to refuse at; `Agents list` says `measured: false` for it. The ceiling is the vendor account's balance. Not built: an estimate of spend from `usage_update.used` (it would be a guess presented as a figure).

| Brake | Claude Code on a route | Claude Code on its own login | DeepSeek Harness (`dsh`, ACP) | Other ACP and app-server harnesses |
|---|---|---|---|---|
| Loop guard (C) | yes | yes | yes (D) | ACP: yes (D); Codex app-server: no |
| Compaction window (B) | yes (300K) | no (the harness's own) | no knob; leader told at 300K and 600K (D) | no |
| Spend cap (A) | yes, at the gateway | no (plan limits) | not measurable (D says so) | no |
