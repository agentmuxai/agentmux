# Report: timers, deadlines and clocks across AgentMux, and how to make them one system

**Status:** analysis. Written 2026-10-17; the code reading is checked against `main` at 8525dbdb0. Nothing is changed yet.
**Author:** Camper@narko, at the operator's request
**Trigger (operator, 2026-10-17):** "Let's take a closer look as to whether this needs a greater framework to operate within, in fact I thought we already had a timer system. Audit the entire codebase to ensure it is cohesive, write a report on how it can be improved." It follows [`RCA_QUESTION_TIMEOUT_DOES_NOT_FIRE_IN_HIDDEN_PANES_2026_10_17.md`](RCA_QUESTION_TIMEOUT_DOES_NOT_FIRE_IN_HIDDEN_PANES_2026_10_17.md).
**Method:** four sweeps of the whole repository:
- every time-based mechanism in `frontend/`;
- every one in the Rust crates;
- every "timed decision", where AgentMux acts for the user when a deadline passes;
- every spec, retro and incident about time.

Each finding this report leans on was then read in the code by hand; §8 lists the two platform questions that could not be.

## 1. Summary

**There is no timer system.** There are about a dozen good pieces, each built for one place and used by one or two callers:

| Piece | Where | What it does | Users |
|---|---|---|---|
| `useTick(ms)` | `frontend/app/hook/useTick.ts` | one shared interval per period, ref-counted | about 30 display labels |
| question timer | `frontend/app/store/question-timer.ts` | one deadline, one clock, published to block meta | the question panel and the Swarm chip |
| promotion clock | `frontend/app/view/agent/activity/promotion-clock.ts` | one-shot wake at the next due time | 2 instances per agent pane |
| `stream-scheduler` | `frontend/app/view/agent/stream-scheduler.ts` | rAF coalescing, aware of user input | 3 |
| `createDebouncedRefresh` | `frontend/app/view/agent/activity/debounced-refresh.ts` | debounce | 2 (about 25 hand-written debounces exist beside it) |
| lease scheduler | `crates/srv/src/backend/sampler/schedule.rs` | pure, fake-clock, "nothing runs unless someone holds a lease" | sysinfo only |
| `RefreshScheduler` | `crates/srv/src/broker/scheduler.rs` | single-flight credential refresh sweep | the MuxBus credential only |
| cron scheduler | `crates/srv/src/backend/cron/` | persisted user schedules | the Schedule tool |
| `agentmux_common::time` + `scripts/check-time-helpers.mjs` | `crates/common/src/time.rs` | wall-clock `now_ms`, enforced by a CI gate | all Rust crates |
| `user_attention::Asking` | `crates/srv/src/backend/user_attention.rs` | "this pane is waiting on you" | browser attention, SSH consent, widget install |

The pieces don't add up to a system. The counts:
- **About 40 srv background loops run for the life of the process**, each its own `tokio::spawn` with its own `interval` or `sleep`. More run per agent, per job and per connection.
- **The frontend has 7 separate countdown implementations, 5 backoff ladders and about 25 hand-rolled debounces.**
- **21 timed decisions** (§3) use **6 different pause policies** and **4 clock styles**. Only 3 of them publish their deadline. Two of the most important, the question auto-answer and the failure auto-retry, are owned by a renderer window. So they can stop forever, restart from zero, or be lost, depending on which panes happen to be mounted.

**What it costs today:**
- Two deadlines can never fire:
  - the question auto-answer in a hidden pane (§4.1);
  - the agent watchdog for every persistent or ACP agent (§4.3).
- Two can be lost:
  - the auto-retry on a renderer reload;
  - a pending shutdown on an srv restart.
- One user-facing countdown never counts down: the rate-limit wait.
- A pending-shutdown warning can be dropped while the 15 s keeps running.
- The docs disagree with each other on the basic rules (§5).

**The proposal (§6) is three small layers, not one big scheduler:**
1. **srv owns every timed decision,** through one deadlines service. Each deadline is persisted, published to every window and fired exactly once, under a pause policy declared per kind.
2. **The renderer has one clock and one countdown primitive.** Countdowns read the published deadline; none of them keeps its own.
3. **srv background loops use one helper** for period, first run, missed ticks, cancellation, jitter, backoff and a name in diagnostics. A CI ratchet keeps new code on these primitives, the way `check-time-helpers` already does for `now_ms`.

