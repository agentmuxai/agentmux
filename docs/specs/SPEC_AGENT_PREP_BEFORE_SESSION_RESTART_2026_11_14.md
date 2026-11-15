# Spec: prepare, then proceed. An agent gets a window to reach a safe point before a user action restarts its session

**Date:** 2026-11-14
**Author:** Agent5@narko (kanban #47642; the build is #47643)
**Status:** active — approved; build #47643: PR 1 (the continuation record, section 6) is in review, PRs 2 and 3 not started.
**Area:** agent lifecycle
**Read at:** origin/main `0aef75be5`. Every file and line below was read there (a few by a read-only audit, then re-read by me where the spec leans on them; the places I did not re-read are marked "audit").
**Asked by:** the operator, 2026-11-14, in their words:

> "do agents read ambient messages? I am thinking if a user changes the account, it should send an auto message to the agent giving it a warning, like 'User selected account change <details>.... prep for change' then once the agent is ready, then the account changes. how does that sound?"
>
> "right, lets also do a DRY audit across that chain of code. We want robust operation, and a reusable framework in case other user actions require agent prep before completing"

---

## 0. The answer in one page

**The decision this spec feeds:** when a user action is about to restart or replace an agent's session, how does the agent get to a safe point and leave a handoff first, and what is the one mechanism every such action uses?

**Recommendation.** One small srv module, `prep` ("prepare, then proceed"), that every such action calls instead of writing its own wait:

1. The action says "I will restart agent A's panes; here is what I will do, and here is the closure that does it."
2. `prep` decides in one place whether A has anything to lose. If not (idle, nothing claimed, nothing running), or if A cannot take a turn (out of quota, signed out, API offline, not running), the closure runs at once on AgentMux's own record. No message, no turn.
3. Otherwise `prep` sends A **one** `[AgentMux]` system note that **starts a turn** (the existing `deliver_system_note`, which nothing else can forge, section 3.2). It says what will happen, when, and what to leave.
4. A answers with one new tool action, **`Agents ready`**, carrying a handoff of at most about ten lines. The closure runs, and the handoff goes **first** in whatever the new session starts from.
5. It is a **window, never a gate**: 30 s by default; one extension to 120 s if the agent says it is mid-push or mid-deploy; the person's **"Switch now"** ends it at once; if the agent has not answered when it runs out, the closure runs anyway on AgentMux's own record.
6. The person sees the countdown **inline in the element they acted in** (the composer's account chip), no modal.

**And** the record a new session starts from is made robust on its own (section 6), because the commonest reason for an account switch is an account that cannot take a turn, and then AgentMux's record is the only handoff there is. Today that record is cut by the character, from the head, with no note (section 2.3). That is the bug that cost AgentY's "In flight when last seen" on 2026-11-14.

**Build order** (section 8): (1) the record, which stands alone and fixes the incident; (2) the framework with the account switch as its first user; (3) the next two actions (model or route switch, app upgrade and quit) as their own small PRs.

**What this adds that was not asked:** nothing human-only and nothing agent-only (the operator's autonomy tenet): the same path runs whether the person, an agent (`Identity` `switch`) or AgentMux itself asks, and a window that runs out proceeds.

---

## 1. The problem, with the real case

An account switch restarts an agent's panes with `--resume`. Today:

- The agent is told **nothing** before it. After a failover or a login expiry it is told once, after the restart (`failover::notify`, `login_expiry::resume`).
- AgentMux waits for the **turn** to end, not for the **task** to reach a safe point. An agent between turns can be mid-push, mid-deploy, waiting on a background command, or holding an uncommitted change.
- When `--resume` fails, the new session starts from a "continuation packet" that AgentMux built. On 2026-11-14 about 18:47Z, AgentY's switch failed to resume; the packet's running summary was cut mid-sentence inside "In flight when last seen", the section that mattered most, and AgentY rebuilt its state from GitHub.
- "Ambient" notes (a held `[AgentMux]` note, a bulletin) never start a turn. They are read with the agent's next turn, so they cannot say "prepare now".

---

## 2. What is there today (Part 1: the DRY audit)

### 2.1 The paths, one table

"Waits" is who holds the restart until the agent's turn ends. "Told" is what the agent hears from AgentMux about it, and when. "Record" is what the new session starts from. "Person sees" is the UI while it is pending.

| # | Path (initiator) | Waits for the turn | Agent is told | Record carried over | Window | Audit row | Person sees |
|---|---|---|---|---|---|---|---|
| 1 | **Account switch**: composer chip, failure row, `Identity` `switch` by an agent (person or agent). `account_episodes/switch.rs` `switch` -> `switch_to` -> `apply` | `AfterTurn` (default): the choice waits in pane meta `agent:pending_account` and `apply_pending` runs on every episode refresh (a 15 s tick, poked by a turn ending) until no pane is `busy` (`moves.rs` `busy`: agent state Working or Waiting). `Now`: `stop_turn` replaces the process | **Nothing before.** Nothing after for a person's switch | `--resume` of the provider session. If that fails, the continuation packet (2.3) | none; waits as long as the turn runs | `log_fleet_action_audit` `agent.account.switch`, actor = the caller's slug or `operator` (`switch.rs` `apply`) | A chip note "Switching to X after this turn" |
| 2 | **Bulk move** of an episode's agents (account strip). `moves.rs` `move_agents` | per agent, through the same `switch_to` | nothing | as 1 | as 1 | as 1, per agent | counts: moved, busy, failed |
| 3 | **Failover** on a usage limit (AgentMux). `failover.rs` | as 1 (`AfterTurn`) | **after** the restart: "[AgentMux] Your account hit its usage limit, so AgentMux moved you to X" (`failover::notify`); the lead gets a held note | as 1 | as 1 | as 1, actor `failover` | the agent's pane note |
| 4 | **Login expiry** (AgentMux). `login_expiry.rs` | as 1 | **after**, only to a pane whose turn the expiry cut: "Your last step may not have finished: check git status and your PR" (`login_expiry::resume_note`, via `resume::start`) | as 1 | as 1 | as 1, actor `failover:expired` | the agent's pane note |
| 5 | **Signed in again** (AgentMux). `login_recheck.rs`, `resume.rs` | no restart: it tells the pane whose turn failed on the expired login | after, paced 3 at a time, 2 s apart | none needed | n/a | not a switch | the agent's pane note |
| 6 | **Model or route switch** (the person, `operator_only`). `app_api/agent_model.rs` `switch_model` -> `restart_panes` -> `account_episodes::restart_panes_of` -> `moves::restart` | `resync_controller(forcerestart)` defers the restart to the turn's end inside the controller (a different wait from row 1's, 2.2) | **nothing** | `--resume`, packet if it fails | none | no `agent.model.switch` row of its own found in `agent_model.rs` | the panel shows the result |
| 7 | **Runtime change** (permission mode, effort, model alias) from the model panel: `runtime-apply.ts` `patchRuntime` -> `ControllerResyncCommand` `forcerestart` **from the window** | the controller's own deferral | nothing | `--resume` | none | not found (audit) | "Restart to apply" |
| 8 | **App upgrade and restart** (the person, sensitive op `upgrade`, 120 s). `sensitive_ops.rs`, `pending_shutdown.rs`; agents stopped with `Policy::app_exit` (`sagas/agent_teardown.rs`); `restart_resume` brings back those recorded as wanted running | the stop **interrupts** a turn; no wait for it | **before: nothing.** After: "[AgentMux] AgentMux was upgraded from X to Y ... your last turn was cut off mid-tool" (`upgrade_notice.rs`) | `--resume`, packet if it fails | 120 s countdown for the person to veto | `pending_shutdown` audit | banner with countdown and Keep/Now buttons |
| 9 | **Quit** (tray, Cmd+Q) | as 8 | nothing | as 8 | up to 120 s for the person | as 8 | as 8 |
| 10 | **Stop or close pane by an agent** (`Stop agents`, `ClosePane`). `pending_shutdown.rs` `Action::CloseTab/ClosePane/Stop` | no wait for the turn: stopped from outside | after the outcome, to the requester only | none; the session ends | 30 s (`OVERRIDE_WINDOW`) for the **person** to keep it | `pending_shutdown` audit | banner with Keep and Close now |
| 11 | **Stop or close by the person** (pane ×, tab ×) `sagas/close_pane.rs` | none: `Policy::close()` interrupts | nothing | none; the agent is parked or ended | none | saga record | the pane closes |
| 12 | **Watchdog stop** (idle output or max runtime; AgentMux) `blockcontroller/watchdog.rs` | none | nothing; the person is notified | none | none | log; claims released | a notification |
| 13 | **`Agents fresh`** (the agent itself). `app_api/agent_context.rs` `fresh_when_turn_ends` | `wait_for_turn_end`: polls `turn_active` every `TURN_END_POLL` up to 30 min | the agent wrote the handoff itself | **the agent's handoff alone** (up to 4,000 chars) as the new session's first turn; **no packet** (`spawn.rs`: a start "by choice" skips it) | 30 min wait, 30 s exit wait | `tracing` lines only | "scheduled" |
| 14 | **`Agents compact`** (the agent itself; also AgentMux's backstop, `compact_for_backstop`) | the same `wait_for_turn_end`, in `compact_when_turn_ends` | the agent asked | in-place compaction | 30 min | `tracing` | "scheduled" |

Not found in this audit: a restart path for a move between native and container. It is not a restart of an existing session: it is a different agent (`Agents create`).

### 2.2 Duplicated, and missing

**Duplicated** (one thing written more than once):

1. **"Wait until the agent is out of a turn" is written three ways.** (a) `account_episodes`: pane meta + `apply_pending` on the episode refresh, asking `moves::busy` (agent state Working or Waiting). (b) `agent_context::wait_for_turn_end`: a 250 ms poll of the controller's `turn_active`, 30 min, for `fresh` and `compact`. (c) `resync_controller(forcerestart)`: the controller defers the restart to the turn's end itself (rows 6 and 7). Two different questions are asked ("agent state is Working or Waiting", "`turn_active`"), with the same name in the author's head. (`agent_state::current_state` derives from `turn_active`, so they mostly agree; Waiting is the difference.)
2. **"Tell the agent something from AgentMux" is written seven ways.** `failover::notify`, `login_expiry::resume`, `resume::start`, `upgrade_notice`, `agent_context::handoff_message` (through `system_note_into_turn_or_hold`), `worker_notes::tell_leader`, and `held_notes::hold` callers. Each chooses on its own whether the note starts a turn or waits for the next one.
3. **"Restart the panes of agent A" is reached through three doors:** `account_episodes::moves::restart` (rows 1 to 6), `ControllerResyncCommand` straight from the window (row 7), and `agent_teardown` (rows 8 to 12).
4. **Countdown and veto is written twice for the same job.** `pending_shutdown` (an agent asks, the person has a window to keep: `request`, `keep`, `close_now`, `tell_requesters`, the `shutdown-pending` event, `ShutdownPendingBanner`) and `sensitive_window` for sensitive operations. Both are "someone acts, someone else has a window".

**Missing:**

1. **No path tells the agent before.** Rows 1, 2, 6 to 9 say nothing in advance.
2. **No path asks the agent for a handoff.** Only `Agents fresh`, which the agent starts itself.
3. **The record is not complete without a model.** The continuation packet leans on a Haiku summary (2.3), which an exhausted account cannot make.
4. **No audit row for what the agent was asked or answered**, and none for `agent.model.switch`.
5. **One pending switch has no timeout.** `agent:pending_account` waits as long as the turn does; nothing re-asks or gives up (the 15 s refresh only re-checks).

### 2.3 The continuation record, as it is (the part the incident hit)

Built in `continuity.rs` `build_continuation_packet` and used when a fresh spawn lands on prior history and the start is not "by choice" (`persistent/spawn.rs`, `resume_retry.rs`). Sections, in order: framing, **Running summary**, **Identifiers**, **Last request** (marked answered or NOT answered), **Recent exchange**.

- The **Running summary** is written by one Haiku call after a turn when 3 person turns have passed or 10 minutes have (`continuity_state.rs` `TURNS_PER_UPDATE`, `STALE_AFTER_MS`). It must have these sections, in this order: Current goal, **Last user request**, Done (verified), Done (unverified), **In flight when last seen**, **Waiting on the user**, Decisions and why, User preferences, **Exact identifiers**.
- **The cut.** `accept_state` redacts the reply and then does `redacted.chars().take(STATE_MAX_CHARS)` with `STATE_MAX_CHARS = 12_000`: a **head-only cut by character, with no mark that anything was cut.** The prompt asks for "about 1,500 words" (about 10,000 characters), so a long conversation lands on the cap. Because the "Done" lists come before "In flight" and the identifiers come last, a cut removes the identifiers and preferences first, and lands mid-sentence in whichever section holds the 12,000th character. When the Done lists are long, that is "In flight".
- **The turn cut is different and fine.** `cap` keeps the first two thirds and the last third of an overlong turn with "…[N characters omitted]…", and `build_continuation_packet` drops whole old turns and says "N earlier turns omitted".
- **The 24,000-character budget counts only the recent exchange** (`PACKET_BUDGET_CHARS` is checked against `used`, which the turns alone add to). The summary (up to 12,000), the identifiers (up to 40 lines) and the last request (up to 2,000) are outside it. Worst case the packet is about 40,000 characters, and nothing prioritises what survives.
- **The summary is up to 10 minutes and 3 person turns old** when the switch happens. "In flight when last seen" is when the summarizer last looked, not now.
- **It carries no runtime facts.** Claimed Kanban tasks, running background tasks (`db_background_tasks`), the Presence status line, git state: none is in the packet. They are things AgentMux knows exactly, without a model.
- **`Agents fresh` is a separate record:** the agent's own handoff, delivered as the new session's first turn, and no packet. It is the one place an agent-written handoff exists today.

### 2.4 What exists that this reuses

- **A way to start a turn from srv that nothing else can forge.** `Handler::deliver_system_note` (`reactive/handler.rs`): "now, which starts a turn if none is running", audited as `system_note` from `agentmux`. The `[AgentMux]` marker is quoted out of every delivered body by `sanitize::neutralize_markers` (`MARKER_WORDS`, including `agentmux`), so a jekt or forwarded text cannot carry it. **No new marker is needed.**
- **A countdown with a person's veto and an approve-now.** `pending_shutdown.rs`: a request ledger with an id, a deadline, a `Notify` the window task selects on, `keep` and `close_now` RPCs, an event the frontend renders, `tell_requesters`, a batch join. This feature is its **mirror**: there the agent acts and the person has a window; here the person acts and the agent has a window.
- **Agent state, claims, background tasks, work facts.** `agent_state::current_state`, the Tasks store, `db_background_tasks` (and since #7227, #7242 and #7253 its pids and exits are filed from srv itself), `work_facts/git.rs` (dirty files and commits ahead, cached by the overlap notes).
- **Held notes** (`reactive/held_notes.rs`): a note that attaches to the agent's next message, restored across a restart.
- **An agent tool path** end to end: `crates/mcp` schemas, `agents_tool.rs`, an srv route, caller identity from the agent token (`server/actor.rs`), audit (`log_fleet_action_audit`).

---

## 3. The framework (Part 2): "prepare, then proceed"

### 3.1 The shape

A new srv module `server/prep/` (ledger, ask, ready) and one entry point:

```rust
prep::request(state, Prep {
    def_id,                 // the agent
    action: "account_switch",   // a short name, for the note, the audit and the UI
    what: "move to account <label>; its session restarts and resumes",   // one line, in the note
    details: ...,           // optional lines for the note (from, to)
    actor,                  // who asked: "operator", an agent's slug, "failover"
    wait: Wait::Window,     // the person is watching: 30 s window.  (see 3.5)
    proceed: Box<dyn FnOnce(Outcome) -> Result<...> + Send>,   // what the action does: apply the switch
})
```

`Outcome` says how it came out: `Ready { handoff }`, `NotAsked(reason)`, `TimedOut`, `PersonSaidNow`. The closure runs **exactly once**. The first thing it does with the outcome is hand `Ready.handoff` to the handoff store (3.4).

### 3.2 Who is asked, and the one message

**Asked** only an agent that has something to lose, by this test (cheap reads only, no `git status` run by prep):

| Signal | Source | Cost |
|---|---|---|
| a turn is in flight (agent state Working or Waiting) | `agent_state::current_state` | a map read |
| a Kanban task is claimed by it | the Tasks store | a store read |
| a background command is running | `db_background_tasks` rows `running` for its panes (with a live pid, or a no-pid row seen in the last 2 h: the same rule the dock uses, #7226) | a store read |
| uncommitted or unpushed work | `work_facts` git facts if a fresh one is cached (dirty files, commits ahead) | the cache only; if none is cached the signal is skipped, never computed here |

Any one is enough. **None of them true: the closure runs at once** ("switches at once, no turn").

**Not asked** (the closure runs at once, on AgentMux's own record; section 6 makes that record complete):

- the agent cannot take a turn: `agent_availability` says `Limited` (out of quota), `SignedOut`, `NotRunning`, or its API is offline. This is the commonest case for an account switch, and the right one to skip: a prep turn on an exhausted account would just be held;
- the agent is a container agent whose controller is not up (it reads the choice when it starts).

**The message** is one `[AgentMux]` system note, sent with `deliver_system_note`, so it starts a turn. Its text (the build fixes the exact words in the PR; this is the content):

```
[AgentMux] Prepare for a change (prep p-1a2b): you chose nothing; the person did.
The person switched your account from <A> to <B>. In 30 seconds, or as soon as you answer, your session
restarts and resumes. A tool call in flight will be cut.
Do now: stop at a safe point (finish or park the step you are on; commit or push what must not be lost;
do not start anything long). Then call Agents ready with a handoff of at most about 10 lines:
  In flight: ...   Waiting on: ...   Next step: ...
If you are in the middle of a push or a deploy and need longer, say so: Agents ready more_time=true why="...".
You get up to 2 minutes, once. Then end your turn.
```

It names the actor ("the person", an agent's name, or "AgentMux"). It never asks for approval: it reports what will happen.

**Mid-turn.** If the agent is mid-tool-call, the note is queued behind it by the controller like any message; the window runs from the person's action, not from when the agent reads it. An agent that never reads it in time simply times out (3.5).

### 3.3 The answer: one tool action

`Agents ready` (a new action on the existing `Agents` tool; no new tool). Parameters: `handoff` (text, at most 1,500 characters, required unless `more_time`), `more_time` (boolean) with `why` (one line, shown to the person). Identity comes from the agent token, as `Agents compact` and `fresh` do (`own_pane_by_token`): an agent can answer only for its own pane, so a request id that leaks is useless to another agent.

- `ready` with a handoff: the window ends now; the closure runs (once the agent's turn has ended; see 3.5).
- `ready more_time=true`: the deadline moves to 120 s from the person's action. Once per request. The person's chip shows "waiting for <agent>: <why>".
- A second `ready` after the closure ran is refused: "this change has already happened".

### 3.4 What the new session starts from: the handoff goes first

The handoff is stored once, durably, in the agent's own zone (`agent:<defId>:current`, the place the running summary lives, so it survives account and channel changes): a file `continuity.handoff.json` holding `{prep_id, at, actor, action, text}`. One consumer, in the spawn path that already decides what a new session starts from (`persistent/spawn.rs`, next to `carry_continuation`):

- **`--resume` succeeded:** the handoff goes in as a held note on the new session's first message (`held_notes`, key `prep-handoff`): "[AgentMux] Before the change you left this handoff: ...".
- **`--resume` failed, so a continuation packet is built:** the handoff is the **first section of the packet**, above the running summary: "## Handoff you left before the change (written HH:MM)".
- Either way it is cleared when consumed, and dropped after 24 hours if never consumed.

`Agents fresh` keeps working as it does: its handoff is the same kind of thing, and the build makes it use the same store and the same consumer, so there is one place an agent-written handoff lives.

### 3.5 The window: never a gate

- **30 s by default**, counted from the person's action. **One extension to 120 s** when the agent says it needs it (`more_time`). These two numbers are constants in one place; the existing ones (`OVERRIDE_WINDOW` 30 s, `SELF_QUIT_WINDOW` 15 s) are not changed.
- **Ends early** when: the agent answers `ready` and its turn has ended; the person presses **"Switch now"**.
- **Ends at the deadline** whatever the agent has done. If it is still in a turn, `proceed` uses `Now` (the existing `stop_turn`). If it answered `ready`, it had already ended its turn by then; if it did not, AgentMux's own record is the handoff (section 6).
- **Never blocks the person.** There is no state in which the action waits for an agent that cannot answer.
- **Cancel:** choosing the account the agent is already on clears the pending change today (`set_pending(None)`); `prep` listens for that and drops the request, and tells the agent: "[AgentMux] The change was cancelled; carry on."
- **A second request for the same agent** while one is pending replaces the target and **keeps the deadline** (it does not restart the window); an identical one is a no-op. One prep per agent at a time.
- **srv restarts during a window:** the ledger is in memory, but the pending choice is already persisted in pane meta (`agent:pending_account`); after the restart `apply_pending` applies it at once, with no prep (the agent's turn context is gone anyway), and the framework's record (section 6) carries it.

### 3.6 What the person sees

Inline, in the element they acted in, no modal. For the account switch that is the composer's account chip: today "Switching to X after this turn" becomes

> Switching to X: waiting for Agent5 to reach a safe point · 24 s · **Switch now**

and, once the agent asks for longer: "Agent5 asks for more time: finishing a push · 1:40 · **Switch now**". It ends in "Switched" or, on timeout, "Switched (Agent5 did not answer in time; it resumes from AgentMux's record)". The bulk move (account strip) shows one such line with the longest window.

**Shared with the shutdown countdown, so not written again:** the ledger shape (id, deadline, a `Notify` the window task selects on), the `now` RPC (the mirror of `close_now`), the event the frontend renders (a `prep:pending` and `prep:cleared` pair in the same form as `shutdown-pending`), the countdown component's timer (a frontend `setInterval` over `deadline_ms`), and the audit and `tell_requesters` helpers. **Not shared:** the banner (it is the pane's, operator-only, and has Keep, which means the opposite here) and the actions (`Stop`, `ClosePane`).

### 3.7 Audit

Each request writes two rows with the same `request_id`, through `log_fleet_action_audit`:

- `prep.requested`: actor = the person (`operator`), the agent, or `failover`; subject = the action and the target.
- `prep.answered`: actor = **the agent**, with the handoff's length and whether it asked for more time, or `timed_out`, `not_asked:<reason>`, `person_said_now`.

The action's own row (`agent.account.switch`) is unchanged and is written when it runs. "Recorded as the person's action and the agent's answer, each under its own name."

### 3.8 Containers

The same: `deliver_system_note`, `Agents` actions and audit are the same for a container agent (audit). The only difference is the "something to lose" test: `work_facts` git facts may not exist for a container's workspace, so that signal is skipped; the other three work as they do.

### 3.9 What a prep turn costs

One ordinary turn, at the agent's current context size: the note in, a short reply and one tool call out. No more than any message costs, and **spent only on an agent with something to lose**: an idle agent with nothing claimed and nothing running costs nothing. An exhausted account is never asked. For a very large context (AgentY's) it is the same cached re-read any message costs; the build's first live run measures it (section 9) and reports the number.

---

## 4. The account switch as the first user (Part 3)

**Where it goes.** In `switch_to` (`switch.rs`), the one door. Today: if `AfterTurn` and a pane is busy, `set_pending(...)` and return `Queued`; else `apply`. New: before that, `prep::request(...)` with `proceed = apply(..., now)`. `AccountSwitchStatus` gains `Preparing` (the chip shows the countdown; the existing `Queued` stays for the no-prep case). `apply_pending` stays as the restart-resilient fallback, so a switch is never lost if srv restarts.

**Which callers prepare:** the composer chip, the failure row and `Identity switch` (all through `switch`), and the bulk move (through `switch_to`). **Which do not:** `failover` and `login_expiry` (AgentMux acts because the account failed; the agent is mostly not able to take a turn, and the existing after-the-fact notes stay). They benefit from section 6 instead.

**The next two actions to move onto it,** each its own small PR after the account switch is proven:

1. **Model or route switch** (row 6). It restarts the pane at the end of the turn and says nothing. A person-initiated change of the model the agent runs on is exactly this case, and the call is already `restart_panes` -> `restart_panes_of`, so it is one `prep::request` around the restart. (Today it is `operator_only` because it spends the person's key; the framework adds no new gate and does not touch that one.)
2. **App upgrade and quit** (rows 8 and 9): the largest user, since every agent restarts. The existing 120 s countdown is the person's window; prep runs inside it for each agent with something to lose, so the agent gets the note while the person can still keep AgentMux running. Waking many agents at once is the cost to watch, so the build paces them (a few at a time, as `resume.rs` already does after a sign-in) and only wakes those that pass the test in 3.2.

**Stay as they are, and why:**

- **Stop or close by the person** (row 11): the person decided to end it now, and an agent being closed has no new session to hand a handoff to.
- **Stop or close by an agent** (row 10): already has its mirror (the person's window to keep it).
- **Watchdog stop** (row 12): the agent is hung or idle; there is nobody acting.
- **`Agents fresh` and `compact`** (rows 13, 14): the agent is the one acting; it is its own prep. They only move onto the shared turn-end wait and the shared handoff store (section 5).
- **Runtime change** (row 7): see section 5, item 3: it is folded into the one door first, and then stays an immediate "Restart to apply" (the person is looking at the panel and chose it).
- **Failover, login expiry, login recheck** (rows 3 to 5): AgentMux acts; the agent usually cannot take a turn.

---

## 5. The DRY consolidation this includes

Only what the build touches anyway; each is a small, reviewable change, not a rewrite.

1. **One "is this agent in a turn" helper**, used by `moves::busy`, `agent_context::wait_for_turn_end`, `switch::pane_in_a_turn` and `prep`. One definition (agent state Working or Waiting), one name.
2. **One `wait_for_turn_end`**, the one `fresh` and `compact` already share, moved out of `agent_context.rs` to where `prep` and `account_episodes` can use it; `apply_pending` keeps its refresh but asks the shared helper.
3. **One door to restart a pane.** `runtime-apply.ts` calls `ControllerResyncCommand` from the window; the build routes it through the srv `restart_panes_of` that the account and model switch use, so a restart has one place a handoff and an audit row can be added. (Frontend change small; behaviour the same.)
4. **One "tell the agent" helper** with the choice made explicit (`starts_a_turn` or `held`): `failover::notify`, `login_expiry::resume`, `resume::start`, `handoff_message` and `prep` call it instead of each choosing.
5. **The countdown ledger** stays in `pending_shutdown.rs`; `prep` reuses its pieces (3.6) and does not copy them. The sensitive-operation window is left alone in this task; it is noted as the next candidate to merge into the same ledger.

---

## 6. Robustness of the continuation record (Part 4)

This part stands alone, ships first, and fixes the incident whether or not the framework is approved.

1. **Cut by section, never by character.** `accept_state` parses the summary by its `## ` headings. When it is over the cap it shortens in this order: the oldest lines of "Done (unverified)", then "Done (verified)", then "Decisions and why", keeping the newest; it never cuts **Current goal, Last user request, In flight when last seen, Waiting on the user, Exact identifiers**. It cuts on a line boundary, and where it shortened a section it writes "(N older items omitted)". If the protected sections alone exceed the cap, they are kept whole and the cap is raised for that version rather than cut mid-line.
2. **Priority order in the packet, whatever order the summarizer wrote.** The packet prints, in this order: the **handoff** (3.4, when one exists), **runtime facts** (item 4), Current goal, Last user request, In flight, Waiting on, Exact identifiers, Decisions, Preferences, then the Done lists last.
3. **One budget.** The 24,000 characters cover the whole packet, not only the exchange. The fixed-priority parts above are taken first; the recent exchange gets what is left (and still drops whole old turns with the existing "N earlier turns omitted"). The packet states what it shortened.
4. **Runtime facts, written by AgentMux, no model involved.** At the moment of the switch (and refreshed at the start of a prep), srv writes a short block from what it knows exactly: the Kanban task(s) the agent holds (number, title, status); background commands running (label, age, pid alive or not); its Presence status line and age; for its working clone, branch, files dirty and commits ahead (from `work_facts`, if cached); and the pending action itself. This is **the handoff when the agent cannot give one** (an exhausted account, a timeout), and it makes "In flight" no longer depend only on a summary that is up to ten minutes old.
5. **The summary's age is printed** with it ("written 18:31, the exchange below is newer and wins"), as the packet already does; the build also stamps the runtime facts with their own time.
6. **A count when anything is shortened**, in one line at the top of the packet: "Shortened to fit: 14 older Done items, 3 earlier turns."

---

## 7. Acceptance: three decisive live runs in a `task dev` (not a matrix)

Each in my own isolated `task dev` with a seeded stand-in agent (`task dev:seed`, the scripted fake Claude Code), driven by me, with the numbers reported.

1. **A switch on an agent mid-task with quota left.** The agent has a claimed Kanban task and a running background command. Switch its account from the composer. Expect: the agent receives one prep note and starts a turn; the chip counts down; the agent calls `Agents ready` with a handoff; the account changes; the new session's first message carries the handoff; the audit has `prep.requested`, `prep.answered` (actor the agent) and `agent.account.switch`. Report the prep turn's token cost.
2. **A switch off an exhausted account.** The agent's account is out of quota. Expect: no note, no turn; the switch runs at once; the new session starts from AgentMux's own record, including the runtime facts, with "In flight" intact (force a resume failure so the packet is used).
3. **An idle agent.** Nothing claimed, nothing running. Expect: it switches at once, no turn, no note.

And the unit and integration tests: section-aware summary shortening (a summary over 12,000 characters keeps In flight, Waiting and identifiers whole and says what it dropped); the single budget; the test for "something to lose"; the closure runs once under a race between `ready`, "Switch now" and the deadline; the window extension once only; a forged request id from another agent is refused.

---

## 8. Build plan

Three PRs, in this order. The spec commits with the first.

1. **The record (Part 4 and the runtime facts block).** `continuity_state.rs` section-aware shortening and `continuity.rs` one budget and priority order, plus the runtime facts writer. Stands alone; fixes the incident.
2. **The framework and the account switch (Parts 2 and 3).** `server/prep/`, `Agents ready` (the MCP schema, `agents_tool.rs`, the srv route), the handoff store and its one consumer, `switch_to` wired, the inline chip countdown, the audit rows, and the DRY items 1, 2 and 4. Frontend look checks on macOS and Linux (the chip).
3. **Follow-ups, one PR each:** model or route switch; app upgrade and quit (paced); DRY items 3 and 5.

---

## 9. Open points (for the operator, through AgentY)

1. **The window: 30 s default and 120 s on request.** The operator's words did not give numbers; these match the existing `OVERRIDE_WINDOW` (30 s) and the upgrade countdown's cap (120 s). Say if you want different ones.
2. **Whether an idle agent that holds a claimed task or a running background command should be woken.** Recommended: yes, because that is exactly "something to lose" while idle; the cost is one turn for that agent. The alternative is to wake only agents in a turn, which would miss the mid-deploy idle agent.
3. **Whether the app upgrade and quit should prepare every agent that passes the test, paced.** Recommended as the second follow-up, not in the first PRs, because it wakes many agents at once and deserves its own live run.
4. **The cost of a prep turn on a large context** is measured in run 1 and reported; if it is too high for the biggest contexts, the fallback is a cheaper form (a held note the agent reads at its next turn, the window shortened to the person's "Switch now") for agents above a size.

## 10. Non-goals

- No new gate: nothing in this is human-only or agent-only. The person, an agent and AgentMux use the same path; a window that runs out proceeds.
- No change to `failover` or `login_expiry` behaviour beyond sharing the helpers.
- No change to stop, close pane, the watchdog, or the existing 15 s and 30 s windows.
- No attempt to make the agent's handoff mandatory or to judge it: it is the agent's words, put first.
