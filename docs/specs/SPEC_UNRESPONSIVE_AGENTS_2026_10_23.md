# SPEC: unresponsive agents — out-of-quota and signed-out states, the reset time, held messages, the per-account notice

**Date:** 2026-10-23
**Status:** active — approved by the operator on 2026-10-23. Step 1 (§3, §4.1, §4.2, §4.5) shipped in #5389; step 2b (§4.4 and the R8 display) is in #5398; step 2d (§6) in #5402; step 2a (§4.3 and R5) in #5410; step 2c (§5) in #5419; step 3a (§7, the wake) in #5433; step 2e (§4.6) and step 3b (§8, the watchdog) remain. Implements R1–R4, R6 and R7 of `docs/reports/REPORT_UNRESPONSIVE_AGENTS_2026_10_23.md` (Kanban #1255, operator: "yes proceed with all"); R5 and R8 are scoped here only as far as they read this spec's state. Ships with the first build PR (§9, step 1).
**Author:** Loap-3 (loap-loap-2-loap-3). Build order and owners agreed with AgentX and AgentY.
**Task:** Kanban #1787.

---

## 1. Problem

An agent whose account is out of quota (or whose sign-in was refused) is alive but can't work. AgentMux classifies the failure (`agents/failure.rs`, `UsageLimit` / `Auth`) and stores it on the pane as `agent:last_failure`, but only the pane, one toast, and `Agents spawn`'s account choice read it. Everything that tracks work sees an idle agent:

- srv's `AgentState` is `idle`, so the Kanban card stays In progress and other agents and leaders see nothing wrong.
- The lease keeps renewing, because the process runs.
- A message to it is answered "Delivered", and its turn fails in under a second.
- The reset time in the provider's message is thrown away.

On 2026-10-23 Naki-2 held two tasks this way for 6 h 12 m. This is the most common failure on this machine: about 70 limit hits in two weeks (report §2).

## 2. Goals and non-goals

**Goals**
1. One definition, used everywhere, of when an agent is **limited** (out of quota) or **signed_out**, with the reset time when it's known.
2. Messages to such an agent are held, not lost, and delivered when it can work again.
3. The operator gets **one** notice per account episode, with a one-click move to another signed-in account. The task's leader gets one note.
4. A limited agent wakes on its own when its limit resets.
5. The watchdog stops killing silently.

**Non-goals**
- Detecting hung turns from silence. The August removal stands (`SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md`); R8's long-tool display is information only.
- Automatic account moves. An opt-in standing rule is a later spec; moves never cross providers.
- Codex classification gaps (R9). Codex gets these states only through what `classify()` already sees from `exec`'s exit and stderr.
- Remote agents' failures (LAN or cloud): §4.5 says what a peer sees.

## 3. Definitions

### 3.1 The two states

`AgentState` (`crates/srv/src/backend/agent_state.rs`) gains two variants:

| Variant | Wire | Meaning |
|---|---|---|
| `Limited` | `"limited"` | The pane's last run ended on the plan's usage limit (`FailureClass::UsageLimit`), and the reset, if known, hasn't passed |
| `SignedOut` | `"signed_out"` | The pane's last run ended on refused credentials or a spawn-gate "no account" (`FailureClass::Auth`) |

`signed_out` needs an explicit `#[serde(rename = "signed_out")]`, because the enum is `rename_all = "lowercase"`.

### 3.2 The rule

`decide()` takes two new inputs:
- `failure: Option<(FailureClass, Option<u64>)>`, the class and `resets_at_ms` read from the block's `agent:last_failure`;
- `now_ms`.

The order of precedence:

1. **A turn is active:** Working or Waiting, as today. A turn that is running is evidence the agent can work.
2. **The failure is `UsageLimit`** and `resets_at_ms` is `None` or still in the future: **Limited**, whatever the process state (running, init, or done). Anything started on that account fails the same way.
3. **The failure is `Auth`:** **SignedOut**, whatever the process state.
4. **Otherwise:** today's rule (Idle, Error, Stopped).

Other failure classes (`RateLimited`, `Overloaded`, `Network`, …) are transient and don't change the state.

`current_state()` already reads the block, so the only extra cost is parsing one meta key.

### 3.3 What ends the state

