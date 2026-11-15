# Report: unresponsive agents — how to detect them, who to tell, and how to recover their work

**Date:** 2026-10-23
**Status:** analysis — research only, no code. Read from source on `main` @ `52561f1db` and from this machine's srv logs, transcripts and Kanban history (read-only). Nothing was reproduced in a running build; the items marked *unverified* need a task dev run.
**Author:** Loap-3 (loap-loap-2-loap-3)
**Task:** Kanban #1255. **Trigger (operator, 2026-10-23):** "we need to think more about agents who are unresponsive, like on accounts with no quota. naki-2 was just stuck, but I released it."

---

## 1. Summary

- **The common case is not a hung agent: it is an account out of quota.** In the last two weeks this machine logged about 45 weekly-limit and 25 session-limit failures across 11 agents, against one API outage, three sign-in failures and no hung turns found. Several agents share an account, so they all stop in the same second.
- **AgentMux already knows.** srv classifies the failure (`UsageLimit`, `Auth`, …), stores it on the pane as `agent:last_failure`, and publishes a toast. But that fact never reaches the places where work is tracked: srv's agent state says **idle**, the Kanban card stays **In progress**, the lease keeps renewing because the process is alive, a message to the agent is answered **Delivered**, and no leader is told. The toast fires once, and is the only signal outside the pane.
- **Naki-2 today:** hit the weekly limit at 16:52 UTC, held two tasks, failed every message in under a second, and waited about 6 h 12 m until the operator moved it to another account at 23:03. Nothing was released on the Kanban; "released" was the account move. Loap-3 (this agent) and QTest were on the same account and waited about 6 h 40 m.
- **Build first:** turn the failure AgentMux already has into a visible state with its reset time — on the Kanban card, in the Swarm, in `Message send`'s answer and in MuxAPI — and tell the task's leader once (§7, R1–R3). That alone would have cut today's six hours to minutes, and it is built on positive evidence (a classified error), not on silence, which is what got the old detector removed.
- **Never automatic:** killing a working agent, cancelling a task, moving an agent to another account without the operator's standing consent, and reassigning a task that already has a branch or PR.

## 2. The ways an agent becomes unresponsive, with real cases

Counts are from this machine's srv logs and Claude transcripts, 2026-10-09 to 2026-10-23 (UTC).

