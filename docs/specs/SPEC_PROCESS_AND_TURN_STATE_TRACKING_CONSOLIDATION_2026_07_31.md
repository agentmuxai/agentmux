# SPEC — Process & Turn-State Tracking: This Session's Findings, and the Case for a Unified State Machine

**Date:** 2026-07-31
**Type:** Consolidation spec — synthesizes multiple bugs/gaps found in one investigation session, proposes a unified fix direction
**Status:** Investigation complete for all findings below; two fixes already shipped — the heartbeat pattern (operational, no code change) and `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md` (merged via PR #2369 on 2026-07-30, **after** this session's initial pass mistakenly reported it as "not yet implemented" — see correction in §5.4). Note: #2369 merged *after* the `v0.54.7` release cut (2026-07-29), so it is not yet in any released/installed build — only a fresh `dev` build off `main` has it. Remainder need design work.
**Trigger:** User asked "why are we still having trouble with `task dev`?" after a `task dev` background-task bookkeeping loss, plus reported a new regression: the agent pane spontaneously shows "Picked up more work — starting another round…" and gets stuck showing "Working" indefinitely. User's framing: **"we need a state machine for all these things, and it seems like they are always leaking."**
**Related:** `RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`, `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md`, `retro-task-dev-isolation-multi-agent-2026-06-23.md`, `retro-persistent-agent-working-status-stuck-2026-07-16.md`, `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md`, `REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md`, `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md`, `SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md`.

---

## 1. Executive summary

The user's diagnosis is correct, and it's a pattern, not a coincidence: **this app has at least five independent mechanisms that each answer some version of "is this thing still going?", none of which share a common source of truth, and every one of them has its own gap that lets state desync from reality.** Across one investigation session, we hit four of the five:

| # | Mechanism | What it tracks | Where it lives | Status found this session |
|---|-----------|-----------------|----------------|---------------------------|
| A | `agentmux-bashwrap` idle-timeout | Raw PTY byte silence for a wrapped OS process | `agentmux-bashwrap/src/bash_wrap.rs` | Working as designed for its original case (pager hangs); **false-positive on GUI/daemon launches** — fixed operationally this session (§2) |
| B | `ActivityDock` | Conversation-transcript `ToolNode`/`ShellNode` status | `frontend/app/view/agent/components/ActivityDock.tsx` + adapters | No OS visibility at all — **by design**, but this makes it invisible to a detached process (§2) and vulnerable to orphaned entries (§3) |
| C | `AgentProcessRegistry` | Real OS processes inside the agent's own Job Object | `agentmux-srv/src/backend/process_tracker/registry.rs` | Separate data model from B — "none of them talk to each other" (already named in `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` §2.3/§2.5) |
| D | Background-task/task-notification bookkeeping | Whether a `run_in_background` Bash tool call has finished | Tied to the Claude Code process's own lifecycle | **Newly observed this session: does not survive a session/process restart**, even when the underlying OS process does (§4) |
| E | Agent-pane `TurnPhase` reducer + `settled-grace.ts` + liveness watchdog | Is the agent "Working" on the current turn | `frontend/app/store/agent-pane-state/reducer.ts`, `frontend/app/view/agent/settled-grace.ts` | **Confirmed real gap: can get stuck "Working" indefinitely** when resumption rounds are tool-call-only and arrive faster than the 3-minute watchdog (§5) — **fix already fully designed, not yet implemented** |

None of A–E were built together or share a design. Each was added independently to solve one narrow, previously-reported symptom (a pager hang, a dock-visibility spec, a process-registry badge, background-task result delivery, a stuck turn indicator) — and each one's own "is it still alive" heuristic is different: PTY byte counts, transcript message shape, OS Job Object membership, the calling process's own lifetime, and message-content-block classification, respectively. That's the actual root cause behind "it seems like they are always leaking": **there is no single answer anywhere in this codebase to "is X still running," so every consumer maintains its own approximation, and every approximation has edge cases.**

Section 6 proposes a unified direction, explicitly modeled on the same shape of fix that just closed an analogous class of bug in the Rust backend this session (PR #2371/#2373 — see that PR's own module doc in `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs`): a single, generation-tagged, pure `(state, event) -> (state, effects)` state machine, instead of several independently-mutated fields/registries that can disagree.

---

## 2. Mechanism A + B — `task dev` idle-kill vs. dock visibility (fixed this session, operationally)

Full writeup: `RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`.

- **The bug:** `agentmux-bashwrap`'s idle-timeout (600s of PTY silence → kill; introduced in `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md` to fix a real `less`-pager hang) can't distinguish "stuck forever" from "succeeded by opening a GUI window that will now legitimately go silent." `task dev` hit this: builds clean, Vite comes up ready, then ~10 minutes later gets killed and reported as a failed background task.
- **First-draft fix (superseded):** detach the launch entirely via PowerShell `Start-Process`, bypassing bashwrap. This does avoid the kill, but a detached process is invisible to **both** mechanism B (the dock — no `ToolNode` was ever created for it) and mechanism C (`AgentProcessRegistry` — it's not inside the agent's Job Object). A user has no way to find or stop it through the app itself.
- **Corrected fix:** keep it as a normal `run_in_background: true` Bash tool call (so it gets a `ToolNode` mechanism B can promote to visible after ≥30s), and wrap it in a backgrounded heartbeat loop so mechanism A never sees 10 minutes of silence:
  ```bash
  cd "<repo root>" && (
    while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &
    HEARTBEAT_PID=$!
    trap "kill $HEARTBEAT_PID 2>/dev/null" EXIT
    ./scripts/dev-agent.cmd TITLE="<label>"
  )
  ```
  This is independently-precedented — another agent session solved the identical problem the same way (`retro-persistent-agent-working-status-stuck-2026-07-16.md`), without either session knowing about the other's fix. **That itself is a symptom of the root problem**: the correct workaround has now been rediscovered twice because it isn't written down anywhere a caller would find it before hitting the bug.
- **Status:** fixed operationally (no app code changed). Follow-up (not yet done): bake the heartbeat into `scripts/dev-agent.cmd` itself, and/or document the pattern in `CLAUDE.md`, so a third rediscovery doesn't happen.

---

## 3. Mechanism B — orphaned `ToolNode`s never resolve mid-session

- **The bug:** a Bash tool call rejected by the harness *before* it ever runs (e.g. a built-in guardrail against standalone `sleep <N>` — confirmed external to this repo; no matching validator exists anywhere in `agentmux-srv`/`agentmux-bashwrap`'s source) can leave its `ToolNode` permanently at `status: "running"` in the dock. The only cleanup, `scrubOrphanedInProgress` (`frontend/app/store/agent-document/reducer.ts`), only runs at a session boundary (`SessionEnd`, `HistoryRestored`, `HistoryLoaded`) — not immediately when the rejection happens.
- **Why:** the dock's entire model is "trust the transcript's own status field, with no independent liveness check" — the same root pattern as every other row in the table in §1.
- **Status:** diagnosed, not fixed. See §7 for a proposed direction (a max-age fallback even for `"running"` status).

---

## 4. Mechanism D — background-task completion tracking doesn't survive a session/process restart

- **The bug, directly observed this session:** a `run_in_background: true` Bash tool call (the corrected `task dev` launch from §2) received this notification on the next turn:
  > "No completion record was found for this background shell command from the previous session. It may have been stopped (via the UI, Monitor timeout, or agent teardown — these leave no transcript marker), or it may have been running when the previous Claude Code process exited."
- **What actually happened:** checked via `tasklist` — the underlying OS processes (`agentmux-launcher.exe`, 8× `agentmux-cef.exe`, `task.exe`) were **still alive and healthy**, unaffected. Only the *tracking record* for that specific tool call — tied to the Claude Code process's own session lifetime, not to anything persisted about the OS process itself — was lost across whatever session boundary occurred (this conversation underwent at least one compaction/restart during this investigation).
- **Why this matters beyond "confusing":** this is the same failure shape as mechanism A's false positive — a perfectly healthy underlying process, misreported by an layer that has no persistent, ground-truth-checked model of "is it actually still running." The difference here is the desync source is the **agent harness's own session lifecycle**, not bashwrap's PTY timer.
- **Direct answer to "why are we still having trouble with `task dev`":** the idle-kill (mechanism A) is fixed. This is a **different, independent gap** in a different layer (D) that the same investigation surfaced — not a regression of the A fix, and not something the heartbeat pattern touches at all, since it's about *tracking-record persistence across a harness restart*, not about the underlying process's liveness. `task dev` itself was fine the whole time; the reporting layer wasn't.
- **Status:** diagnosed, no fix attempted — this is inside the Claude Agent SDK/harness, not this repo, so there is nothing in `agentmux`'s own source to change. Worth knowing as an operational fact: **always cross-check `tasklist`/`muxlog` directly before trusting a "no completion record"/"failed" notification for a long-lived background process** — the notification layer can be wrong in exactly this way.

---

## 5. Mechanism E — the agent pane can get stuck showing "Working" indefinitely

This is the user's newly-reported regression. Investigated in full; **the fix already exists, fully designed, and has simply never been implemented.**

### 5.1 How "turn done" is detected today

Claude Code runs as a long-lived persistent process for the whole pane session — it never exits between turns, so there's no real per-turn "done" signal from the CLI itself. `claude-translator.ts`'s `handleAssistantMessage` (originating from PR #1757) synthesizes one: a non-partial assistant message with **no `tool_use` block** is treated as `session_end`.

### 5.2 The gap

The current heuristic is `!hasToolUse` — it doesn't check whether the message contains any real text. A message that is *only* thinking, or entirely empty (a transitional/incomplete message — e.g. a message boundary lands between a tool result and the model's real reply), gets misclassified as "done." `TurnPhase` settles to `Done.completed`, the UI shows "Worked" — and then, when the real reply actually streams in moments later, the reducer's own `StreamFlushObserved`/`bumpEvent` logic correctly re-promotes `Done.completed → Streaming` (by design — `session_end` can legitimately mean "round 1 of N," not "the whole turn is over"). `settled-grace.ts`'s `shouldNotifyOnReopen` fires the `"Picked up more work — starting another round…"` notification whenever that re-promotion happens ≥500ms after the (premature) `Done.completed` — exactly the notification the user is seeing.

### 5.3 Why it gets stuck, not just flaps briefly

`settled-grace.ts` and the reducer's re-promotion logic are *reporting* the re-promotion correctly — they are not the bug. The actual recovery path for a genuinely-stuck `Streaming` phase is `StreamWatchdogTick`'s liveness recovery: force-recover to `Idle` after `LIVENESS_RECOVERY_MS = 180_000` (3 minutes) of **continuous silence** (`reducer.ts`, `types.ts:750`). But any subsequent real activity — a new tool call, a new chunk of text — resets the silence clock (`lastEventMs`) and restarts the 3-minute countdown from zero. During a long session where `ScheduleWakeup`/task-notification-driven resumptions keep arriving (as happened repeatedly in this exact conversation), each new round's tool activity can refresh `lastEventMs` before the prior window elapses — **perpetually deferring the watchdog's recovery for as long as new activity keeps arriving close enough together.** The result: the notification fires once (correctly reporting a real re-promotion), and then "Working" never clears, because the *specific episode* that triggered it never gets its own resolution — only a generic silence timer does, and that timer keeps getting reset by unrelated-but-real subsequent activity.

### 5.4 The fix already exists — and, correction, was already merged

`SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md`, dated the day before this investigation, was checked against the working tree at the time and showed status **"Proposed — audited and designed, not yet implemented."** This session's first report (below, §8, and the earlier chat response) took that status at face value and recommended implementing it. **Correction:** it had, by that point, already been implemented and merged — `PR #2369` ("fix(agent-stream): require real explanation text before ending a persistent-mode turn"), merged **2026-07-30T14:18:18Z**, exactly the `!hasToolUse && hasText` gate the spec describes (confirmed directly in `claude-translator.ts`'s current `handleAssistantMessage`, which cites this same spec file in its own comment). The spec file itself was simply never updated to reflect its own implementation — a docs-hygiene gap, not a code gap.

**Practical consequence:** PR #2369 merged *after* the `v0.54.7` release cut (2026-07-29T's `VERSION_HISTORY.md` entry). So the fix exists on `main` but is not present in any *released* build — a user's regular installed AgentMux instance won't have it until the next release ships, even though a fresh `dev` build off `main` right now does. This is the most likely explanation for why the stuck-"Working" symptom was still observed live during this investigation: the running instance being watched was on `v0.54.7`, predating the fix.

Per that spec's own §4 ("Why this is safe"): every genuinely-finished turn ends with `stop_reason: "end_turn"`, which by definition carries real text, even a one-word reply — so this doesn't reintroduce the original #1757 "stuck forever" bug it replaced. Its own §6 ("Non-goals") is honest that it reduces, not eliminates, the notification's firing rate (Claude Code's own `Stop`-hook-driven mid-CLI resumption is a separate, unobservable-from-AgentMux case) — so some residual "Picked up more work…" firing is still expected even after this fix, just not the specific spontaneous-and-permanently-stuck pattern this investigation chased.

---

## 6. Recommended direction: one shared, generation-tagged state machine, not five independent trackers

The user's framing — "we need a state machine for all these things" — is the right instinct, and there's a working template for it already in this exact repo, shipped this same session: `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs` (PR #2371/#2373). That module replaced four independently-mutated fields on `PersistentInner` (each maintained by a different concurrently-scheduled task, each capable of disagreeing with the others about "is this generation still retryable") with:

- **One state enum** (`ResumeState`: `NotTracking` / `AwaitingOutcome` / `ConfirmedRetry`), so "is this still live" is always a single, unambiguous question.
- **Generation-tagged events** — every event carries the exact spawn generation it was observed for, so a stale event from an already-superseded generation can never corrupt newer state (`update()`'s own module doc: "a stale event from an already-resolved generation can never corrupt a later, unrelated generation's state").
- **A pure `(state, event) -> (state, effects)` transition function** — fully unit-testable without real timing, real processes, or real I/O; the caller executes the returned effects.

Mechanisms A–E in §1 are the frontend/process-tracking analogue of exactly the bug class that module was built to close: several independently-updated trackers (bashwrap's PTY clock, the dock's transcript-derived status, the registry's Job Object membership, the harness's own session bookkeeping, the pane's turn-phase reducer) that can each independently believe something different about the same underlying question ("is this thing still going"), with no generation/epoch concept tying a specific launch to a specific tracking record — which is exactly why a stale record (an orphaned `ToolNode`, a lost background-task bookkeeping entry, a perpetually-reset watchdog) can outlive the thing it was supposed to represent.

**A full unification (one registry, one set of events, one pure reducer, feeding *all* of the dock/registry/pane-status/background-task surfaces) is a substantial redesign** — not proposed as a single PR here. The concrete, incremental steps that move in that direction, roughly in the order they'd naturally land:

1. ~~Implement the already-designed `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md` fix~~ — **done; see §5.4 correction.** Merged via PR #2369, 2026-07-30, before this spec was even finished being written. Not yet in a released build (post-dates `v0.54.7`).
2. Add the still-missing "long-running attached process" pane status (`retro-persistent-agent-working-status-stuck-2026-07-16.md`'s own "Fix direction") — this is the one axis every mechanism-E investigation (that retro, and this one) has independently identified as the real missing state: *turn-phase* and *attached-background-process-liveness* are orthogonal, and the reducer currently only models the former.
3. Give the dock (mechanism B) a generation/max-age fallback so an orphaned `"running"` entry can self-expire instead of waiting for a session boundary — directly closes §3.
4. Longer-term: consider whether the dock (B) and the process registry (C) could share one underlying event-sourced model instead of two independently-derived views over different data — `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` already recommends this direction; this spec adds mechanisms A, D, and E to the same argument.

---

## 7. Open items (not fixed by this spec, flagged for follow-up)

- [x] ~~Implement `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md`~~ — already merged (PR #2369), see §5.4.
- [ ] Bake the heartbeat-loop pattern (§2) into `scripts/dev-agent.cmd` itself, or document it prominently in `CLAUDE.md`, so it isn't rediscovered a third time.
- [ ] Give `scrubOrphanedInProgress` (or a new mechanism) a way to expire an orphaned `"running"` `ToolNode` sooner than the next session boundary (§3).
- [ ] Mechanism D (§4) has no fix available from inside this repo — document as a standing operational caveat: cross-check ground truth (`tasklist`/`muxlog`) before trusting a background-task "no completion record"/"failed" notification.
- [ ] Add the distinct "long-running attached process" pane status (§6, item 2).
- [ ] Longer-term architectural convergence of B/C (§6, item 4).

---

## 8. Immediate action — corrected

~~`SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md` is fully designed... recommend implementing it now.~~ **Superseded — already merged, see §5.4.** No frontend code change is needed for this specific item. Practical follow-up instead: when a new release is cut, confirm `v0.54.7`'s successor includes PR #2369 (`VERSION_HISTORY.md` entry should list it), and if the stuck-"Working" symptom is reported again against a build that already includes it, that's a signal item 2/3 above (not this fix) is the remaining gap, per §5.4's own residual-scope note (Claude Code's own `Stop`-hook-driven resumption is unaffected by this fix).