The state is derived, so it ends when its input does:
- a clean turn clears `agent:last_failure` (already true: `stdout_reader.rs:702`, `host_spawn.rs:726`);
- the reset passes (rule 2 stops applying);
- an account rebind clears it (§6.2);
- for `signed_out`, a sign-in or validate that proves the same account works clears its `auth` failures (`account_episodes::after_sign_in`, #2237); a token refresh doesn't;
- the wake at the reset clears it (§7).

### 3.4 `AgentFailure.resets_at_ms` (R2)

`AgentFailure` (`agents/failure.rs`) gains two optional fields:
- `resets_at_ms: Option<u64>` (camelCase `resetsAtMs` on the wire);
- `limit_window: Option<String>`, the provider's name for the window, such as `"five_hour"` or `"seven_day"`.

Both are skipped when `None`, so the stored meta stays compatible both ways.

Where `resets_at_ms` comes from, in order:
1. **Claude's `rate_limit_event`.** srv currently discards it (`agents/translator/claude.rs:51`). The persistent controller's stdout reader keeps the **latest** `rate_limit_info` per block: `status`, `resetsAt` (unix seconds) and `rateLimitType`. Claude Code sends one with every request, with status `allowed`, `allowed_warning` or `rejected` (see `claude-translator.ts:236-249` and its tests). When `classify()` returns `UsageLimit`, use the latest one whose `status` is exactly `"rejected"`. Don't go by `overageStatus`, which is often `"rejected"` while `status` is `"allowed"`.
   - Its top-level `resetsAt` and `rateLimitType` are the exhausted window's. The event's `unifiedWindows` (`five_hour`, `seven_day`, …: `{utilization, resetsAt}`) also gives every window's use, which a later warning could read.
   - Manoz checked this against 65 real usage-limit errors in stored pane streams: a rejected event with `resetsAt` came at most 30 stream lines before the error for 19 of 20 weekly limits, 21 of 21 session limits and 22 of 24 monthly spend limits.
   - The 14 "temporarily limiting requests (not your usage limit)" errors had a rejected event with no `resetsAt`. They are `RateLimited`, not `UsageLimit`.
   - `classify()` gains "spend limit" (`You've hit your monthly spend limit · …`), which it didn't match before. It is the same condition: the agent can't work until the window resets.
2. **The message text**, as a fallback, e.g. `resets Oct 26, 7pm (America/Los_Angeles)` or `resets 7pm (…)`:
   - parse it only when the named zone is the machine's local zone (`chrono::Local`), because srv has no tz database and this spec doesn't add one;
   - a time with no date is the next occurrence; a date with no year is the next occurrence of that date.
3. **Codex:** `resetsAt` from its rate-limit snapshot, when R9 classifies Codex events. Until then, none.

A value is kept only if it lies in `(now, now + 8 days]`; otherwise it is `None`. A `None` reset still means Limited, just with no time shown.

### 3.5 The read helper

One function, so no consumer re-derives the rule:

```rust
// agent_state.rs
pub struct Unavailable { pub state: AgentState, pub code: FailureClass, pub title: String,
                         pub resets_at_ms: Option<u64>, pub since_ms: u64, pub account_id: Option<String> }
pub fn unavailable_of_block(mstore: &Store, block_id: &str, now_ms: u64) -> Option<Unavailable>
```

- It returns `Some` only for Limited and SignedOut.
- A pane has no account field of its own. `account_id` is the account its run launched on: block meta `agent:run_account` (`RUN_ACCOUNT_META_KEY`, `server/app_api/agent_spawn.rs`), and for a pane from before that key, its agent's current identity link.
- That rule moves out of `account_blocks` into one shared helper, `run_accounts_of_block(state, block) -> Vec<String>`. `account_blocks`, `unavailable_of_block` and the episodes (§6.1) all use it, so they can't disagree.

### 3.6 The hold reason (R3)

`HoldReason` (`server/reactive.rs:1299`) gains:
- `Limited`, wire `"usage_limit"`;
- `SignedOut`, wire `"needs_login"`, which reuses the existing reason. To the sender these are the same condition as a spawn-gate refusal.

## 4. Who reads the state

### 4.1 srv consumers

These come right with no code change, because they compare against `Idle`:
- the Kanban reminder loop (`backend/kanban.rs:157`): a limited agent isn't nudged;
- `Agents spawn`'s worker states.

Each of the following has to be checked, and is listed in the step-1 PR:
- `fleet_source.rs` and `fleet_feed.rs`: they carry the new values.
- `lan_discovery/agent_names.rs:106`: an **older** peer's parser drops an unknown state (`.ok()?`), so it shows no status for that agent. That is acceptable, and the same as "unknown".
- `swarm_remote.rs`: carries the new values.
- `agent_state::SinceTracker`: a state change restarts `since_ms`, as today.

### 4.2 MuxAPI (step 1)

- `Agents conversations` (`server/muxspect_handlers.rs:1039-1136`) and `Agents workers` (`server/app_api/agent_spawn.rs:527-597`) gain:
  - `state`, the `AgentState` string; `workers` already has it;
  - `unavailable`: `{state, code, title, resets_at_ms, since_ms}` or `null`.
  - The account id is **not** included: agents on other hosts receive these answers.
- The tool descriptions in `crates/mcp/src/tool_schemas.rs` say what `limited` and `signed_out` mean, and that a message to such an agent is held.

### 4.3 Kanban (step 2, Naki-2)

- `kanban.list`'s `holder` carries the new states, plus `holder_unavailable` from `unavailable_of_block`.
- `needsReason` (`frontend/app/view/warden-kanban/kanban-columns.ts`):
  - `limited` reads "out of quota until Sun 7:00 PM", or "out of quota" with no time;
  - `signed_out` reads "needs sign-in";
  - either one puts the card in Needs you and counts it in the badge.
- **As built (step 2a, Agent1):**
  - `kanban.list` carries `holder_unavailable` (`state`, `code`, `title`, `resets_at_ms`, `since_ms`; never the account) beside the holder's `limited` or `signed_out` state.
  - The Needs-you badge counts a task held by a limited or signed-out agent only when no open account episode (§6) covers that holder (`agent_ids` or `block_ids`); the episode is counted once, by the caller. The card keeps its reason either way. Every other reason counts as before.
  - **Release** and **Reassign** (`kanban.act` `release`, `reassign`) take a claimed or blocked task back: it waits again, its claim and question gone, and with Reassign it is for the chosen agent or anyone, in the same step. The attempt isn't spent (the task's limit grows by one), and `attempts` stays as the fence: the next claim is a new number, and the old holder's late calls find the task no longer theirs. The old holder gets a system note. A task with a branch or PR link is not taken back until the operator confirms (`confirm: true`), because that work stays where it is.
  - **The lease** of a limited or signed-out holder stops being renewed only on a task open to anyone or a group, after 30 minutes of it, and with the reset more than an hour away or not known (`renews_lease` in `server/work_tasks.rs`). A task aimed at an agent stays with it.