| # | Mode | Seen here | What it looks like today |
|---|---|---|---|
| 1 | **Out of quota** (weekly or session limit) | ~45 weekly + ~25 session messages, 11 agents | Process alive and idle; every new turn ends in < 1 s with `You've hit your weekly limit · resets Oct 26, 7pm (America/Los_Angeles)` |
| 2 | **Rate limited / overloaded** (429, 529) | none in transcripts | The pane shows "Rate limited — retrying in …"; transient |
| 3 | **Signed out / credentials expired** | 3 incidents (`Not logged in · Please run /login`; `No account linked` ×6 with `identity.spawn.blocked`; `OAuth session expired` on two test agents) | Turn fails at once (running agent) or never spawns (spawn gate; messages HELD `needs_login`) |
| 4 | **Model API unreachable** | 1 outage, 10-20 10:15–10:27, ~11 agents (`ECONNREFUSED`, 10–40 CLI retries each) | v0.59.40's offline handling holds input and retries once on reconnect (Claude only) |
| 5 | **Turn hung on a tool** | none found | Pane shows Working indefinitely; srv `turn_active` stays true |
| 6 | **Turn waiting on a permission prompt nobody sees** | cannot happen for Claude or Codex as launched | Claude: every `can_use_tool` is auto-allowed except `AskUserQuestion` (`persistent/input.rs:948-966`); Codex runs with approvals bypassed (`providers/codex.rs:17-22`) |
| 7 | **Waiting on a question** (`AskUserQuestion`, Tasks `ask`) | routine | Handled: needs-you, Kanban "question waiting in its pane", leader note after 30 s for workers |
| 8 | **MCP down** | no mid-session disconnects in the logs | Self-heal works from the CLI's init frames only; a server dying between them is not noticed |
| 9 | **Pane closed while holding a task** | 2 lease expiries (Korp #161, #166, 10-23 23:00) | Lease lapses ~120 s later, but only when some agent next calls `claim` |
| 10 | **Crash loop** | none clear (exit −1 flips line up with upgrades) | No restart policy; a persistent agent respawns lazily on its next message |
| 11 | **Codex's own failures** | — | `turn.failed`/`error` events are rendered but not classified; the App Server controller records no failure at all |
| 12 | **Agents on other hosts** | — | LAN entries go stale after 60 s; a relayed message expires after 30 min without telling the sender; a remote agent's failure is not visible here |

### 2.1 Naki-2, 2026-10-23 (times UTC)

- **16:51:58** — the Claude CLI returned the weekly-limit message. srv logged `agent failure classified: Usage limit reached` (`UsageLimit`, `retryable:false`) and stored it on the pane. Loap-3, QTest and Agent5 hit the same limit in the same second: they share the account.
- **16:52:08–10** — srv published four notifications (one per agent, as far as the log shows; whether Windows showed all four or folded them is not logged). Nothing else outside the pane changed.
- **16:52 → 23:01** — nothing. The 0.59.40 upgrade at 19:06 respawned it with a resume, and it stayed idle. It still held **#163** (claimed since 10-22) and **#140** (blocked on a question to the operator); both leases kept renewing because its process was running.
- **23:01:51–23:02:13** — four messages from AgentX and AgentY each opened a turn that ended in under a second with the same limit and `ambient outcome … cli_failed`. Each sender was told the message was delivered (inferred from the delivery path; their turns ran).
- **23:03:32** — the operator moved Naki-2 to another account (`identity rebind changed this agent's Claude config dir`, then a forced restart). The new account could not resume the old session, so AgentMux started fresh and carried its record of the conversation (28,796 characters). The operator typed at 23:04 and it worked normally.
- **The Kanban never moved.** The operator's only Kanban actions that day were cancels, assigns and answers; there is no operator "release" of a claimed task in the Kanban at all (§3.3).
- The pane was behind another in its stack until a layout change around 23:0x, which is probably why the pane's own failure row wasn't seen.

The same pattern on 10-19/10-20: Naki-2 waited ~5 h 50 m, Loap-3 ~6 h. Across the two weeks there were 54 account moves among three accounts: moving the agent is the fix, and it is always done by hand, late.

## 3. The signals AgentMux already has, and where they stop

### 3.1 Failure classification — reliable, but stops at the pane

- `crates/srv/src/agents/failure.rs` `classify()` maps provider text to a class: `RateLimited`, `Overloaded`, `UsageLimit` ("usage limit", "weekly limit", "hit your session limit", "quota", …), `Auth` (401, "please run /login", spawn-gate wording), `Network`, `ContextExceeded`, `Killed`, `SpawnFailure`, `UnknownNonZero`, … Its tests include the exact weekly-limit text.
- It runs on Claude's mid-stream error frames (`persistent/stdout_reader.rs:683`), on persistent-process exit (`persistent/mod.rs:1615`), and on per-turn subprocess exit (`subprocess/host_spawn.rs:646-688`, used by Codex `exec` and Gemini).
- The result is the pane's `agent:last_failure`, cleared by the next clean turn, and an `agentfailure` event that the notification router turns into a toast ("Reached its usage limit.", "Needs you to sign in again." — `backend/notify/router.rs:386-399, 505-525`).
- **Who reads it:** the pane's failure row (`frontend/app/view/agent/failure/failure-accessory.ts`, which offers "Accounts (switch / upgrade)" for a usage limit), the toast router, `account_blocks` in `server/app_api/agent_spawn.rs:406-450` (so a new worker avoids an out-of-quota account), and `worker_notes` (a spawned worker's leader gets a note). **Nothing else:** not `agent_state`, not the Kanban, not the Swarm line, not the Supervisor, not `Message send`, not any MuxAPI field.
- **Gaps:** no reset time is extracted (the CLI's structured `rate_limit_info.resetsAt` is ignored; the "· resets …" text survives only in the stderr tail); Codex's `turn.failed`/`error` events and the Codex App Server path are not classified.

### 3.2 Agent state — says idle

- `backend/agent_state.rs:80-99` `decide()`: Working (turn active), Waiting (turn active and `term:awaiting_user`), Idle (process running), Error (exited non-zero), Stopped. A limited agent's process is running, so it is **Idle**, which is what the Kanban, `Agents workers` and other agents see.
- `Agents conversations` gives `turn_active` and `last_activity_ms`; `Agents transcript` shows the limit only as raw text.
- The Swarm line passes only live / needs-you / paused (`frontend/app/store/swarm-line.ts:156-163`), so it never shows errored either.

### 3.3 The Kanban — leases renew on "process running"

- `server/work_tasks.rs:241-263` `extend_live_leases`: a claim's lease (120 s) is extended while its holder is registered and its process status is `running`. Turn activity, output and failures are not checked, so **an out-of-quota or signed-out holder keeps its tasks indefinitely.**
- There is no background reaper: expiry is handled inside the next `claim` (`server/work_queue.rs:260-281`). A closed pane's task therefore returns to the pool only when someone next asks for work, and nobody is told.
- Holder states on the card (`frontend/app/view/warden-kanban/kanban-columns.ts:20-37`): gone → "agent closed", waiting → "question waiting in its pane", error → "agent errored", stopped → "agent stopped". **Idle and working have no reason**, so a limited holder's card stays in In progress, out of the Needs you count.
- `kanban.act` (`server/app_api/kanban.rs`) has create, edit, move, start, answer, accept, send_back, cancel, retry, track. **There is no operator release or reassign of a claimed task**; for a stuck holder the drawer offers "Open <holder>" and "Cancel task". Cancel doesn't tell the holder (it finds out from a 409).
- The reminder loop (`backend/kanban.rs`) is off by default, only nudges idle agents about open tasks targeted at them, and does nothing if they don't respond.
- Fencing exists: `attempts` is the fence token, so a holder whose claim was superseded gets a 409. That's the right base for any reassignment.

### 3.4 Messages — "Delivered" to an agent that can't answer

- A running agent is "Delivered" even if its turn fails in under a second on its limit. The HELD mechanism (`server/reactive.rs:734-743`, `server/jekt_held.rs`, 24 h, replayed when the agent can take it) is used only for not-running agents and spawn-gate refusals (`needs_login`). A usage limit is a perfect fit for it and isn't used.
- A cloud-relayed message expires after 30 min without telling the sender; held messages that expire or are dropped aren't reported either (mailbox spec, receipts not built).

### 3.5 The Swarm Supervisor — blind to failures

- It acts only at a completed turn end (`backend/supervisor/watcher.rs:59-71`), ignores process exits, and its judge runs on the pane's own account (`judge.rs:120`). An out-of-quota agent's judge fails too, and the result is "Couldn't judge this turn, so it stopped as usual". It has no stuck-agent or Kanban logic.

### 3.6 Other signals

- **Watchdog** (`backend/blockcontroller/watchdog.rs`): opt-in limits on turn runtime and mid-turn idle (`term:agentmaxruntimehours`, `term:agentidletimeoutmins`, both 0 = off). When it trips it kills the CLI (`stop_now`), **doesn't release the agent's claims and doesn't tell anyone** beyond a log warning. The claims lapse about 120 s later, at the next `claim` by anyone. The table at `sagas/agent_teardown.rs:16` says the watchdog uses `Policy::stop`, which would release claims; the code doesn't.
- **Turn liveness in the pane**: `STUCK_THRESHOLD_MS` 45 s (log only) and `LIVENESS_RECOVERY_MS` 180 s (the pane quietly goes back to Idle when no tool is running; srv still says `turn_active`). The srv-side silence detector and "Agent unresponsive" banner were removed in August for false positives during long tools and compaction (`SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md`). Any new stuck-turn signal has to avoid repeating that.
- **MCP self-heal** (`backend/mcp_health.rs`, `SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`): detects from the CLI's `system/init` frames, reconnects twice, restarts once in a safe profile, then gives up and tells the agent. No toast, no Swarm chip, nothing between init frames, Claude only.
- **Offline handling** (v0.59.40, `SPEC_OFFLINE_RESILIENCE_2026_10_20.md`, `crates/srv/src/connectivity/`): tracks Anthropic reachability from Claude's frames and a probe, holds input, retries a network-failed turn once. Codex isn't tracked.
- **Spawn gate / accounts**: `oauth_status` (valid / expired / needs_reauth) is probed from the token file at spawn (`identity/resolver/oauth_probe.rs`), with no network call; a mid-turn 401 is not written back to the account. `Identity validate` probes API keys only.
- **`Agents spawn` account fallback** (`agent_spawn.rs:345-450`): a new worker skips an account whose status is expired, or on which some open pane last failed with `UsageLimit` or `Auth`. "Out of quota" is just "some open pane's last failure", with no reset time; closing that pane forgets it. It never moves a running agent.
- **Tower / Warden**: CPU and memory per process and pane; Warden Host shows routing registration as active or idle (30 s). No agent health view.
- **Agents on other hosts**: Swarm marks a LAN or cross-channel entry stale after 60 s and hides its status; cloud installs aren't listed on the desktop.

### 3.7 Reliability verdict

| Signal | Reliable? | Missing |
|---|---|---|
| `agent:last_failure` (UsageLimit, Auth) | **Yes**: positive evidence from the provider, cleared on the next good turn | reset time; readers outside the pane; Codex coverage |
| Process running | Yes for "alive", useless for "can work" | — |
| Agent state Idle / Working | Misleading for limited agents (Idle) and hung tools (Working forever) | a failure-aware state |
| Kanban lease | Only catches closed panes, late | failure-aware renewal; operator release/reassign; notice on expiry |
| Toast | Fires once; transient | a persistent, grouped item |
| Silence / turn duration | **No**: removed for false positives | (inform only, with high thresholds, if at all) |
| MCP init frames | Yes at spawn | anything mid-session |
| Remote agent last-seen | Yes for "reachable" | its failures |

## 4. What should happen, and who acts

| Situation | Show | Tell | Recover (who) |
|---|---|---|---|
| Out of quota, reset known | Pane, Swarm, Kanban card: "Out of quota until Sun 7pm (account A)" | Operator: **one** item per account episode, listing the agents and tasks. Leader / requester of each held task: one note. Senders: `Message send` answers HELD (usage_limit) | Operator: move the agents to another signed-in account (one click for the group), or wait. AgentMux: deliver held messages and nudge the agent at the reset time |
| Signed out | "Needs sign-in" on pane, Swarm, card | Operator (it's theirs to do); leader once | Operator signs in; held messages replay (already built for spawn-gate refusals) |
| Rate limited / overloaded / API down | Already shown in the pane | Nobody (transient) unless it lasts > ~15 min | AgentMux retries (already built) |
| Hung tool | Swarm: "running `cargo test` for 40 min" (swarm-longrunning exists) | Leader once at a high threshold; never the operator by default | Leader or operator decides; nothing automatic |
| Pane closed holding a task | Card: "agent closed" (exists) | Leader / requester once | Task returns to the pool on expiry; an operator Reassign |
| Question waiting | Exists | Exists | Exists |
| MCP gave up | Swarm chip (self-heal spec Phase 4) | Operator only if the agent is holding a task | Agent runs `agentmux-mcp doctor` (exists) |
| Remote agent gone | Swarm "seen 12m ago" (exists) | Sender of a relayed message when it expires | — |

**Who acts:**
- **AgentMux** detects, labels, holds messages, renews or doesn't renew leases, and wakes an agent at the reset. All of these are reversible and cost nothing.
- **The task's leader** (seat lead, or the requester for now; workers already have `agent:worker_of`) gets one note per episode and can reassign within its own lane. That fits the seats model: the leader notices before the operator has to.
- **The operator** decides anything that spends money or changes identity: moving an agent to another account, reassigning a task that already has code, stopping an agent.

**Switching accounts.** It works today for Claude (54 moves in two weeks), at the cost of a fresh session carrying a 24k-character record rather than the full context. Codex keeps credentials per identity the same way, but a Codex move hasn't been exercised here (*unverified*). Moving work between the operator's own accounts is the operator's call, and the provider's terms for each plan are theirs to judge, not AgentMux's. So: one click with the operator's consent by default. An opt-in standing rule ("when an agent in group X runs out, move it to any signed-in account of the same provider with quota") can come later; it should never cross providers.

**Never automatic:**
- killing or stopping an agent mid-turn for silence;
- cancelling a task;
- moving an agent to another account without a standing rule the operator set;
- reassigning a task with a branch or open PR, where two agents would push to the same branch (fencing protects the Kanban, not git);
- answering an agent's question on the operator's behalf.

## 5. How other systems handle stuck workers

The same four parts appear everywhere; AgentMux has one (fencing) and parts of the others.

- **Leases renewed by heartbeat, with a cap.**
  - SQS hides a received message until a visibility timeout that the worker extends, with a 12-hour cap. After `maxReceiveCount` receives the message goes to a dead-letter queue. [AWS: visibility timeout](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-visibility-timeout.html), [DLQ](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-dead-letter-queues.html)
  - BullMQ marks a job whose lock isn't renewed as stalled, then failed after `maxStalledCount`, and warns that a busy CPU can starve the renewal. [BullMQ: stalled jobs](https://docs.bullmq.io/guide/workers/stalled-jobs)
  - AgentMux renews on "process running", which is a liveness check, not a progress check.
- **Liveness is not readiness.**
  - Kubernetes restarts a container that fails its liveness probe, but only takes one that fails its readiness probe out of service. [Kubernetes: probes](https://kubernetes.io/docs/concepts/workloads/pods/probes/)
  - Temporal separates a heartbeat timeout (a dead worker) from start-to-close (a slow attempt) and schedule-to-start (no capable worker). [Temporal: detecting activity failures](https://docs.temporal.io/encyclopedia/detecting-activity-failures)
  - **An out-of-quota agent is alive but not ready.** Drain it (stop handing it work, hold its messages), don't kill it.
- **Typed, non-retryable errors with a known delay.**
  - Temporal lets an error be non-retryable, or set its own next-retry delay. [Temporal: retry policies](https://docs.temporal.io/encyclopedia/retry-policies)
  - Anthropic's API distinguishes 429 (with `retry-after`), a spend cap (no `retry-after`), 529 and 401. [Anthropic: errors](https://platform.claude.com/docs/en/api/errors), [rate limits](https://platform.claude.com/docs/en/api/rate-limits)
  - Devin reports a suspended session's reason, including `out_of_quota` and `usage_limit_exceeded`, rather than "idle". [Devin API: sessions](https://docs.devin.ai/api-reference/v3/sessions/post-organizations-sessions)
  - Claude Code prints usage limits with a reset time and documents an auto-continue that waits for the reset in an open interactive session. That does not happen in AgentMux's stream-json sessions: no stored transcript shows a turn resuming after a limit error on its own (checked in the step-3 build, spec §7). [Claude Code: errors](https://code.claude.com/docs/en/errors), [usage limits](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work)
- **Bounded restarts, then escalate.**
  - Erlang/OTP supervisors give up after `intensity` restarts in `period` and pass the failure to their parent. [Erlang: supervisor principles](https://www.erlang.org/doc/system/sup_princ.html)
  - Kubernetes backs off crash loops up to 5 minutes. [Kubernetes: pod lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/)
  - Sidekiq Pro treats a job that crashed its process 3 times as poison. [Sidekiq: reliability](https://github.com/sidekiq/sidekiq/wiki/Reliability)
  - AgentMux's leader is the natural "parent" to escalate to.
- **Fencing tokens.** A reclaimed task gets a new epoch, and writes carrying an older one are refused. [Kleppmann: how to do distributed locking](https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html), [Chubby](https://static.usenix.org/events/osdi06/tech/full_papers/burrows/burrows.pdf). AgentMux's `attempts` already does this for the Kanban, but not for the agent's branch.
- **Hard caps and reassignment for agents.** GitHub's Copilot cloud agent stops a session after 59 minutes, and the documented fix for a stuck one is to unassign and reassign. [GitHub Docs: troubleshoot cloud agent](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/troubleshoot-cloud-agent)
- **Alert on symptoms, grouped.**
  - Google's SRE book: every page should be actionable and need judgement. [SRE book: monitoring](https://sre.google/sre-book/monitoring-distributed-systems/)
  - Alertmanager groups related alerts and inhibits downstream ones while a broader one fires. [Alertmanager](https://prometheus.io/docs/alerting/latest/alertmanager/)
  - Here that means one "account A out of quota until Sun 7pm: 4 agents, 2 tasks" in place of four toasts, then silence.

## 6. Design constraints from history

- The August removal of silence-based detection stands: **detect from positive evidence** (a classified failure, a closed pane, an expired lease), and treat time-based signals as information shown to a leader, never as a reason to act.
- The account, not the agent, is the unit of quota. Several agents share each account, and one event stops them together, so grouping by account is both less noisy and more useful (one action moves the group).
- Keep account details out of anything published: show the account by its label in the UI, and never put one in a toast body or a jekt to an agent on another host.

## 7. Recommendations, in order

Costs: S = under a day, M = a few days, L = a week or more.

**R1. Make the classified failure a state everyone can see. Build this first (S–M, low risk).**
- In `agent_state::decide()`, add `limited` (last failure `UsageLimit`) and `signed_out` (last failure `Auth`) while the process is alive and no turn is active, read from `agent:last_failure`.
- Kanban: `needsReason` "out of quota until …" / "needs sign-in", so the card moves to Needs you and counts in the badge.
- Swarm: a chip.
- MuxAPI: `Agents conversations` and `workers` return `failure {code, title, resets_at}`.
- Supervisor: skip the judge and pause with that reason.
- Files: `backend/agent_state.rs`, `server/app_api/kanban.rs`, `frontend/app/view/warden-kanban/kanban-columns.ts`, `frontend/app/store/swarm-line.ts`, `server/muxspect_handlers.rs`, `server/app_api/agent_spawn.rs` (workers), `backend/supervisor/policy.rs`.
- Risk: a stale `last_failure` showing after the account was fixed. It's cleared by the next clean turn and by an account rebind; make sure the rebind clears it.

**R2. Extract the reset time (S).**
- Prefer the CLI's structured `rate_limit_event.rate_limit_info.resetsAt`, currently ignored in `agents/translator/claude.rs:51` and `frontend/app/view/agent/providers/claude-translator.ts`.
- Fall back to parsing the "resets <date>, <time> (<tz>)" text.
- Store it as `resets_at_ms` on the `AgentFailure`. Codex's `resetsAt` is in its schemas.
- Files: `agents/failure.rs`, `persistent/stdout_reader.rs`.
- Also: `account_blocks` should expire an out-of-quota block at `resets_at` rather than keeping it until the pane closes.

**R3. Tell the sender and the leader (S–M).**
- `Message send` to a limited or signed-out agent answers `HELD for X (usage_limit, until …)` instead of Delivered. Replay at the reset or after an account move, reusing `jekt_held.rs` with a new `HoldReason`.
- One `[AgentMux]` note per episode to the leader or requester of each task the agent holds, by extending `backend/worker_notes.rs` from spawned workers to Kanban holders, using the task's `requested_by` until seats are data (#1102).
- Files: `server/reactive.rs`, `server/jekt_held.rs`, `backend/storage/jekt_held.rs`, `backend/worker_notes.rs`, `crates/mcp/src/tool_helpers.rs`.
- Risk: double delivery when an account move and the reset race. The replay is already one-shot per row.

**R4. One grouped, persistent operator item per account episode (M).**
- "Account A is out of quota until Sun 7pm: Naki-2, Loap-3, QTest, Agent5 — holding #163, #140". It's one toast that inhibits the per-agent `AgentCrashed` toasts for the same account, plus an item that stays in Needs you until it's resolved.
- Action: **Move these agents to another signed-in account**. It calls the existing `bindAccountToAgent` path for each idle agent in the group, which forces a restart with the continuation record.
- Files: `backend/notify/{policy,router}.rs`, `frontend/app/view/identity/bind-to-agent-menu.ts`, a Warden/Kanban surface.
- Risk: moving an agent mid-turn. Only idle ones are offered, the same rule the composer chip already uses.

**R5. Kanban recovery actions for the operator and leaders (M).**
- Add **Release** and **Reassign** to `kanban.act`. Release returns a claimed task to open; Reassign retargets it to another agent. Both bump the fence so a late call from the old holder gets 409, and both send the old holder a note, since cancel tells nobody today.
- Warn when the task has a branch or PR link.
- Stop renewing the lease of a `limited` or `signed_out` holder only for tasks open to anyone or a group, after the failure has lasted 30 min and the reset is more than an hour away, so someone else can take it. Targeted tasks stay put and show in Needs you.
- Files: `server/app_api/kanban.rs`, `backend/storage/work_queue.rs`, `server/work_tasks.rs`, `frontend/app/view/warden-kanban/kanban-drawer.tsx`.

**R6. Wake at the reset (S, after R2–R3).**
- At `resets_at` plus jitter, if the agent is still on that account: clear the block, replay held messages, and give it a system note to continue its claimed task. This uses `jekt_held` replay timing, or a one-shot `Schedule`.
- *Verified in the step-3 build (spec §7):* Claude Code's own auto-continue does not resume AgentMux's sessions, so the wake is built.

**R7. Fix the watchdog's silent kill (S).**
- When `term:agent*` limits trip, release the agent's claims (or say why not), raise one notification, and correct the stale table comment at `sagas/agent_teardown.rs:16`.
- Files: `backend/blockcontroller/watchdog.rs`, `sagas/agent_teardown.rs`.

**R8. Long tool calls as information, not failure (S–M).**
- Show "running <tool> for N min" in the Swarm, building on `swarm-longrunning.ts`.
- Send one note to the task's leader past a high threshold (for example 45 min, none for background tasks).
- No operator toast by default, no kill. This is the one time-based signal, and it only informs.

**R9. Codex parity (M).**
- Feed `exec`'s `turn.failed` / `error` events to `classify()`, record failures in `app_server_controller.rs`, and add Codex to connectivity tracking.
- Until then, R1–R6 cover Claude only. That is where all the cases here came from.

**R10. Later.**
- MCP self-heal signal B (a hello from `agentmux-mcp`) to catch mid-session deaths, plus its Swarm chip.
- Tell the sender when a relayed or held message expires (mailbox receipts).
- Carry a remote agent's failure code in LAN agent listings.
- An opt-in standing rule for automatic account moves within a provider (§4).

**What to build first:** R1 + R2 in one PR, then R3. Together they make an out-of-quota agent visible on the Kanban, in the Swarm and to its leader within a minute, with its reset time, and stop messages being lost into it. R4 then turns today's 6-hour manual fix into one click.

## 8. Not verified

- How many of the four toasts at 16:52 were shown, and whether the operator was away (the presenter's decisions aren't logged at this level).
- Whether senders were literally told "Delivered" (inferred from the delivery path: their turns ran).
- Codex account moves.
- The counts in §2 come from string matches over logs and transcripts. Builds before 0.59.35 filed session limits as `UnknownNonZero`, so the older counts are approximate.
