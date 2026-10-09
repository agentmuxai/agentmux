# SPEC: Agent turns that return to the user, what started them, and a live status that says what is happening

**Status:** active. Phase 1 (the turn ledger, §4) is implemented in PR #4492; see §4.7 for how the build differs from the design below. Phases 2–4 are proposed.
**Date:** 2026-10-08 · **Author:** agent5
**Components:**
- Turn accounting: `crates/srv/src/backend/blockcontroller/health.rs` (`TurnActivityTracker`), `persistent/stdout_reader.rs`, `persistent/queue.rs`, `persistent/input.rs`, the controller status publish; frontend `frontend/app/store/agent-pane-state/` (`reducer.ts`, `types.ts`, `turn-contribution.ts`).
- Working row: `AgentWorkingRow` in `frontend/app/view/agent/components/AgentFooter.tsx`.

**Builds on:**
- `SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md`: the ambient summary in the row.
- `SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md`: one output counter with a ↑/↓ arrow.
- `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.3: `TurnOrigin` and turn provenance.
- `SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md`: jekts are written to a running agent at once.
- `REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md` §2.3a: the busy indicators are the input gate, and backgrounded work belongs to the dock.

---

## 1. Request, as given

> the agent will be busy for a while, and the stat/timer will reset, it looks like the tally is per sub-turn, we want it per turn that returns to the user. We also need to be made aware of turns triggered by external events, whats a best practice for that. Is there a difference between a human turn and an agent turn? lets hash out that terminology. Finally, for the status, inprogress text, currently it is just to ambient haiku summary, we want to make it a richer more dynamic experience, with a sophisticated state machine that conveys surprisingly appropriate data at the right time. It plays into the exact architecture, so we can get the right, lets take a look, write a spec to file

There are three asks:
- **A.** The timer and the token tally cover the whole turn that returns to the user (§4).
- **B.** Make the user aware of turns started by something other than the user. This includes a recommended practice (§5).
- **C.** A live status line driven by a real state machine, not just the ambient summary (§6).

The terminology in §3 underpins all three.

---

## 2. Decisions this spec asks for

| # | Decision | Recommendation |
|---|---|---|
| D1 | What "turn" means in the UI | The span from leaving "waiting for you" to returning to it with nothing queued (§3). The CLI's own unit becomes a **pass**. |
| D2 | Who owns the turn boundary | srv. It sees every input, every `result`, and the CLI's own wake-ups, and it is alive when no pane is mounted (§4.3). |
| D3 | Whether input that arrives during a turn joins it | Yes: queued messages, mid-pass jekts, and a CLI wake-up that follows a `result` back-to-back all join. A message the user types after the turn ended starts a new one (§4.3). |
| D4 | Naming turns by trigger | **User turn** (the human, in the pane or by Swarm broadcast) and **external turn** (everything else). No separate "human turn"/"agent turn" (§3.2). |
| D5 | Where the status text comes from | Mostly deterministic signals and text the model already writes (Bash `description`, the Agent tool's `description`, the todo's `activeForm`, thinking). Haiku stays for the session goal only (§6.2). |
| D6 | Whether raw tool text returns to the row | No. The row says what is happening in human terms, only once it has lasted long enough to matter. This respects the 2026-10-02 decision to drop `tool · arg` (§6.6). |

---

## 3. Terminology

### 3.1 The four units

From smallest to largest:

| Term | What it is | Boundaries | Today's name in code |
|---|---|---|---|
| **Call** | One model API request and its streamed reply | `message_start` … `message_delta` | "call" (`TokensIn`/`TokensOut`) |
| **Step** | A call plus the tool calls it made | A call's start to its tools' results | none; Claude's `result.num_turns` counts these |
| **Pass** | One CLI query: input in, `result` out | Input written, or the CLI's own wake-up … the `result` frame | "turn" everywhere: `turnPhase`, `TurnEnd`, `turn_active`, `result.duration_ms` |
| **Turn** | Everything the agent does between two moments of "waiting for you" | Leaves idle … back at idle with nothing queued for it (§4.3) | not modelled |

A turn holds one or more passes. Most turns are one pass. Multi-pass turns are the case the request describes: a queued message, a jekt that lands mid-pass, or a background task whose completion wakes the CLI right after a `result`.

**Two collisions to fix as part of this work:**
- The Worked line's secondary text, `$0.412 · 14 turns`, prints `result.num_turns`. That figure counts **steps**, not turns, so it becomes `14 steps`.
- New code uses `pass` for the CLI unit. Existing identifiers (`turnPhase`, `TurnEnd`, `turn_active`) are not mass-renamed in this work. Their doc comments get a one-line pointer to this glossary, so a reader knows they mean a pass. A rename can be its own refactor later (§9, Q1).

### 3.2 Human turn vs agent turn

In conversation terms the human takes a turn by speaking, and the agent takes one by working and replying. In AgentMux the human's message is input, not something we time. The unit we time and report is always the **agent's turn**. What varies is what started it, so turns are named by their **trigger**:

- **User turn:** started by the human. That means typing in this pane, or a Swarm broadcast, which is the user's own words by definition (`fleet.rs` `BROADCAST_TURN_ORIGIN = TurnOrigin::User`).
- **External turn:** started by anything else. That includes:
  - a jekt from another agent;
  - a service jekt (github-consumer review notices);
  - a schedule (cron, loop, a supervisor nudge);
  - a background task whose completion woke the CLI.

A turn has exactly one **trigger** and may absorb further **inputs** (§4.3). The trigger fixes the turn's kind, and absorbed inputs never change it. A user turn that absorbed a jekt is still a user turn, and shows the jekt as an absorbed input.

"System" input (hidden memory reinjection after compaction, a `/btw` side question) is neither. It never starts a visible turn. It joins the current turn if one is running, and otherwise runs as a hidden pass that doesn't count (§4.4).

**This is a display model only.** The security meaning of `TurnOrigin` and `TurnProvenance.tainted` (what the self-quit gate checks per pass, `sagas/self_quit.rs`) is unchanged. A user turn that absorbed a jekt is reported as a user turn with an external input. The gate keeps refusing a self-quit in that pass exactly as it does today.

---

## 4. Part A — the tally covers the whole turn

### 4.1 What happens today

All of these reset at every pass boundary:

| What | Where | Why it resets |
|---|---|---|
| Elapsed timer | `AgentWorkingRow`'s `loadStartMs` (`AgentFooter.tsx` ~L225–281) | Set when `props.loading` rises, nulled when it falls. `loading` falls at each pass end: srv publishes `turn_active: false` on the `result` frame (`stdout_reader.rs` ~L199–289), so `ReconcileTurnActive{active:false}` sets Idle, then `TurnEnd` sets Done. The signal is also component-local, so any remount of the row restarts it. |
| Token counter | `turnTokens`, cleared by `TurnEnd` and `StreamSessionStarted` (`reducer.ts` ~L860, L888) | Each pass starts from zero. |
| Worked line | `sessionStats` from `mergeStats(result, turnTokens)` at `TurnEnd`; cleared on `TurnStart` | Shows the last pass's `duration_ms`, `output_tokens` and cost, so "✓ Worked · 12s" can follow ten minutes of work. |

Ways a busy agent crosses a pass boundary without ever returning to the user:
1. **A message queued while busy.** The pane holds it (`PendingMessageQueued`) and flushes it the moment the pass's idle push arrives. That is a new pass.
2. **A jekt delivered mid-pass.** Since 2026-09-28 it is written to stdin at once. Claude Code reads it at its next inference step, or answers it in a new pass right after the `result`.
3. **A background task finishing.** The CLI emits `system/task_notification` and starts a pass by itself.
4. **A deferred config restart** at the boundary, followed by queued input.

### 4.2 A gap found on the way: self-started passes are not marked active

srv sets `turn_active` only when it writes input (`queue.rs` `decide_send_action_from`, `input.rs` `mark_turn_active_locked_from`, `status.rs` `mark_turn_active_and_publish`) or spawns (`spawn.rs` ~L417). The stdout reader sets it false on `result`, but never sets it true. When the CLI starts a pass itself (case 3 above), srv reports idle for that whole pass:
- The Swarm row shows the agent idle while it works.
- `muxspect`/`GetAgentTranscript` report `turn_active: false`.
- No provenance is recorded for the pass.

The pane still shows Working, but only because `StreamFlushObserved` promotes `Done.completed` on live content (`reducer.ts` ~L350, FIX 2).

Found by reading the code; confirm live (§8, phase 1). The fix belongs to Part A, since the turn ledger needs to see these passes anyway.

### 4.3 Design: srv keeps a turn ledger

Add to `TurnActivityTracker` (`health.rs`), which every controller kind already shares (persistent, ACP, app-server):

```rust
pub struct TurnLedger {
    pub turn_id: u64,               // monotonic per block
    pub started_at_ms: u64,
    pub trigger: TurnTrigger,       // what started it (§5.2)
    pub inputs: Vec<TurnTrigger>,   // absorbed after the start, capped (say 20) with a count
    pub passes: u32,
    pub output_tokens: u64,         // summed over finished passes' result.usage.output_tokens
    pub cost_usd: f64,              // summed result.total_cost_usd
    pub steps: u32,                 // summed result.num_turns
    pub api_ms: u64,                // summed result.duration_api_ms
    pub ended_at_ms: Option<u64>,
    pub outcome: Option<TurnOutcome>, // completed | stopped | errored | exited
}
```

**Pass start** (a new idle→active flip):
- *Joins* the current ledger when any of these holds:
  - **J1:** the input came from srv's own queue: the backlog drained after a spawn or restart, or a `retry_after_resume_failure` flush;
  - **J2:** the input arrived *during* the previous pass, so the CLI is answering it now (a mid-pass jekt);
  - **J3:** the pane flushed a message it held while busy, tagged `joins_turn: <turn_id>` (new field on the send RPC). The pane sets the tag only for its held-message flush (`PendingMessageFlushStarted`), never for a fresh send;
  - **J4:** the CLI started the pass itself within **S = 2 s** of the previous `result`. Claude Code runs queued notifications straight after a query, so the real gap is milliseconds; 2 s is margin.
- *Otherwise* it opens a new ledger, whose trigger is the input's label or, for a CLI wake-up, `task` (§5.2).

**Pass end** (`result`):
- Add the pass's stats to the ledger.
- The ledger **stays open, marked settling**, for S. It closes when S passes with no new pass, or at once when nothing can join. "Nothing can join" means: no input arrived during the pass, srv's queue is empty, and the pane reported no held messages (it already knows, and sends `held: n` with its idle acknowledgement).

**Process exit or user Stop:** the ledger closes immediately with `exited` or `stopped`. A stop ends the turn, and queued input after a stop starts a new one.

**What doesn't keep a turn open:**
- **Background tasks still running.** They are the dock's job (§2.3a of the 09-09 report). Their later wake-up is a new external turn unless it lands within S.
- **Waiting on the user.** An AskUserQuestion or a permission prompt is inside the turn and keeps it open. The turn reports it as a needs-you state (§6).

**Publish:** add `turn: TurnLedger | null` to the controller status the pane and Swarm already subscribe to. srv publishes on pass start, pass end, absorbed input and close. The settle close is one extra publish.

**Self-started passes (fixes §4.2):** in the stdout reader, the first non-control line of a new pass while `!is_active_turn()` (`system/init`, or the first `assistant`/`stream_event`) calls a new `mark_turn_active_from_cli(trigger)`. The trigger is the last `task_notification` seen since the previous `result`, or `task: unknown`. That records provenance as `Automated`, re-arms the status heartbeat and publishes. It does not reopen a pass from a replaced process (same `turn_boundary_locked` generation check as the `result` path).

### 4.4 Design: the pane reads the turn, not the pass

Reducer:
- New `TurnObserved { turn }` stores `state.turn` (srv's ledger, verbatim).
- `turnTokens` keeps its per-pass meaning (the 10-07 counter logic is unchanged inside a pass).
- **Live counter** = `turn.output_tokens` (finished passes, exact) + `turnOutputTokens(turnTokens)` (this pass, high-water). Still monotonic: a new pass starts from the carried total, and the 10-07 high-water rule applies within the pass.
- **Elapsed** = `now − turn.started_at_ms`. This replaces the component-local `loadStartMs`, so the timer also survives remounts and pane-stack tab switches. If no ledger exists (an older srv, a pane mounted mid-turn before the first publish), fall back to today's `loadStartMs`.
- **Between passes of one turn** the working row stays up. While the ledger is open and settling, the row shows the last activity, dimmed. It does not show "✓ Worked", which reads as finished and then un-finishes 200 ms later.
- **This is not the input gate.** The busy predicate (`paneBusyForInput`) keeps its exact meaning. In the settle gap the composer is open, and a message typed then is a fresh send that starts a new turn, by D3. The 09-09 rule holds: the indicators are the input gate. Only the row's rendering waits out the settle.

**Worked line, on close:**
- `✓ Worked · 12m 4s · 41.2k tokens` gives turn wall time and turn output tokens.
- The secondary text is `$1.214 · 63 steps`, plus ` · 3 passes` when there was more than one pass.
- For an external turn, the verb carries the trigger (§5.3), for example `✓ Worked on AgentX's jekt · 2m 10s · 3.1k tokens`.
- Absorbed inputs show in the secondary text: `+1 jekt · +1 your message`.