### 4.4 Swarm (step 2, Camper)

- A chip on the agent's row: "Out of quota · until Sun 7 PM" or "Signed out", ranked with needs-you in `rankAgentStatus` (`frontend/app/store/agent-status.ts`).
- The status line (`frontend/app/store/swarm-line.ts`) gives limited and signed_out a line instead of dropping them.
- R8's "running <tool> for N min" goes in the same PR.

### 4.5 Supervisor (step 1)

`backend/supervisor/policy.rs` `on_turn_end`: if the pane is Limited or SignedOut, **Pause** with "out of quota until …" or "needs sign-in", without calling the judge. The judge runs on the same account and would fail anyway.

### 4.6 Mobile and peers

- `agentmux-mobile` reads `agent_status` from the fleet feed. QTest checks that it shows nothing for an unknown state rather than failing, then adds the two values.
- An older desktop peer shows no status for these agents (§4.1).

## 5. Held messages (R3, step 2, AgentY)

**Hold.** In the local fast path (`server/reactive.rs`, before `inject_here`):
1. Resolve the target's block. If `unavailable_of_block` is `Some`, hold the message with `HoldReason::Limited` or `SignedOut` through `hold_for_absent_target`. It is not delivered.
2. The sender's answer (`crates/mcp/src/tool_helpers.rs`):
   - `HELD for X (usage_limit, until Sun 7:00 PM) — X's account is out of quota; delivered when it can work again (an account move or the reset), for up to 24 h. Do not resend it.`
   - With no reset time known: "… (usage_limit) …".
3. The same `hold_applies` conditions as today: a full-key host-tier sender, not cron.

