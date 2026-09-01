# Transient-failure retry for turns with no rendered pane

**Date:** 2026-08-31
**Status:** Proposal. Not implemented.
**Owner:** unassigned
**Scope:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`,
`agentmux-srv/src/backend/blockcontroller/subprocess/host_spawn.rs`,
`agentmux-srv/src/backend/blockcontroller/subprocess/container_spawn.rs`,
`agentmux-srv/src/agents/failure.rs`,
`frontend/app/view/agent/hooks/useAgentFailure.ts`,
`frontend/app/view/agent/agent-view.tsx`
**Related:** `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` (the retry policy
and budget semantics this extends),
`SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md` (wired
classification into the persistent controller — deliberately stopped short of
retry), `docs/reports/REPORT_TRANSIENT_API_FAILURE_RETRY_STATE_2026_08_31.md`
(the assessment that surfaced this gap),
`REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` (identified the
ladder gaps fixed in PR #2870)

---

## 1. The gap

**Detecting a transient provider failure is server-side and works. Acting on it
is client-side and only exists while a pane is rendered.**

Everything up to the decision is backend:

- `agents/failure.rs::classify()` → `RateLimited` (429) / `Overloaded` (529) /
  `Network`, all `retryable: true`
- `core::persist_last_failure()` writes it to block meta (survives reload)
- `wps::EVENT_AGENT_FAILURE` publishes it per-block

The decision itself is `useAgentFailure` — a SolidJS hook, mounted in exactly
one place, `agent-view.tsx:1815`. **If no pane is rendered, nothing retries.**

`persistent.rs` already classifies on exit (`classify_exit_line`,
`persistent.rs:4116`) and persists the result, and its own doc comment states
the intent plainly: the banner is surfaced *"just without auto-retry."* That was
a correct scope decision for `SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION`
— which only meant to close the Claude/persistent classification gap — but it
leaves the retry decision structurally unreachable for headless turns.

**There are THREE independent classify/persist/publish sites, not two.** Any
implementation must cover all of them or it will leave a whole execution mode
silently un-retried:

| Site | Used by |
|---|---|
| `persistent.rs` | Claude Code (`ControllerType::Persistent`) |
| `subprocess/host_spawn.rs` | other host-run providers |
| `subprocess/container_spawn.rs` | any agent with `agentMode=container` — forced onto the subprocess controller by `blockcontroller/mod.rs`, executing each turn through its own path (`container_spawn.rs:410` classify, `:446` persist, `:453` publish) |

Container-backed turns are *especially* exposed, since running an agent in a
container is a headless-by-default posture. This is the strongest argument for
§2.1's recommendation to put retry ownership in **shared** controller-side
machinery rather than bolting an equivalent onto each spawn path — three copies
of a retry budget is three chances to diverge, and the divergence would be
invisible until someone's container agent silently stopped retrying.

### 1.1 Turns genuinely run without a pane

Cron is the clearest proof. `backend/cron/mod.rs:225` POSTs to
`/agentmux/reactive/inject`, and that handler delivers straight to
`blockcontroller::get_controller(&block_id)` (`server/reactive.rs:1152`) — a
server-side registry. No frontend participation at any point. The same is true
of jekt delivery, `FleetBroadcast`, `Loop`, and an MCP `SendMessage` aimed at an
agent whose pane isn't up.

So a cron-driven turn can hit a 429, be classified correctly, be persisted
correctly, show you a banner correctly the next time you open the pane — and
never have retried. The scheduled work simply didn't happen.

### 1.2 What is NOT affected

Worth stating precisely, because the obvious guess is wrong: **a background tab
is fine.** `workspace.tsx:35-40` keeps every tab mounted and hides inactive ones
with `display: none`, so their panes are still mounted and still retry.

The exposure is panes that are genuinely not rendered:

- a closed pane whose agent is still registered
- a non-active block inside a pane's block stack (only the active block mounts)
- any turn driven while the window is closed

## 2. Why this isn't a small change

The naive fix — "call the retry from `persistent.rs` too" — creates three
problems that need deciding, not discovering.

### 2.1 Two owners, one budget

If the controller retries *and* a pane is open, both fire: two re-sends per
failure, and two independent budgets counting down against the same episode.
The budget is currently hook-local (`autoRetries`, reset on genuine turn success
or a fresh user message).

**Recommended resolution: the server owns the budget; the pane renders it.**
The retry decision, the countdown, and the attempt count move into the
controller. `useAgentFailure` stops running its own timer and instead displays
server-published state (`retrying`, `next_attempt_at`, `attempts_remaining`),
with the manual *Retry now* / *Dismiss* actions becoming commands rather than
local mutations.

This is the same "single source of truth" direction
`SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md` already argues for
within the frontend — extended one layer down, because that spec's reducer is
still per-pane and therefore still absent for a headless turn.

The alternative — pane retries when mounted, controller retries only when not —
is rejected: "is a pane currently mounted" is not a fact the server can observe
reliably (a renderer can die without unsubscribing), and making retry behaviour
depend on it produces exactly the ordering-race class this codebase has already
spent ~40 documents on.

### 2.2 What "retry" means without a document

The pane's retry re-sends the **last user message**, read from the pane's
document (`retryLastTurn`, `agent-view.tsx`). If there is no prior user message
it falls back to relaunching the agent.

The controller has no document. It needs an equivalent source of truth for
"what was the turn that failed" — most plausibly the last input it dispatched,
which it already handles at the `agent.input` boundary. The interaction with
`persistent_resume`'s stale-`--resume` recovery (a *different* retry mechanism)
must be worked out explicitly: they must not both respawn the same turn.

### 2.3 Cap behaviour with nobody watching

A pane at cap shows a banner and waits for a human. A headless agent has no one
to show. Options, in increasing intrusiveness:

1. **Persist and stop** (status quo behaviour, just after N attempts instead of
   zero). Cheapest; the failure is already durable in block meta.
2. **Emit a notification** — the OS-taskbar/notification surface already exists
   (`SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md`).
3. **Mark the agent degraded** so Fleet/Swarm views can surface it in aggregate.

(1) plus (2) is the recommended starting point. (3) wants its own design.

## 3. Non-goals

- **No change to the retry policy itself.** The ladder, jitter and episode
  semantics from PR #2870 (`AUTO_RETRY_BACKOFF_S`) are the policy; this spec is
  about *where the decision runs*, not what it decides.
- **No change to Layer 1.** When the CLI is handling its own 429 backoff and
  emitting `rate_limit_event`, nothing here applies — the process is alive and
  retrying, and AgentMux's job remains to not misdiagnose it as a stall.
- **Not a queueing or scheduling system.** A retry re-runs one failed turn; it
  does not introduce durable job semantics for cron misses.

## 4. Phasing

**Phase 0 — quantify.** Before building: measure how often a transient failure
is classified for a block with no mounted pane. If that number is near-zero in
practice, this whole spec is not worth building and the honest answer is to
document the limitation instead. Same "measure before structural work"
discipline `SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` §5 arrived at
the hard way.

> **Phase 0(a) is DONE (2026-09-01); Phase 0(b) still needs a running build.**
>
> The data did not exist: no classify/persist/publish site emitted a log line,
> and the only logged classification was `agents/runner.rs:328`
> (`"agent run failed"`), a different path that a controller-driven turn never
> takes. Log archaeology confirmed it — searching the retained srv logs for
> `rate_limited` / `overloaded_error` / `agent_failure` returns **nothing**.
>
> **Do not read that silence as evidence of low frequency; it was evidence of
> no telemetry.** Closing this spec on an empty grep would have been the wrong
> call for the right-looking reason.
>
> **(a)** is now implemented, and deliberately *not* as a line per call site:
> one `tracing::warn!(target: "agent-failure", …)` inside
> `core::persist_last_failure` — the single choke point every path funnels
> through. It records `block_id`, `code` and `retryable`, and fires only for
> `Some` (the same function is called with `None` after every clean turn).
>
> Enumerating call sites here would have been a mistake, and nearly was. A
> draft of this note said "three". Review found more. A recount said "nine".
> Review caught that the recount was wrong too. The list spans `persistent.rs`,
> `container_spawn.rs`, `host_spawn.rs`, `agent_handlers/input.rs` and
> `app_api/agent_io.rs` — and this note deliberately gives **no number**, having
> now been wrong twice. Instrumenting the choke point is immune to the whole
> question, which is the point: if the correct count is load-bearing, the design
> is wrong.
>
> **(b)** remains: let a build run, then read the `agent-failure` target. Note
> it answers only half the original question — *how often a transient failure
> is classified* — because "was a renderer subscribed for that block" has no
> query behind it today (`Broker` exposes no per-scope subscriber lookup;
> `WaveEvent::has_scope` is a different thing). Adding one is a prerequisite
> for the full measurement and is deliberately left out of a logging change.

**Phase 1 — move the budget server-side**, with the pane rendering server state
(§2.1). Behaviour-neutral when a pane *is* open; that equivalence is the
acceptance test.

**Phase 2 — controller-side retry** for headless turns (§2.2), including the
`persistent_resume` interaction.

**Phase 3 — cap surfacing** (§2.3 options 1+2).

## 5. Open questions

1. **Should a headless retry re-mount anything?** If a turn is retried while no
   pane exists, the transcript still accumulates server-side — but nothing
   validates that the pane reconstructs correctly from a turn it never
   observed. Needs a test, and possibly nothing more.
2. **Does this interact with agent idle-timeout / auto-start?** A controller
   that retries for ~4 minutes is a controller that stays alive for ~4 minutes;
   `idle_timeout_minutes` must not reap it mid-ladder.
3. **Fleet-wide backoff.** PR #2870 added per-agent jitter, which de-syncs a
   broadcast. If retry moves server-side, the server can see *all* agents
   failing on one account and could back off globally rather than per-agent —
   strictly better, and only possible once the decision is centralized. Worth
   scoping in Phase 2, not before.
4. **Is `restart_on_crash` related?** The agent definition carries the field
   (`agent_seed.rs`) but no controller consumes it. Either it is dead config
   that should be removed, or it is the natural home for some of this policy.
   Resolve before adding a parallel mechanism beside it.