**Hidden System passes:**
- A reinjection pass with no ledger open doesn't create a turn. It gets an internal ledger flagged `hidden`, which the row doesn't render.
- If its pass is followed within S by a real trigger, that trigger starts the visible turn.

### 4.5 Edge cases

| Case | Behaviour |
|---|---|
| The user stops mid-turn, then a queued message flushes | The turn closes as `stopped`. The flush starts a new user turn (J3 doesn't apply to a closed ledger). |
| A turn that spans a compaction | The same turn. A compaction is inside a pass, or between passes within S. |
| A config restart at the boundary | J1: the replacement's first pass drains srv's queue, so it joins. If nothing is queued, the turn closes. |
| A background task completes 3 minutes after the turn closed | A new external turn, trigger `task: <description>`. |
| Two jekts in quick succession while idle | The first opens an external turn. The second, written mid-pass, joins it (J2). |
| Pane remount mid-turn | Elapsed and the counter come from the ledger, so they are unaffected. |
| Providers without `result`-style usage (Codex, Gemini, ACP) | Same ledger; `output_tokens` stays 0 and the row shows elapsed time only, as today. |

### 4.6 Tests

- **health.rs:**
  - each join rule (J1–J4) joins;
  - a fresh user send after close opens a new ledger;
  - S expiry closes;
  - Stop and exit close at once;
  - provenance per pass is unchanged by joining (the self-quit tests keep passing untouched).
- **stdout_reader:** a self-started pass after `result` flips `turn_active` and publishes. A line from a replaced generation does not.
- **Reducer:**
  - the counter carries across passes and never drops;
  - elapsed reads `turn.started_at_ms`;
  - the settle gap renders no Worked line;
  - the busy predicate is unchanged in the settle gap.
- **AgentWorkingRow:** the multi-pass Worked line, `N steps`, `+1 jekt`.

### 4.7 As built (phase 1)

**Evidence first.** Claude Code 2.1.112 was run headless (`-p --input-format stream-json --output-format stream-json --verbose`) with one `run_in_background` Bash call. Its stdout, in order:

```
system/init → assistant (tool_use Bash) → system/task_started → user (tool_result) → assistant "started"
→ result (num_turns=2)
→ system/task_updated → system/task_notification        (8 s later, the task finished while idle)
→ system/init → assistant "done" → result (num_turns=1)
```

This settles three points:
- Every pass opens with `system/init`, including a pass the CLI starts by itself.
- A background task that finishes while the agent is idle reports *after* the `result`, so it is a new external turn and is not joined.
- `result.num_turns` counts model calls, which is why the Worked line now says "steps".

**What differs from §4.3/§4.4:**

| Design said | Built | Why |
|---|---|---|
| Add `turn` to the controller status | A separate persisted event, `agentturn` (`EVENT_AGENT_TURN`), published by `TurnActivityTracker` through a publisher hook | 13 call sites build controller status without the tracker. The ledger changes on its own schedule (stats, absorbed inputs). |
| The settle window applies after every pass, closed early when nothing can join | The turn ends at the `result` unless something is expected. A pass settles for `TURN_SETTLE_MS` (2 s) only when input reached the CLI during it, or a `task_notification` arrived during it. | Keeps "✓ Worked" instant for the common turn. srv cannot see the pane's held messages, and J3's tag covers those instead. |
| The pane reports `held: n` with its idle acknowledgement | Not built. J3 alone: the pane remembers which open turn a busy-path message was held during, and sends `joins_turn` on the flush (unless that turn was stopped). srv rejoins the turn up to `HELD_FLUSH_JOIN_MS` (10 s) after it ended. | One mechanism instead of two. |
| Self-started pass: gate on a `task_notification` | Gate on a `task_notification` **or** input written during the previous pass | A jekt written mid-pass can be answered by the CLI in a follow-up pass that srv never writes input for. Without this, that pass was idle to srv and the turn closed under it. Such a continuation keeps the previous pass's provenance (taint included), so the self-quit gate sees the same turn it saw. A wake-up from a task is `Automated`. |
| Live counter = ledger total + this pass | The same, de-duplicated: the ledger carries `counted_passes`, and `TokensIn` stamps `turnTokens` with the ledger's turn and pass. A pass's live tokens are added only while srv hasn't counted that pass. | srv counts a pass at its `result`, before the pane drops that pass's live tokens. Without the stamp, the figure doubled for that moment. |

**Where it lives:**
- **srv:**
  - `health.rs`: `TurnLedger`, `PassStats`, the join rules in `begin_pass`/`end_pass`, `mark_turn_active_from_cli`, `note_cli_task_notification`, and `ledger_publisher`.
  - `stdout_reader.rs`: the notification, the self-started `init`, and the pass stats.
  - `status.rs`: `cli_started_pass`.
  - `run_agent_turn_joining` and `CommandAgentInputData.joins_turn`.
- **Pane:**
  - `agent-pane-state/turn-ledger.ts`: parsing, `turnSettling`/`turnOpen`/`turnEndedAt`, `turnLiveOutput`.
  - The `TurnObserved` reducer arm and the `TokensIn` stamp.
  - `hooks/useTurnLedger.ts`.
  - `AgentWorkingRow`: clock and counter from the turn, the `is-settling` row, the turn's Worked line with `N steps` and `N passes`.
  - The `joinsTurn` on held messages in `useAgentCommands.ts`.

**Not in phase 1:**
- The `+1 jekt` inputs breakdown, which needs §5.2's triggers.

**Tests against a real process:** `persistent/tests/turn_ledger.rs` runs the controller and its stdout reader on a node stub that replays the recorded line order. It covers three cases:
- a background-task wake-up is a busy, automated turn of its own;
- a jekt answered in its own pass joins the turn, giving two passes, both counted;
- an `init` at spawn is not a pass.


---

## 5. Part B — knowing when something other than you started a turn

### 5.1 Recommended practice

How mature systems handle the same problem (CI's "triggered by", chat apps' bot and system messages, mail and chat unread versus mention):

1. **Provenance at the boundary.** Every turn shows what started it, at the place it starts, in the transcript. Logs and tooltips don't count.
2. **The user's voice stays the user's.** An external trigger never looks like the user's own message bubble. (Jekts already have `JektBubble`; a task wake-up and a cron fire need the same treatment.)
3. **Attention follows need, not activity.** An external turn that finishes quietly marks the pane *unread*. It interrupts (OS notification, sound) only when it ends needing the user: a question, an approval, a failure, or a message explicitly addressed to them. That is the difference between unread and a mention.
4. **Digest on return.** When the user comes back to a pane that ran external turns unattended, give one line: "While you were away: 3 turns: 2 jekts (AgentX, github-consumer), 1 background task." Don't make them scroll to find out.
5. **User-controlled policy per trigger kind.** For example: github reviews badge only, a supervisor nudge is silent, a cron fire badges.
6. **Never borrow authority.** An external turn can't do what only a user turn may do. This is already enforced for self-quit.

### 5.2 Trigger taxonomy

`TurnTrigger` is display data next to the existing `TurnOrigin`. It is set on the same delivery paths that already pass a `TurnOrigin`.

| `kind` | Origin class | Label (example) | Set at |
|---|---|---|---|
| `user` | User | "your message" | `server/agent_handlers/input.rs` (pane send) |
| `broadcast` | User | "your Swarm broadcast" | `app_api/fleet.rs` |
| `agent` | Automated | "jekt from AgentX" (+ `TRUST`) | reactive delivery (`bootstrap/delivery.rs`), from the jekt marker |
| `service` | Automated | "github-consumer: PR #4477 reviewed" | the same path, when the sender is a service rather than an agent |
| `schedule` | Automated | "cron 'nightly-check'", "loop", "supervisor nudge" | cron, loop and supervisor delivery |
| `task` | Automated | "background task finished: npm test" | stdout reader, from the `task_notification` (§4.3) |
| `system` | System | (hidden) | memory reinjection, side question |

The label is built by srv from data it already holds: the jekt marker's `FROM`, the cron job name, the task's description from the task feed. It never comes from message body text.

### 5.3 Where it shows

| Surface | User turn | External turn |
|---|---|---|
| Transcript | unchanged (your bubble) | a slim **trigger header** above the turn's first node: icon + label + time, e.g. `⚡ jekt from AgentX · 14:02`. A jekt keeps its bubble; the header goes on task, schedule and service turns that have no bubble of their own |
| Working row, first ~3 s | normal | lead-in `↳ AgentX's jekt`, then normal status (§6) |
| Worked line | `✓ Worked · …` | `✓ Worked on AgentX's jekt · …` |
| Tab, pane header, Swarm row | as today | an **unread** dot after an external turn finishes while the pane isn't focused. It clears on focus |
| OS notification / sound | as today | only when the turn ends needing the user (question, approval, failure). This uses the existing notification router's families (`backend/notify`), with a per-kind policy (§5.1 item 5) |
| Return digest | — | on focusing a pane that ran external turns unattended: one line above the composer, dismissable. Built deterministically from the ledgers; Haiku is optional, for a one-sentence "what changed" (§9, Q4) |

---

## 6. Part C — a live status that says what is happening

### 6.1 Today

The left zone, first match wins (`loadingLeftText`, `AgentFooter.tsx` ~L152):
- reconnecting, compacting, stopping or rate-limited;
- a launch phase;
- the **ambient summary** (`readSwarmSummary`: a Haiku-written, PR-title-style phrase for the session's overall goal, re-evaluated per user message);
- a phrase cycling every 30 s.

It answers "what is this session about". It never answers "what is it doing now", "is that normal", or "do you need me".

### 6.2 Principle: use the words the model already wrote

Claude Code's tool inputs carry human phrasing the model writes for exactly this purpose. Using it is free, instant, accurate and in the agent's voice. Verify each field against the pinned CLI version, as the 10-07 spec did for the counter.

| Source | Field | Example |
|---|---|---|
| Bash | `description` | "Run the srv test suite" |
| Agent / Task (subagent) | `description`, `subagent_type` | "Explore: find turn-origin code" |
| Todo list (`TodoWrite`, or the newer task-list tools) | the `in_progress` item's `activeForm`, plus position | "Writing the spec (3/7)" |
| WebSearch / WebFetch | `query`, URL host | "Searching: claude code stream-json result fields" |
| Read / Edit / Write | `file_path` (basename) | "Editing AgentFooter.tsx" |
| Grep / Glob | `pattern` | "Searching for turn_active" |
| thinking | the first sentence of the current thinking block, when the stream carries text | "Thinking: whether the ledger can join a queued flush" |
| AskUserQuestion / permission `control_request` | the question, the tool and command | "Waiting for your approval: rm -rf build" |

Claude Code's own spinner already prefers the in-progress todo's `activeForm` over a random verb. This spec generalizes that.

Haiku keeps one job, the **goal** (today's summary). It doesn't narrate steps. Per-step Haiku would add cost and latency to say worse what the model already said. The left-open question is §9, Q3.

### 6.3 The state machine

The machine is a statechart: one activity region plus orthogonal regions. It lives in the pane reducer, next to the signals it consumes, as `state.activity`. Most inputs are already reducer commands.

```
Turn ── Idle
     ├─ Active
     │   ├─ [Activity]   Starting ─▶ Requesting ─▶ Thinking ─┐
     │   │                    ▲            │                  ├─▶ Composing(tool) ─▶ Running(tools…) ─┐
     │   │                    │            └─▶ Writing ───────┘                                        │
     │   │                    └─────────────────────────── tool result (↑) ─────────────────────────────┘
     │   │               Delegating(subagents)      — an Agent/Task tool is running
     │   │               NeedsYou(question | approval | login)   — preempts all, clock keeps running
     │   │               Held(compacting | rate_limited{retryAt} | reconnecting)
     │   │               Stopping
     │   ├─ [Health]     Normal | Slow(reason) | Quiet(since)   — orthogonal
     │   ├─ [Plan]       None | Todo{index,total,activeForm}     — orthogonal
     │   └─ [Inbox]      queued: n (held user msgs + absorbed-pending inputs)
     ├─ Settling   (§4.4: between passes of one turn)
     └─ Ended(outcome)
```

**Inputs.** Existing commands and their new uses:

| Signal | Source today | New use |
|---|---|---|
| `TurnStart`, `TurnObserved` | reducer / §4.4 | Starting, trigger lead-in |
| `RequestStarted`, `TokensIn` (`message_start`) | `useAgentStream` → reducer (10-07) | Requesting ⇄ streaming. **Time to first token** = `message_start` − `RequestStarted` |
| `OutputStreamed { chars }` | 10-07 | Add `kind: text \| thinking \| tool_input`: Writing / Thinking / Composing |
| `ToolStart` / `ToolEnd` (`currentTool`, `currentToolArg`, `toolsActive`) | reducer | Running(tools). Add the tool's input object to `ToolStart` so §6.2's fields are readable |
| Subagent lines (`parent_tool_use_id`) | ignored by the counter | Delegating detail: the subagent's own current step, at most one level deep |
| Control request (`can_use_tool`, AskUserQuestion) | `useAgentQuestions` | NeedsYou |
| `compacting`, `waitingReason`/`retryAfterMs`, resume retry, launch phase | reducer / props | Held, Stopping (unchanged texts) |
| Todo tool input | not parsed | Plan region |
| Attached tasks / dock | reducer axis | Not in the row (dock's job). They feed the Worked line ("2 tasks still running") |
| Ambient summary | block meta | Goal fallback |

### 6.4 The presenter: the right line at the right time

`presentStatus(activity, now, memory) → { text, detail?, tone, revealKind }` is a pure function in a new `frontend/app/view/agent/status/present-status.ts`. It holds a small presenter memory (what is shown and since when) so dwell can be enforced without timers in the reducer.

**Salience order** (first eligible wins):

| Rank | Class | Eligible when | Example | Tone |
|---|---|---|---|---|
| P0 | Needs you | NeedsYou | `Waiting for your approval: cargo publish` | `warn` (pane accent → warning color) |
| P1 | Held / stopping | Held, Stopping | `Rate limited, retrying in 12s` · `Compacting… 41s / ~1m` | today's |
| P2 | Anomaly | Health ≠ Normal | `Waiting on Claude · 34s (slow today)` · `cargo build · no output for 2m` | `muted-warn` |
| P3 | Trigger lead-in | first 3 s of an external turn | `↳ AgentX's jekt` | normal |
| P4 | Now | an activity that has lasted ≥ its promote threshold | `Running the srv test suite · 1m 12s` · `Explore agent: find turn-origin code` | normal |
| P5 | Plan | Plan ≠ None | `Writing the spec (3/7)` | normal |
| P6 | Goal | ambient summary exists | `Fix the login redirect loop` | normal |
| P7 | Phrase | always | `Working…` | normal |

**Timing rules.** These carry most of the "right time":
- **Promote thresholds.** An activity becomes eligible for P4 only after it has lasted: tools 1.5 s, Composing 2 s, Thinking 4 s, Requesting never (it surfaces through P2 if slow). Bursts of fast reads never reach the screen, which is the reason the old `tool · arg` row felt noisy.
- **Aggregation.** Same-class activities inside one step fold: "Reading 6 files", "Searching the codebase (4)", "3 tools running".
- **Dwell.** A shown line stays at least 1.2 s before a same-or-lower rank replaces it. P0–P2 preempt immediately.
- **Hold after end.** When a P4 activity ends, its line stays up to 2 s or until the next eligible line, then falls back to P5/P6. This avoids flashing back to the goal between two tool calls.
- **Anomaly thresholds.** These are starting values, tuned from `muxlog phases` data (§7):
  - Slow: time to first token over 20 s, or over 2× this pane's rolling median.
  - Quiet: a tool with no output for 60 s, for tools that stream output (Bash); a subagent with no step for 90 s.
- **Reveal.** The type-out runs only when the *class* changes (P6→P4, P4→P0). A change within P4 swaps instantly, and counters and elapsed time tick without a reveal. This also settles `SPEC_AGENT_WORKING_ROW_TOOL_BURST_REVEAL_INTERRUPT_2026_08_21.md`.

**Layout.** The left zone shows the primary text. When the primary is P2–P5, the goal (P6) follows as a muted segment ` · Fix the login redirect loop` while width allows, and it is the first thing truncated. The right zone shows today's `↓ 2.4k tokens · 12m 4s`, plus ` · 1 queued` when Inbox > 0.

### 6.5 What the user sees across one turn

| t | State | Row (left · right) |
|---|---|---|
| 0.0 s | Starting (user turn) | `Fix the login redirect loop` · `0s` |
| 0.4 s | Requesting | (unchanged: Requesting is never promoted) |
| 2 s | Thinking, under its 4 s threshold | (unchanged) |
| 6 s | Thinking past 4 s | `Thinking: where the redirect is issued · Fix the login…` · `↓ 310 tokens · 6s` |
| 9 s | 6 quick Reads + 2 Greps | (each under 1.5 s: folded, then shown once) `Reading 6 files` |
| 14 s | TodoWrite sets item 2 in progress | `Tracing the session cookie (2/5)` |
| 20 s | Bash `npm test` running | `Running the auth tests · 6s` |
| 1m 30s | Bash quiet 60 s | `Running the auth tests · no output for 1m` (P2) |
| 2m 05s | permission prompt | `Waiting for your approval: git push` (P0, warn) |
| 2m 40s | result; a jekt was written mid-pass | Settling: the last line dimmed, no "Worked" |
| 2m 40.1s | next pass (J2) | `Fix the login redirect loop` · `↓ 4.2k tokens · 2m 40s` (carried) |
| 3m 12s | end, nothing queued | `✓ Worked · 3m 12s · 5.0k tokens` · `$0.38 · 22 steps · 2 passes · +1 jekt` |

### 6.6 What it deliberately doesn't do

- **No raw `Tool · arg`** (the 10-02 decision). Tool facts appear only through the model's own description or a plain verb + basename, and only past a threshold.
- **No per-step Haiku calls.**
- **No background-task status in the row.** That is the dock's job, and lighting the row for it would contradict the input-gate meaning.
- **No change to what "busy" means for the composer gate.**
- **No content from subagents beyond one line.** The document view has the rest.

### 6.7 Architecture placement

- **srv:**
  - the turn ledger and `TurnTrigger` (§4.3, §5.2);
  - the self-started-pass fix;
  - `joins_turn` / `held` on the send and idle-ack paths.

  No srv involvement in the activity machine: everything it needs already streams to the pane.
- **Frontend:**
  - `agent-pane-state`: `turn` (`TurnObserved`) and `activity`;
  - the new and extended commands: `OutputStreamed.kind`, `ToolStart.input`, `PlanUpdated`, `NeedsYouChanged`, `SubagentStep`.

  These are dispatched from `useAgentStream` where the lines are already parsed. The reducer stays the one owner of turn state (`turn-confirmation.ts`'s rule).
- **`status/present-status.ts`:** pure, with a fake clock in tests.
- **`useStatusLine`:** a small hook that ticks the presenter (it reuses `useTick`).
- **`AgentWorkingRow`:** it renders `{text, detail, tone}` and keeps its reveal, dot, compaction bar and right zone. `loadingLeftText` is replaced by the presenter.
- **Swarm and muxspect:** get the turn ledger free from the status publish. Swarm can show the external-trigger chip and the unread dot from it.

---

## 7. Verification

- **Unit tests:** as listed in §4.6.
- **Presenter:** table-driven tests over (activity snapshot, clock) → expected line, covering every rank, promote threshold, dwell, aggregation and hold-after-end.
- **Replay tests:** feed recorded `stream-json` transcripts through `useAgentStream`'s parse → reducer → presenter at recorded timestamps, and snapshot the line sequence. Cover:
  - a long single-pass turn;
  - a multi-pass turn with a queued message;
  - a mid-pass jekt;
  - a background-task wake-up within and beyond S;
  - a permission prompt;
  - a rate limit;
  - a compaction.
- **Live:** in an isolated `task dev` build (not the operator's instance):
  - reproduce each multi-pass case and confirm the timer and counter never reset and the Worked line reports the turn;
  - confirm the Swarm row stays busy through a self-started pass (§4.2).
- **`muxlog phases`:** log ledger open, join (with the rule), settle and close events next to the existing `[wave-turn]`/`[health]` lines, so a "why did my timer reset" question has a direct answer. Use the same data to tune §6.4's thresholds.

## 8. Phases

1. **Turn ledger (A).**
   - srv: the ledger, join rules, the publish, the self-started-pass fix, the `joins_turn` tag.
   - Pane: `TurnObserved`, elapsed time and counter from the ledger, the settle rendering, and the Worked line with `steps`/`passes`.

   This alone fixes the reported reset.
2. **Triggers (B).**
   - `TurnTrigger` on every delivery path.
   - The transcript trigger header, the row lead-in, the Worked-line verb, unread dots.
   - Notification policy per kind.
   - The return digest, deterministic.
3. **Activity machine and presenter (C).**
   - The reducer `activity` region, with deterministic sources only (§6.2 minus thinking).
   - The presenter with the timing rules.
   - Replay tests.
4. **Enrichments.**
   - The thinking headline.
   - Subagent step detail.
   - Test-runner progress extraction from Bash output, for example `41/120 passed`, for known runners (vitest, cargo test, jest), behind a small extractor registry.
   - Per-pane threshold learning (the rolling median).

Each phase ships with its own changeset and tests, and this spec goes in phase 1's PR.

## 9. Open questions

1. **Rename `turn` → `pass` in code?** Recommend a later mechanical refactor once the glossary has settled. It touches several hundred identifiers and log lines (`[wave-turn]`) that `muxlog phases` parses.
2. **S = 2 s.** Is a fixed window right, or should J4 also require that the `task_notification` was emitted *before* the `result` (the CLI had it queued)? The latter is exact, if the CLI's line order guarantees it. Confirm from a recording.
3. **Haiku for steps.** Kept out. Revisit only if §6.2's sources leave visible gaps, for example long thinking with no text on a model that redacts it.
4. **The return digest:** deterministic only, or one Haiku sentence on top ("merged #4477, waiting on CI for #4478")? Recommend deterministic first.
5. **Should the clock pause while NeedsYou?** Recommend no: wall time is what the user lived. The Worked line can append `(1m 10s waiting for you)` when that time was over 10 s.