**Not held:**
- the operator's own input in the pane, which runs a turn and shows the failure where they can see it;
- AgentMux's own `[AgentMux]` notes, which already wait for the next turn.

A Swarm broadcast that goes through the inject path is held like any jekt, because the operator's words would otherwise be lost the same way. AgentY confirms the path in step 2c.

**As built (step 2c, AgentY):**
- The check runs in `reactive::deliver` before the local fast path (and before attachments are charged), and in the file receiver: a target registered here that is `limited` with a known reset is held (`usage_limit`), with `held_until_ms` and `held_until` ("Sun 7:00 PM") in the answer.
- **`signed_out` is not held yet.** That state lasts until a clean turn, and a sign-in to the same account doesn't clear it, so holding every message would keep that turn from ever running (muxreview P1 on #5419). It joins once a confirmed sign-in clears the failure (Kanban #2237); until then a message to a signed-out agent is delivered as before. The same goes for a usage limit with no known reset time: nothing but a turn or an account move ends it.
- Where the hold doesn't apply (a LAN or cross-channel hop, a file transfer, a cron fire), the message is **refused**, not injected: the error begins `agent unavailable: ` (`reactive::UNAVAILABLE_PREFIX`), and the Message tool answers "Message delivery failed (unavailable)". The jobs' pending fires retry it like a spawn-gate refusal.
- **A Swarm broadcast doesn't go through the inject path**: it starts a turn directly (`deliver_broadcast_turn`). An unavailable target is therefore not held but refused in the broadcast's result ("agent unavailable: Out of quota until Sun 7:00 PM: not sent"), which the operator, its sender, sees. Holding it would replay the operator's words as a jekt, with another sender and trust.
- The replay skips a target whose block is still unavailable, counting no attempt. The account move, rebind and episode close already wake it (`held_jekt_wake`, step 2d); the reset is picked up by the next 30 s pass until step 3's wake.
- The note to the requester or leader (§6.2) was built in step 2d.

**Replay** (`server/jekt_held.rs`):
- A target whose block is still unavailable is skipped. That is a cheap read each 30 s pass, with no gate check.
- When it becomes available (account move, reset, or wake), the next pass delivers in order. `held_jekt_wake()` is notified on rebind and on the wake (§6.2, §7), so the wait is seconds, not 30 s.

**TTL.** The 24 h hold limit is unchanged. A weekly reset can be days away, and an account move is the usual recovery, so the answer always states the 24 h limit. Expiry is still not reported to the sender; that is mailbox work (G9).

## 6. The per-account notice and the account move (R4, step 2, AgentX)

### 6.1 Episodes

An **account episode** opens when the first block on account A becomes Limited or SignedOut. It closes when no block on A is. A block is on A when `run_accounts_of_block` (§3.5) contains A, the same rule `account_blocks` uses.

srv keeps the episodes in memory, rebuilt on start from blocks' meta, and publishes `accountavailability` events. The payload:

```
{ account_id, provider, state: "limited"|"signed_out"|"ok", resets_at_ms,
  agents: [{block_id, agent_name}], tasks: [{number, title}] }
```

- `tasks` are the claimed or blocked Kanban tasks those agents hold.
- An RPC `identity.account.availability` lists the open episodes for the UI.

### 6.2 Telling people

- **The operator:** one OS toast when an episode opens, from a new `NotifyKind::AccountUnavailable` in the attention family.
  - Body: "4 agents are out of quota until Sun 7:00 PM", or "2 agents need you to sign in again".
  - It **inhibits** the per-block `AgentCrashed` toasts for `usage_limit` and `auth` on that account for the rest of the episode (`backend/notify/router.rs:386-399`).
  - No account label or email goes in a toast body. The in-app item shows the account's label, since it stays local.
- **A persistent item** while the episode is open, placed by AgentX: a strip at the top of Warden ▸ Kanban, and the Swarm header. It reads "Account <label>: out of quota until Sun 7:00 PM · Naki-2, Loap-3, QTest, Agent5 · holding #163, #140", with the actions below. It counts once in the Needs you badge.
- **The leader:** one `[AgentMux]` note per episode to the requester (`requested_by`) of each task the agents hold, and to a worker's leader (`agent:worker_of`, already done by `backend/worker_notes.rs`). That becomes the seat lead once seats are data (#1102). For example: "Naki-2 is out of quota until Sun 7:00 PM and holds #163. Its messages are held."