§7 orders the work. Independent quick fixes come first, so nothing waits on the framework.

## 2. Why there isn't one already

It was decided against, for good reasons at the time.
- **The polling analysis rejected a central scheduler.** [`ANALYSIS_POLLING_LANDSCAPE_2026_06_23.md`](ANALYSIS_POLLING_LANDSCAPE_2026_06_23.md) found a 1 s tick re-invented 13 times. It weighed "a full centralized scheduler… owns all timers, supports pause/resume, priority, batching, and visibility-gating", called it overkill, and chose the shared `useTick`. That was right for **display** ticks, and it worked: about 30 labels now share it. It said nothing about **deadlines that take an action**, and the backend was judged not to need a framework.
- **Every later unification was local.**
  - The Swarm question work made "one timer, many readers" for the question only (`SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md`).
  - The sysinfo rewrite built a general lease scheduler, and only sysinfo uses it (`REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md` §5.5).
  - The pane-state reducer emits timers as data (`schedule-interrupt-timeout`, `schedule-submit-timeout` with a `deadlineMs`), but nothing ever read those events (§4.4).
- **Rules were added one incident at a time,** and some contradict each other (§5). The keep-alive work (#3391) added "never fire invisibly" to the question timer to preserve an old side effect. That one line is the cause of the hidden-pane bug.

The missing parts are two decisions, not a missing library. **Who owns a deadline that acts for the user** has been decided case by case, often by whichever component drew the countdown. And **one policy for time** (clocks, pauses, visibility, restarts) was never written down.

## 3. The timed decisions

These are the places AgentMux does something for the user when time runs out, or waits on the user with a limit. In the table:
- *Hidden*: the pane is a dormant pane tab, or its window tab isn't displayed.
- *Unmounted*: no window holds the pane.
- *srv restart*: whether the deadline survives one.

| # | Decision | Default | Owner | Hidden | Unmounted | srv restart | Pause policy |
|---|---|---|---|---|---|---|---|
| 1 | Question auto-answer (recommended option) | 30 s, `agent:askquestiontimeoutms` | renderer (`question-timer.ts:176`) | **paused, no limit** | **no timer** | question re-asked on resume | activity: pause, then a fresh full timer 15 s after the last activity; hidden: unbounded |
| 2 | Answer dead-air fallback (re-send the answer) | 4 s | srv (`persistent/input.rs:733`) | runs | runs | lost | none |
| 3 | Tool permission prompt | none | srv | n/a | n/a | n/a | the decision panel is switched off (`persistent/mod.rs:1066`), so no deadline exists |
| 4 | Failure auto-retry | 5→120 s ±20%, 5 rungs | renderer (`useAgentFailure.ts:251`) | **frozen, resumes the remainder** | cancelled | n/a; **lost on renderer reload** | visibility freezes it |
| 5 | Shutdown override window | 15 s | srv, in memory (`sagas/pending_shutdown.rs:351`) | runs | runs | **lost: the shutdown never happens** | none |
| 6 | Self-quit turn grace | 30 s | srv (`sagas/self_quit.rs:117`) | runs | runs | lost | ends early at turn end (by polling, §4.6) |
| 7 | Swarm row auto-retire | 60 s | renderer (`swarm-model.ts:1747`) | runs | n/a | n/a | hover: freeze, then a fresh 60 s |
| 8 | Held jekt expiry | 24 h | srv, persisted | runs | runs | survives | none; **the sender is never told** |
| 9 | Cloud relay queue | 30 min | relay | — | — | — | none |
| 10 | Deferred deliveries, "stranded" | 20 × 500 ms | srv, in memory | runs | runs | lost | none |
| 11 | Agent watchdog (max runtime, idle output) | off by default | srv (`blockcontroller/watchdog.rs`) | runs | runs | survives | none; **never acts on persistent or ACP agents** (§4.3) |
| 12 | Notification debounce | 0–10 s | srv (`notify/policy.rs:64`) | — | — | lost | cancelled by the user acting |
| 13 | Notification pause and quiet hours | user | srv | — | — | survives | notices due during a pause are dropped for good |
| 14 | Supervisor (judge, Drive run limit) | 90 s judge, 2 h run | srv | runs | runs | survives (meta) | the user's Stop; the 2 h limit is checked when a turn ends, the only moment the Supervisor acts |
| 15 | Cron jobs / `every` loops | user | srv (cron, persisted) / the agent's MCP process (loops) | runs | runs | cron survives; loops die with the session | none |
| 16 | Upgrade waits and launcher dialog | 10 min / 150 s / 3 min | srv and launcher (two clocks) | — | — | — | none |
| 17 | Browser handoff / approvals / bypass requests | 10–15 min | srv, in memory (`browser_attention.rs`) | runs | runs | lost | none; fails closed |
| 18 | Browser bypass grant | 15/60/240 min | srv, in memory | — | — | lost (grant ends) | — |
| 19 | SSH consent | 120 s srv, 150 s host | srv and host (two clocks) | — | — | — | none |
| 20 | Credential approval | 60 s | host | — | — | — | none |
| 21 | Others: widget install (300 s, then "pending"), crash page (30 s, any key cancels), take-over confirm (5 s) | | various | | | | |

Seven of these (1, 4, 5, 7, 16, 17, 19) have the same shape:
- an owner;
- a deadline;
- a pause policy;
- an action at expiry;
- a countdown the user sees;
- state other windows need to read.

Each is built differently: `setTimeout` plus a 1 s tick, a decrementing `setInterval`, `tokio::select` against an absolute `deadline_ms`, `tokio::time::timeout`, and a dialog thread. Only #1 (written by a renderer), #5 and #18 publish their deadline at all.

## 4. Findings

Severity: **P1** acts wrongly or never acts on something the user relies on; **P2** wrong or misleading but recoverable; **P3** waste, duplication or leaks.

### 4.1 The question auto-answer depends on which windows are open (P1)

The full chain is in the RCA. The parts that matter for the framework:
- **Hidden:** the timer is not scheduled at all while the pane is hidden (`question-timer.ts:169-173`), and showing the pane starts a fresh full timer (`:218-223`). A question in a background pane tab or a background window tab never auto-answers.
- **Unmounted:** the only owner is the renderer that mounts the panel (`question-timer.ts:10-13`). With no window holding the pane there is no timer. srv parks the question (`pending_questions`) and has no deadline.
- **Phantom countdown:** when the owning pane unmounts, its published `counting` state stays in block meta (`releaseQuestionTimer` writes nothing, `:247-250`). The Swarm chip counts down to zero and then shows nothing, while the question is still pending (`:303-311`).
- **Backwards visibility:** a minimised or covered OS window still counts as visible (`usePaneTabVisibility` doesn't read `document.visibilityState`, `pane-tab-visibility.ts:31`). So a question auto-answers in a minimised window but never in a hidden tab, which is the opposite of the stated intent.
- **Wrong transcript note:** it says "no response in 30s" even when the setting is different (`useAgentQuestions.ts:186-187`).

### 4.2 The failure auto-retry can be lost or doubled (P1)

- A renderer reload restores the failure row from meta but never re-arms the retry: the seeded path only dispatches (`useAgentFailure.ts:272-278`), and arming happens only on a live event (`:291`).
- It counts down by decrementing once per tick (`:258`), so a throttled background window falls behind and never catches up.

### 4.3 The agent watchdog never stops persistent or ACP agents (P1)

`run_watchdog_loop` (`blockcontroller/watchdog.rs:25`) checks:
- the runtime limit against `spawn_ts_ms`, which is `None` for persistent and session agents (`blockcontroller/mod.rs:1211`);
- the idle limit only on a `ShellController` (`watchdog.rs:69`).

So `term:agentmaxruntimehours` and `term:agentidletimeoutmins` do nothing for Claude, Codex App Server or ACP agents, which is most agents. The per-agent `idle_timeout_minutes` and `restart_on_crash` fields are stored but never read by runtime code. Separately, the two limits use different clocks: wall clock for runtime, `Instant` for idle (§4.7).

### 4.4 The pane reducer's timers as data are ignored (P2)

The reducer emits `schedule-interrupt-timeout` and `schedule-submit-timeout`, each carrying a `deadlineMs` (`store/agent-pane-state/reducer.ts:707, 1148`). Nothing consumes those events. Instead:
- `INTERRUPT_TIMEOUT_MS = 5_000` and its `InterruptTimeoutElapsed` arm are never dispatched outside tests. The real stop fallback is a hard-coded 1.5 s `finalizeTurn` in `useTurnLifecycle.ts:250`, which bypasses the reducer's own rule.
- The submit timeout re-derives its deadline from the phase (`useTurnLifecycle.ts:344`) instead of using the one the reducer computed.

This is the best existing seed for the framework: a pure reducer that asks for a deadline, which an interpreter turns into a timer. The interpreter was never built (see also `ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md`).

### 4.5 Pending shutdown: lost on restart, warning can be dropped (P1)

- The 15 s window is an in-memory task (`sagas/pending_shutdown.rs:351`). An srv restart inside the window loses the request, so the shutdown silently never happens.
- The ShutdownPending notice can be dropped while the clock keeps running:
  - during a notification pause or quiet hours, unless `pause_allow_attention` is on (it is off by default; `notify/policy.rs:579-584, 606-610`);
  - by the global rate cap (`:616`).

  With no window showing the pane, that notice is the only warning.

### 4.6 Smaller defects (P2 and P3)

| Finding | Evidence |
|---|---|
| The rate-limit "retrying in X" never counts down: it formats a fixed `retryAfterMs` every second | `view/agent/status/row-status.ts:57`; the value comes from `phase.retryAfterMs` (`AgentBottomPanels.tsx:266-268`) |
| Flash-error expiry starts a new `setTimeout` chain on every effect run, with no handle | `app.tsx:306-321` |
| The process poller says it stops when its registry is dropped, but it holds a strong `Arc` to it | `backend/process_tracker/registry.rs:461-475` |
| The session archiver sleeps an hour before its first sweep, so an srv restarted more often than hourly never archives | `bootstrap/background.rs:532-535` |
| Three places poll `turn_active` every 50–250 ms for up to 30 s instead of waiting for the turn-end event the health tracker already has | `sagas/self_quit.rs:126`, `sagas/pending_shutdown.rs:421`, `persistent/mod.rs:1500, 1535` |
| `STOP_GRACE_MS = 15_000` is defined twice, with two separate "user stopped this" maps | `notify/router.rs:158`, `supervisor/mod.rs` |
| Held-jekt expiry and watchdog kills leave only a log or counter; neither the sender nor the user is told | `server/jekt_held.rs:136`, `watchdog.rs` |
| `timeoutms: 0` means "no timeout" in srv but auto-cancels after about 1.3 s in the modal (which srv no longer produces) | `backend/userinput.rs:51`, `modals/userinputmodal.tsx:18, 83-92` |
| Module-level intervals that are never cleared | `termViewModel.ts:36`, `mos.ts:394`, `launcher-event-store.ts:314`, `atom-cache-diagnostic.ts:63`, `ws.ts:61`, `AsciiSpinner.tsx:30` |
| Countdown banners that tick every 250–500 ms for the life of the component, even when idle | `ShutdownPendingBanner.tsx:80`, `MaintenanceSection.tsx:109` |
| Dead code | `storage/filestore/core.rs:1014` `start_flusher` |

### 4.7 Clocks are mixed inside single deadlines (P2)

- **Cron** computes the next fire on the wall clock, then sleeps toward it on tokio's monotonic clock (`backend/cron/mod.rs:206-207`). Where the monotonic clock stops during suspend (Linux, macOS), a fire after a laptop sleep is late by the time the machine slept.
- **The watchdog** measures runtime on the wall clock and idleness on `Instant` (§4.3).
- **The turn settle window** keeps its deadline on the wall clock but waits with a tokio sleep (`health.rs:368`).
- **The question timer** shows `endsAt` from `Date.now()` but fires from `setTimeout(durationMs)` (`question-timer.ts:175-176`): two sources of truth.
- **Countdowns that decrement per tick** (auto-retry, the resume banner, the user-input modal) drift in a throttled window. The ones that subtract from a deadline (shutdown banner, pairing code, Swarm) don't.
- **Login flows** mix `performance.now()` and `Date.now()` within one flow (`run-provider-login.ts:264, 352` vs `:483, 588`).
- **Gaps in the CI gate:** `agentmux_common::time` is wall-clock only, and `check-time-helpers` doesn't catch `chrono::Utc::now().timestamp*()` (used about 10 times) or anything about timers.

### 4.8 Duplication (P3)

| What | Copies |
|---|---|
| Countdown | 7 in the frontend (question timer, auto-retry, Swarm retire, resume banner, user-input modal, shutdown banner, pairing code), several more in srv |
| Backoff ladder | frontend 5 (`ws.ts`, `transient-network.ts`, `layoutIntents.ts`, `usePtyWidth.ts`, `useAgentFailure.ts`); srv 4 copies of the same 5 s→60 s doubling (`cloud_subscriber.rs:522`, `telegram/poller.rs:118`, `slack/socket.rs:181`, `discord/gateway.rs:121`) plus 3 other shapes (`wan_presence` machine, launcher ×2) |
| Debounce | frontend about 25 hand-rolled, plus `createDebouncedRefresh`, plus the `throttle-debounce` library; four copy-pasted 150 ms debounces in `swarm-model.ts` alone; srv 4 styles |
| Rate limiter | two near-identical token buckets (`reactive/rate_limiter.rs`, `lan_discovery.rs:1491`), plus fixed-window and sliding-window ones |
| Expiry sweep | frontend 3 (notifications, flash errors, activity dock) |
| Login polling | 3 |
| Loop setup | about 40 srv loops with no shared rule for first run, missed ticks (Burst, Delay, Skip all used) or cancellation (about 30 take no cancel token) |

## 5. The rules the docs already give, and where they collide

Fifteen principles appear across the specs, retros and incidents. Most are good and should become the framework's rules (§6.4). Six pairs contradict each other:

| A says | B says | Resolution proposed |
|---|---|---|
| Work must never stall; the question timer must not be gated even when minimised (`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` §2.3) | Never fire invisibly; pause while hidden (`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md`, pane tab contract Phase 3b) | Visibility never pauses a decision deadline (operator, 2026-10-17). Being unseen is handled by telling the user afterwards, not by waiting |
| Every pause must be bounded (hover and keyboard pause specs) | The dormant pause has no limit | Every pause has a maximum; the framework enforces it |
| Restart fresh on resume (question, Swarm retire) | Resume where it left off (auto-retry) | Declared per kind; a resumed timer reads the remaining time from the deadline, never from a counter |
| No timers, no watchdogs: state holds until a signal arrives (Host reducer Phase H, the operator's "find the deterministic signal") | Hold-time watchdogs and self-heal timers (09-07 incident, host-hang watchdog) | Signals first; a deadline only as a declared backstop, named, logged when it fires |
| Wall clock (question `endsAt`, `now_ms` everywhere) | Monotonic for elapsed time and rates (sysinfo report, 09-07 incident) | Wall clock for deadlines that are published or persisted; monotonic for measuring durations and rates (§6.4) |
| No central scheduler (polling analysis) | A lease scheduler (sysinfo report §5.5) | No central scheduler for display ticks; one service for decisions; one helper for loops |

## 6. The proposed framework

### 6.1 Layer 1: srv owns every timed decision

A small `backend/deadlines` service. A **deadline** is:

```
Deadline {
  id, kind,                       // e.g. "question.auto_answer", "shutdown.override"
  subject: block / agent / request id,
  started_at_ms, fires_at_ms,     // wall clock: published and persisted
  state: counting | paused{reason, until?} | fired | cancelled,
  pause_policy,                   // declared per kind (below)
  on_restart: refire_if_overdue | resume_remaining | restart | drop,
  action,                         // a typed enum handled in srv
}
```

- **One owner, one fire.** srv holds the timer and moves `counting → fired` under a lock (compare-and-set), so an expiry racing the user's own answer, a Keep or a cancel resolves to exactly one outcome, as the shutdown saga already does for Keep versus expiry.
- **Persisted.** Deadlines live in SQLite. At startup srv re-arms each one according to its `on_restart` policy: a shutdown override that was due during the restart fires; a question timer resumes its remaining time.
- **Published.** Every state change goes out as one event (`deadline`) and is mirrored to block meta for the pane it concerns. Every window, Swarm and the mobile viewer read the same deadline. No renderer writes deadline state. That removes the phantom countdowns.
- **Pause policies, declared per kind and bounded by construction:**
  - `none` (shutdown override, browser approvals);
  - `activity_defers{quiet_ms, max_ms}` (question: user activity, reported by the panel, defers it, with a hard maximum);
  - `hover_freezes{max_ms}` (Swarm retire, if it moves to srv).

  Visibility is **not** a pause input.
- **Announced and recorded.** A deadline that waits on the user registers with `user_attention` (which already exists), so the pane, the tab and Swarm show "waiting on you". Its notice goes through the notification router at Attention priority, which a pause must not drop for a deadline that will act. When it fires, it leaves a line in the transcript or the audit log, and tells the sender where there is one (held jekts).

**Who moves here:**
- **#1 question auto-answer.** srv already holds the question's options in `pending_questions`, so the recommended-option choice (`applyRecommendedDefaults`, in the panel today) moves to srv with it.
- **#4 auto-retry.** srv knows the failure.
- **#5 shutdown override and #6 self-quit grace.** These are already in srv; they gain persistence and the event.
- **#17 browser attention and #19 SSH consent.**
- **#8 held-jekt expiry,** to gain a sender notice.
- **#11 watchdog limits,** as per-agent deadlines.
- **#14 Supervisor,** if an override window before a continuation is wanted.

The question timer's tests and the shutdown saga's atomic Keep-vs-expire logic are the starting material.

### 6.2 Layer 2: one clock and one countdown in the renderer

- **`clock.ts`**, generalising `useTick`:
  - `useNow(periodMs)`: a reactive `Date.now()` on the shared interval, with a catch-up on `visibilitychange` for throttled background windows;
  - `useCountdown(deadline)`: remaining time computed from a published `fires_at_ms`, never decremented.

  Every countdown in §4.8 becomes a `useCountdown` over an srv deadline, or over a local one for purely cosmetic timers.
- **`useLocalDeadline(ms, action, policy)`** for UI-only deadlines that have no business in srv: the activity dock's row retention, the attachment-undo offer, the crash page. It takes the same declared pause policies, always bounded, and never takes visibility as an input.
- **One `debounce`, one `backoff`** (with jitter, as `useAgentFailure` and `usePtyWidth` already have), replacing the hand-rolled copies.
- **The pane reducer's `schedule-*` events get their interpreter.** One small adapter turns a `{type, deadlineMs}` event into a `useLocalDeadline`. Then delete the 1.5 s literal, or wire `INTERRUPT_TIMEOUT_MS`.
- **Polling:** pollers that only matter while visible take a lease, the sysinfo model applied to the frontend: `usePoll(fn, period, { while: visible })`. It replaces the dozen ad-hoc `setInterval` pollers with mixed gating (§4 of the frontend inventory).

### 6.3 Layer 3: one way to run a loop in srv

- **`spawn_periodic(name, period, opts)`** where `opts` is `{first_run: now|after_period, missed: skip, jitter, cancel: CancellationToken}`. It registers the loop in a diagnostics registry (name, period, last run, last duration, error count), visible next to the ambient outcomes. The about 40 loops move to it. The archiver's hour-long first wait and the process poller's never-dropping task stop being possible by accident.
- **Shared `Backoff`** (the `wan_presence` machine's jittered version) and **shared `Debounce` and `TokenBucket`.**
- **`wait_turn_end(block_id, max)`** on the health tracker, replacing the three `turn_active` polls.
- **One `STOP_GRACE_MS`** and one "the user stopped this" registry, shared by the router and the Supervisor.

### 6.4 Rules, and the gates that keep them

1. **A timer that acts for the user is an srv deadline** (Layer 1), never a renderer timer.
2. **Visibility never pauses a decision.** It may pause rendering, polling and display ticks.
3. **Every pause is bounded and declared.**
4. **Countdowns read a deadline; they never decrement.**
5. **Wall clock for anything published or persisted** (`fires_at_ms`, held-message TTLs); **monotonic for measuring** (elapsed, rates, idle time). `agentmux_common::time` gains `mono()` and a short doc of this rule.
6. **Signals first; a deadline is a named backstop** that logs when it fires.
7. **Loops go through `spawn_periodic`.**

The gates are ratchets, like `check-time-helpers` (a list that may only shrink):
- extend `check-time-helpers` to `chrono::Utc::now().timestamp*()` and to the remaining crates;
- a new `check-timers` script fails on raw `tokio::spawn` loops with `interval` or `sleep` outside the helper in srv, and on raw `setInterval` outside `clock.ts` in the frontend.

## 7. Plan

Each step is a PR on its own, ordered by user impact.

**Phase 0: fixes that need no framework (independent, small)**
1. Question timer: drop the visibility pause (RCA option A), and make the transcript state the timeout that actually fired. (Clearing the published state on unmount would break a pane moving between windows, #4250; the phantom countdown goes with Phase 1, when srv owns the timer.)
2. Watchdog: make the limits apply to persistent and ACP agents (use the controller's own spawn time and last-output time), or remove the settings. Read or drop `idle_timeout_minutes` and `restart_on_crash`.
3. ShutdownPending: Attention priority bypasses a notification pause; never rate-capped.
4. Rate-limit countdown: subtract elapsed time from a deadline.
5. Flash-error expiry chain; process poller `Weak`; archiver first run at start; one `STOP_GRACE_MS`.
6. Auto-retry: the same visibility rule as the question timer, and count from a deadline, not per tick. (Re-arming after a reload needs to know when the failure happened, and the stored failure has no time; that moves to Phase 1, where srv owns the retry.)

**Phase 1: the srv deadlines service, with two consumers that prove it**
1. The service: model, store, event, restart policy, compare-and-set firing, `user_attention` and router hooks.
2. Move the shutdown override (#5) and self-quit grace (#6) onto it. They are already in srv, so this proves persistence and the event with the least risk.
3. Move the question auto-answer (#1), with the recommended-option choice. The panel reports activity and renders the published deadline.
4. Move the auto-retry (#4).

**Phase 2: the renderer clock**
- `clock.ts` (`useNow`, `useCountdown`, `useLocalDeadline`), `debounce`, `backoff`, `usePoll`.
- The pane reducer's interpreter.
- Move the 7 countdowns, then the pollers.

**Phase 3: srv loops**
- `spawn_periodic` and the loop registry; move the loops.
- Shared `Backoff`, `Debounce` and `TokenBucket`; `wait_turn_end`.
- The remaining timed decisions (#17, #19, #8, #11) onto the deadlines service.

**Phase 4: gates**
- `check-timers`, and the `check-time-helpers` extension, with the current sites on a shrink-only allow list.

## 8. Decisions for the operator, and what wasn't verified

**Decisions:**
1. **Visibility.** This report assumes your rule for the question timer ("trigger regardless of whether the agent is visible") holds for every timed decision. The auto-retry has the same visibility rule. Is the rule for both?
2. **No window open.** Should a deadline fire with no window showing the pane? Layer 1 makes that possible; it's what "work never stalls" implies.
3. **After an srv restart.** A deadline that came due while srv was down: fire it, resume its remaining time, or start over? Proposed per kind: shutdown fires, question resumes, auto-retry starts over.
4. **The Supervisor** sends continuations with no grace window. Add a short override window, like the shutdown's?
5. **The watchdog:** make it work for all agents, or remove the settings?
6. **Notification pause versus a deadline that will act:** always show those notices (proposed), or respect the pause?

**Not verified:**
- **Whether tokio's clock stops during suspend on Windows.** It does on Linux and macOS. The cron drift (4.7) is certain only there.
- **Chromium throttling timers in background windows.** It is the leading explanation in `REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md`, not confirmed live. None of the switches that disable it are passed today. Layer 1 makes it irrelevant for decisions.

## Appendix: sources

**Inventories:** every path in §3 and §4 was read at 8525dbdb0.

**Docs this report builds on:**
- [`ANALYSIS_POLLING_LANDSCAPE_2026_06_23.md`](ANALYSIS_POLLING_LANDSCAPE_2026_06_23.md)
- [`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`](../specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md), the hover and keyboard pause specs
- [`SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md`](../specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md)
- [`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md`](../specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md), [`SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`](../specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md)
- [`SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`](../specs/SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md), [`SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md`](../specs/SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md), [`SPEC_AGENT_SELF_QUIT_2026_09_24.md`](../specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md)
- [`../reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md`](../reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md) §5.5 (the lease scheduler)
- [`SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md`](../specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md), [`SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md`](../specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md)
- [`ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md`](ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md)
- [`../incident/INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`](../incident/INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md), [`../retro/retro-stuck-background-dock-timer-2026-08-10.md`](../retro/retro-stuck-background-dock-timer-2026-08-10.md), [`../specs/uptime-clock-sync.md`](../specs/uptime-clock-sync.md)
- `scripts/check-time-helpers.mjs` and [`SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md`](../specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md) §6.1 (the epoch-time gate)