### 6.3 Moving the agents

The action is **Move these agents to another signed-in account**, done by a new srv RPC `identity.account.move_agents {from, to, block_ids}`, so it is one audited action.
- **Candidates:** accounts of the same **canonical** provider, status valid, not in `account_blocks`, and not `from`. The picker shows each candidate's last known status.
- **Steps per agent:** the ones `bindAccountToAgent` does client-side today (`frontend/app/view/identity/bind-to-agent-menu.ts:146-189`), moved into srv.
  1. Unlink the agent's alias-stored rows for the same canonical provider, or a stale alias link silently wins.
  2. Link `to`.
  3. `ControllerResync{forcerestart:true}`, which carries the continuation record.
- **Only idle agents move.** One mid-turn is listed as "will move when its turn ends".

**On rebind** (any account link change for the agent's provider):
1. Clear `agent:last_failure` (`usage_limit` or `auth`) only on that agent's panes whose `run_accounts_of_block` doesn't contain the new account.
2. Publish the change.
3. Call `held_jekt_wake()`.

**`account_blocks`** (`agent_spawn.rs:406-450`): skip a `UsageLimit` block whose `resets_at_ms` has passed, so a worker can use the account again after the reset even if the pane is still open.

## 7. Wake at the reset (R6, step 3, Manoz)

- **Scheduling.** For each Limited block with a `resets_at_ms`, srv schedules a wake at `resets_at_ms` + 60 s + a random 0–120 s. On start, it rebuilds the schedule from block meta; one already past runs at once.
- **The wake**, if the block is still Limited for the same `resets_at_ms`:
  1. clear `agent:last_failure`;
  2. notify `held_jekt_wake()`;
  3. if no held message is waiting for it and it holds a claimed Kanban task, give it one `[AgentMux]` note: "Your account's usage limit has reset. Continue with #163."
- **Limits.** One wake per `resets_at_ms` value. A limit hit again records a new failure and a new reset.
- **Verified: it doesn't.** In 2,563 usage-limit errors in stored Claude Code transcripts, none was followed by the agent carrying on without a new message from a person or AgentMux. So the wake is needed, all three steps stay.
- **Where.** `backend/limit_wake.rs`. `persist_last_failure` schedules the wake when it stores a usage limit with a reset; `install` rebuilds the schedule from block meta at start. The held-message check reads the held-jekt table by the agent's uid; the claimed tasks are the agent's claimed Kanban tasks (up to 3 named, titles quoted).

## 8. The watchdog (R7, step 3, Manoz)

When `backend/blockcontroller/watchdog.rs` trips and calls `stop_now`:
- release the block's claimed tasks with the reason "stopped by the watchdog after <limit>" (the attempt counts);
- raise one `AgentCrashed` notification: "Stopped by the watchdog after 4 h";
- fix the table comment at `sagas/agent_teardown.rs:16`.

## 9. Build order and owners

| Step | Owner | What | Depends on |
|---|---|---|---|
| **1** | **Manoz** (Area54) | §3 (states, rule, `resets_at_ms`, `limit_window`, `unavailable_of_block`, `HoldReason` variants defined), §4.1, §4.2, §4.5. **Ships this spec.** | — |
| 2a | Naki-2 | §4.3 (Kanban reason) and R5 Release/Reassign | 1 |
| 2b | Camper | §4.4 (Swarm chip and line) and R8 | 1 |
| 2c | AgentY | §5 (hold, answer, replay) | 1 |
| 2d | AgentX | §6 (episodes, toast inhibition, persistent item, leader notes, move action, clear on rebind, `account_blocks` expiry) | 1 |
| 2e | QTest | §4.6 (mobile shows the two states) | 1 |
| 3 | Manoz | §7 (wake), §8 (watchdog), then R9 (Codex parity) | 1; §7 also 2c |

Step 1 also moves the account rule into `run_accounts_of_block` (§3.5). Everything in 2d that reads `resets_at_ms`, `account_blocks` expiring at the reset among it, needs step 1's R2 to have stored the reset on the failure, so none of 2d can land before step 1. Step 1 has no visible UI except the Supervisor's pause reason. Each step-2 PR cites this spec, and adds its section's PR number to the Status line.

## 10. Tests

**Step 1 (unit):**
- `decide()` over the full table of turn active × process state × failure class × reset past/future/none.
- `resets_at_ms` from a `rejected` event, from a stale `allowed` event (ignored), from the text in the local zone and in another zone (`None`), and out-of-range values.
- `unavailable_of_block` with and without `agent:run_account`.
- Serde round trips for `signed_out`, and old meta without the new fields.

**Step 2:**
- A message to a limited agent is held and delivered after a rebind; the answer's wording.
- One toast per episode, and the per-block toasts inhibited.
- `account_blocks` expiry.
- The Kanban card in Needs you with its reason.
- The Swarm chip.

**Step 3:**
- A wake at a past reset after an srv restart.
- One wake per reset.
- The watchdog releases claims and notifies.

## 11. Measurement plan (Loap-3, in a task dev build)

**Rig.**
- Run a task dev build from `main` (before) and from each step's branch (after).
- Its panes use the scripted fake CLI (`scripts/ui-screenshots/fake-claude/cli.mjs`, selected with `FAKE_CLAUDE_SCRIPT`), so no account, quota or network is used.
- Add one script, `limit`. It plays a normal turn, then on the next turn emits:
  - a `rate_limit_event` with `status: "rejected"`, `resetsAt` = now + 5 min, `rateLimitType: "seven_day"`;
  - the error result `You've hit your weekly limit · resets <local time> (<local zone>)`;
  - it keeps failing like that until the reset time passes.
- Four panes are bound to one test account record. A Kanban task is claimed by one of them, and a second agent sends it 3 messages.

**Metrics.** Each one is measured before and after, alternating builds twice:

| # | Metric | Before (expected / seen 10-23) | After (target) | How |
|---|---|---|---|---|
| M1 | Limit hit → shown on the Kanban card, Swarm and `Agents conversations` | never | ≤ 30 s (the board's 15 s poll and the badge's 30 s) | timestamps: the `agent failure classified` log line vs. the first UI query and MuxAPI answer showing it |
| M2 | Messages lost into a limited agent (turn ended < 2 s with `cli_failed`) | 3 of 3 (4 for Naki-2 on 10-23) | 0; all 3 answered HELD and delivered after the move | srv log `ambient outcome` lines; the sender's answers |
| M3 | Toasts per account episode | 4 (one per agent) | 1 | `notify: publish` lines with kind |
| M4 | Account move → messages delivered | — | ≤ 10 s | log: rebind → held replay delivery |
| M5 | Reset passes → agent available (step 3) | never (until a manual move) | ≤ reset + 3 min | wake log line; state in `Agents conversations` |
| M6 | False positives: shown limited or signed out while it can work | — | 0 | after a rebind, after the reset, and after a clean turn, the state must be idle within one poll |
| M7 | Lease held by a limited holder | renews indefinitely | unchanged in steps 1–2 (R5 decides); shown in Needs you | `db_work_events` |

**On the real install, read-only after release.** The same scan as the report (`agent failure classified` → account move) gives limit-to-recovery times. They were 6 h 12 m and 6 h 41 m on 10-23. Re-measure over the two weeks after step 2 ships; expect minutes, since the operator's reaction time is now the bound.

## 12. Risks

- **A stale `last_failure` makes an agent look limited after it was fixed.** It's cleared by clean turns, rebinds, the reset and the wake (§3.3), and M6 checks it. If one still sticks, the operator's Retry in the pane runs a turn, which clears it.
- **A wrong reset time** (text parse). It is only parsed in the local zone, and range-checked. A wrong time only changes the label and the wake time; a wake that fails records a new failure.
- **Peers and mobile on older builds** show no status for these agents (§4.1, §4.6).
- **Holding a message the agent could have handled.** That can't happen: a Limited or SignedOut agent fails every turn until the state ends.
- **Privacy.** Account ids stay out of MuxAPI answers and toasts. Labels are shown only in this machine's UI.

## 13. Decisions (operator, 2026-10-23)

1. **Session limits raise no toast.** A `five_hour` limit gets the persistent item and the leader note only. The toast is for weekly limits (`seven_day`, or a usage limit with no window known) and for sign-in.
2. **The move action asks.** It preselects the first valid account with no block.
