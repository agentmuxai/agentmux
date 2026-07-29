# Report: "Worked" silently reverting to "Working…" with no user input, and a pane stuck on an unanswerable AskUserQuestion

**Date:** 2026-07-27
**Author:** Agent1
**Status:** §1 audit-only, fix proposed, not implemented. §2 (Agent2's stuck pane) — live-confirmed post-merge (§2.7): the CLI subprocess itself had gone unresponsive (`Dead` per the backend health monitor), not a frontend-only race; recovered by closing/reopening the pane. §2.8 — a second, distinct, code-confirmed bug (answering a question can never succeed after any process respawn) is **fixed and merged**. §4 (surface/auto-recover a `Dead` persistent agent before the user has to notice) — the minimum-viable version (surface a "Restart" recovery row; auto-restart stretch goal deliberately NOT built) is **implemented and merged**, see §4's own status line.
**Triggered by:** two live incidents observed in the same session — (1) Agent2's pane (block `210a0e08-1740-4bf0-8f26-93c82a107e4c`), running concurrently in an adjacent pane, visibly stuck on an AskUserQuestion prompt with no way to resolve it; (2) a separate, repeated observation that a pane shows "Worked" and then reverts to "Working…" with no user message sent in between.

This report is a companion to two same-day documents already in the repo — `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` (catalogs 9 other false-"Working" paths, mostly about a turn never *ending*) and `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` (the fire-once `controllerstatus` event gap). Neither covers the two issues below: this report is about a turn that legitimately *did* end, then un-ends itself, and about a specific stuck-forever panel shape distinct from a stuck turn.

---

## 1. "Worked" reverting to "Working…" with no user input

### 1.1 Root cause — confirmed, and it's intentional by design

`frontend/app/store/agent-pane-state/reducer.ts`, the `StreamFlushObserved` case (~line 198-256). When a live stream flush arrives while `turnPhase` is `Done` with `outcome === "completed"` (i.e. the pane is showing "Worked"), the reducer promotes it straight back to `Streaming`:

```ts
: state.turnPhase.kind === "Submitting"
    || state.turnPhase.kind === "Idle"
    || state.turnPhase.kind === "Disconnected"
    || (state.turnPhase.kind === "Done" && state.turnPhase.outcome === "completed")
    ? {
          kind: "Streaming",
          bufferSize: newBuf,
          toolsActive: 0,
          lastEventMs: command.at,
      }
    : state.turnPhase;
```

The inline comment explains why this exists: `session_end` fires after **every** model API round, not just the true end of a turn — so `Done.completed` can mean "round 1 of a multi-round tool-continuation just finished," and if a second round's output starts streaming in, the reducer *has* to leave `Streaming`-land or the working indicator would read "Worked" while the agent is visibly still typing. `StreamFlushObserved` is documented as only ever dispatched for **live** stream content, never history replay — so today's authors believed a flush arriving in `Done.completed` is airtight proof of genuine continued work. `Done.errored` / `Done.stopped` / `Done.interrupted` are deliberately excluded from this promotion (a late stray flush after a failed/stopped turn must not resurrect it) — only `.completed` re-arms.

**This is not a bug in the sense of "wrong code" — it's a real design tension.** Multi-round tool continuations are a genuine backend behavior (the CLI legitimately emits `session_end` mid-conversation), and the reducer's job is to represent that faithfully. The user-visible problem is a UX one: **the pane shows a settled "Worked" state for a span of time that, from the backend's point of view, was never actually settled** — there was no way for the user to know that "Worked" might not be final. That's exactly the shape of the report: "we go into a completed state, then go back into Working… without the user having entered any input."

### 1.2 Why a blanket "never leave Done without user input" rule would break something real

Taking the user's literal ask at face value — "never let a Working… come back from a Worked without user input" — would require either:
- Disabling the `Done.completed` branch of the promotion above, which would leave the UI stuck showing "Worked" while the agent is demonstrably still producing output for any genuine multi-round continuation (a strictly worse bug: a *false* "done" during real work, which is worse than a false "still working" after real work — the former could make a user walk away mid-task).
- Or somehow distinguishing "this flush is a genuine continuation" from "this flush is stray/leftover" at dispatch time — but per the code's own comment, there is currently no such distinction available; `StreamFlushObserved` firing is already gated to live-only content.

So the fix can't be "delete the promotion" — it has to change **what the user is shown**, and **when**, without changing what the backend state machine internally tracks (session-digest correctness depends on `Done.completed` firing at the right point per-round, per `docs/specs/SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md`'s existing framing, referenced in the watchdog code nearby).

### 1.3 Proposed invariant: a "settled" grace window, decoupled from internal state

Introduce a short grace window (proposed: 400-600ms — long enough to absorb a same-breath continuation, short enough that a genuinely-finished turn still visibly settles almost immediately) between "internal state says Done.completed" and "the UI is allowed to call it truly finished and stop accepting silent re-promotion":

- Internal `turnPhase` keeps behaving exactly as it does today — no behavior change to the state machine, no risk to session-digest stats, no change to the promotion logic in §1.1.
- Add a UI-facing derived flag, e.g. `settled: boolean`, computed as "`turnPhase.kind === 'Done' && outcome === 'completed'` AND at least `SETTLE_GRACE_MS` has elapsed since the phase entered `Done` with no intervening `StreamFlushObserved`." This is a pure timer/derived-signal addition, not a reducer change — it can live alongside the existing watchdog-tick machinery (`StreamWatchdogTick` already runs a periodic check; a settle-check is the same shape at a much shorter interval, or a one-shot `setTimeout` armed when `Done.completed` is first entered and cleared if a flush arrives before it fires).
- Once `settled` has been `true` for a given `Done.completed` episode, a subsequent `StreamFlushObserved` should **still** promote `turnPhase` back to `Streaming` internally (so the underlying multi-round tracking stays correct) — but the UI layer should treat this specific case as a **new, visually distinct episode** ("Working again — new round") rather than silently reverting the "Worked" checkmark the user already saw settle. This satisfies the spirit of the ask — a *settled* "Worked" can never silently, invisibly un-happen — while still being honest that the agent picked up more work, instead of hiding it.
- Whether "new episode" should mean an actual audible/visual notification (distinct from the routine "Working…" ambient state) is a product decision worth confirming before implementing — flagged as an open question, not resolved here.

### 1.4 Sizing/risk

Small-to-medium. No reducer/type-machine changes required for the core fix (§1.3's `settled` flag is additive and derived); the only design risk is picking the right `SETTLE_GRACE_MS` value and deciding how "re-opened as a new episode" should look/sound to the user. Recommend confirming both with the user before implementing.

---

## 2. Agent2's pane stuck on an unanswerable AskUserQuestion

### 2.1 What was directly observed

Agent2 (block `210a0e08-1740-4bf0-8f26-93c82a107e4c`), running live in the pane adjacent to this one, is stuck showing an AskUserQuestion prompt (`AgentQuestionPanel`) that isn't resolving. Direct live inspection of Agent2's own backend state (its `db_agent_instances`/block meta row) was **attempted and not completed** in this pass — the `objects.db` reachable from this environment (`C:\Users\asafe\.agentmux\dev\main\data\db\objects.db`) turned out to hold a small, stale instance (3 blocks, 1 tab/window/workspace total) rather than the live multi-agent instance Agent1-5 are actually running in; the correct live DB's location wasn't located in this pass. So the analysis below is grounded in code reading, not a live state dump — flagged explicitly so it isn't mistaken for a confirmed root cause.

### 2.2 Code path

- `frontend/app/view/agent/hooks/useAgentQuestions.ts`: `pendingQuestions()` returns every `ToolNode` with `status === "awaiting_answer"` (oldest first); the panel renders the head. `handleAnswer()` optimistically flips the node to `status: "success"` via a local `StreamFlush` dispatch, then calls `RpcApi.AgentAnswerCommand`; only on a *caught* RPC failure does it roll the optimistic change back (or fall through to a follow-up-message delivery path for non-persistent agents).
- `frontend/app/store/agent-document/reducer.ts`, `scrubOrphanedInProgress()` (~line 38-137): on certain document rebuilds (reconnect/backfill/history-load), any `awaiting_answer` tool node that has real content **after** it in the document is resolved to `success` (the conversation clearly continued past it, so it must have been answered). But — deliberately, per the inline comment — **a TAIL `awaiting_answer` node (the very last node in the document) is left alone**, on the reasoning that "it may still be answerable... a one-shot agent that just asked, or a resumable reopened session."

### 2.3 Hypothesis — an optimistic-answer / resync race, same shape as the login-persist bug fixed earlier today

`handleAnswer()`'s optimistic transition lives only in the client's in-memory document state at first, applied via a local `StreamFlush` dispatch (actor `"user"`) *before* the RPC call is even sent, let alone confirmed durable server-side. If, after that optimistic flip, anything triggers a **full document resync from the server's own transcript** (a reconnect, a pane reopen, a backfill after a dropped WS connection — several of which are already-documented mechanisms elsewhere in this session's work, e.g. `session_backfill.rs`'s startup disk-scan) **before the answer RPC has actually landed and been durably reflected in that transcript**, the resync rebuilds the document from a transcript that still shows the question as `awaiting_answer`. Because that node is (still) the tail, `scrubOrphanedInProgress`'s deliberate "leave the tail alone" rule preserves it rather than resolving it — the optimistic client-side "success" is silently discarded, and the panel reappears/re-hangs on a question that, from the user's point of view, was already answered.

This is structurally identical to the root cause behind §5a of `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` (an optimistic/local state change that races a separate resync path reading from a not-yet-updated source of truth) and to the general theme surfaced by the earlier §4.1 session-continuity work this session did (a locally-applied change that never got durably written where a later reconciliation path looks for it). It is also consistent with the RPC's own documented failure mode: `AgentAnswerCommand` can legitimately fail with `UNSUPPORTED_CONTROLLER` for non-persistent/container agents, at which point the code deliberately does NOT roll back the optimistic "success" state (comment: "Keep the optimistic success") and instead tries a follow-up-message delivery — if *that* follow-up itself silently fails in a way that doesn't reach the `.catch` at line 125-128 (e.g., the message is accepted by the RPC layer but the backend never actually resumes the parked tool_use), the transcript would likewise still show `awaiting_answer` at the tail with nothing left client-side to indicate anything went wrong.

A second, narrower possibility: if Agent2 genuinely has **multiple** questions queued (`pendingQuestions()` returns all matching nodes, panel shows only the head), answering the visible one could look "stuck" to an observer if the *next* one in the queue renders with stale/wrong content, or if the queue's `waiting-for-input`/`waiting-ended` tone-event bookkeeping (lines 65-81) gets out of sync with the actual queue length — worth checking directly against Agent2's live document once reachable.

### 2.4 What would confirm this (next step, not done here)

Direct inspection of Agent2's live document/transcript state is the fastest way to disambiguate the above from a different mechanism entirely (e.g., a rendering-only bug in `AgentQuestionPanel.tsx` itself, unrelated to the document reducer). Locating the correct live `objects.db` (or querying via a running srv's own introspection RPCs rather than a raw file read, to avoid the direct-DB-write hazard already documented in this session — see `docs/specs/SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md`'s PR discussion) and checking whether the tail node's `awaiting_answer` status has a corresponding answer already recorded elsewhere (e.g., in `db_agent_history` or the transcript `.jsonl`) would directly confirm or refute §2.3.

### 2.5 Proposed fix directions (pending confirmation)

- **If §2.3 is confirmed:** the durability gap needs the same shape of fix as the earlier login-persist race — don't let the optimistic client-side transition be the only record of "this was answered" until the RPC has actually round-tripped; and/or make `AgentAnswerCommand`'s delivery synchronously durable (write the answer to the transcript/DB) before the client is allowed to treat it as settled, so a resync mid-flight sees the true post-answer state instead of racing it.
- Regardless of root cause, add a hard backstop: if a tail `awaiting_answer` node has been sitting unanswered past some generous threshold (e.g. several minutes) with the pane otherwise idle (no `Streaming` activity), surface *something* — even just a log line in the vein of report 1's proposed `[wave-turn]` telemetry — so this state is at least grep-able in `muxlog` instead of only visible by looking at the pane. This mirrors report 1's own top recommendation (watchdog reasoning telemetry) and would have made this exact incident diagnosable without needing live DB access.

### 2.6 Sizing/risk

Unknown until §2.4's live confirmation step is done — could range from a small durability/ordering fix (if §2.3 is right) to a rendering bug (if it's in `AgentQuestionPanel.tsx` itself, not yet audited in this pass). Recommend confirming live before sizing implementation work.

### 2.7 Live confirmation (post-merge, same day) — §2.3 was the wrong leading theory; the process itself had died

Direct evidence gathered after the fixes above merged, from `agentmuxsrv-v0.54.5.log.2026-07-28` (grep on the block id):

```
00:30:32.037  agent health transition   old=Dead  new=Healthy
00:30:32.044  inject: structured delivery to non-PTY controller (mid-turn steer)   target_agent=Agent2
00:31:03.204  agent health transition   old=Healthy  new=Stalled     (30s no output)
00:32:33.205  agent health transition   old=Stalled  new=Dead        (120s no output)
00:46:34.076  (last log line — 14 more minutes of total silence)
```

Two things this establishes, neither of which was in §2.1-2.6's hypothesis set:

1. **The backend's own health monitor already considered the process `Dead` at `00:30:32`**, immediately before an external message injection nudged it back to `Healthy` (optimistic — the injection path doesn't verify the CLI actually resumed producing output, it just marks recent activity). The process then genuinely never produced another byte of output, re-confirmed `Dead` 121s later. This means the CLI subprocess itself was hung/unresponsive — not a frontend-only document-resync race (§2.3's leading theory). §2.3 assumed a live, responsive backend racing a stale frontend snapshot; that's not what was happening here. §2.3 may still be a real, independent bug (the mechanism it describes is still plausible in principle), but it was not the cause of THIS incident.
2. **User-observed symptom, matching exactly:** attempting to answer the stuck question "flickers and comes back to the same state." This is `useAgentQuestions.ts`'s `handleAnswer()` working *correctly*: it optimistically flips the node to `success` (the flicker), sends `AgentAnswerCommand`, the RPC fails because there is no live process to deliver it to, and the failure-path rollback (`applyDoc(originals)`) correctly reverts the optimistic change rather than falsely claiming "answered." The rollback isn't the bug — it's the process being dead underneath it that made the rollback the only honest outcome.

**Root cause, revised:** something caused Agent2's CLI subprocess to stop producing output entirely while parked on the `AskUserQuestion` tool call — genuinely hung or crashed at the process level, not a state-sync artifact. *Why* the process hung is still unconfirmed — no CLI-side stderr/crash signal was captured before it was closed and reopened, so this remains open. Candidates worth checking first in a future recurrence, before it's closed: the CLI's own stderr for that block (if still on disk), and whether `agentmux-bashwrap`'s idle-kill (`docs/specs/REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md`) or a resource exhaustion event coincides with the death.

**Recovery, confirmed working:** closing and reopening the pane recovered it. This is expected — a fresh pane mount forces a controller resync/respawn — and note that **this specific recovery path is now materially better than it was before today's earlier PR** (`SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md`): the reopened pane now correctly attempts to resume the actual prior session (§4.1's session-id write-through fix) rather than silently starting fresh, and would have disclosed it via the `session:resume_failed` banner (§4.2) had the resume itself failed. Before today, a reopen after a dead persistent-agent process had a real chance of silently landing on a brand-new, unrelated session.

**What is NOT yet built — no automatic detection/recovery for a `Dead`-classified persistent agent.** Confirmed by reading `health.rs`: reaching `AgentHealth::Dead` only logs and publishes a diagnostic `agenthealth` WPS event (`publish_health`) — nothing kills, respawns, or even surfaces a "this agent is unresponsive, click to restart" affordance in the pane. The user has to notice the pane looks stuck and manually close/reopen it, exactly as happened here. This is the concrete gap between "this specific incident is now recoverable" and "this class of incident is now handled" — closing it is scoped as its own follow-up (§4).

### 2.8 The real gap in "reopen supports this" — fixed, same day

The user's direct follow-up question ("when an agent is at a question state and I close the pane, when I reopen, we need it to support the situation") pointed at a second, distinct, code-confirmed bug — separate from §2.7's process-death diagnosis, and this one **was** implemented:

`PersistentSubprocessController::answer_question` (`persistent.rs`) tracks pending questions in `pending_questions: HashMap<tool_use_id, (request_id, questions)>` — **in-memory only, scoped to one controller instance.** A fresh instance (created on every pane reopen, or any process respawn for any reason) starts with an empty map, even though the persisted transcript still correctly shows the question as the tail `awaiting_answer` node (`scrubOrphanedInProgress` deliberately preserves it as "may still be answerable" — see §2.2). Answering it after any respawn therefore ALWAYS fails at the backend with `"no pending AskUserQuestion for tool_use_id …"` — a different error shape than `UNSUPPORTED_CONTROLLER`, the only string `useAgentQuestions.ts`'s `handleAnswer()` checked for before falling back to redelivering the answer as a follow-up message. Every other failure just rolled the optimistic "answered" UI back to `awaiting_answer` — **this is the precise mechanism behind "it flickers and comes back to the same state," and it means a question surviving ANY pane reopen was permanently unanswerable through the UI, not just this one incident.**

**Fix (implemented, refined twice post-review):**
- `frontend/app/view/agent/hooks/useAgentQuestions.ts` — `handleAnswer`'s fallback is widened from the single `UNSUPPORTED_CONTROLLER` string to a `SAFE_TO_RETRY_VIA_FOLLOWUP` **allowlist** of backend error shapes that structurally guarantee the control_response was never sent (`"no pending AskUserQuestion"`, `"UNSUPPORTED_CONTROLLER"`, `"no controller for block"`, `"persistent process not running"`, `"control_response send failed"` — every one returned strictly before/instead of `tx.try_send`). Originally shipped as "fall back on ANY failure," but reagent P2 (round 1) correctly flagged that an RPC-engine-level timeout (`agentmux-srv`'s `EC-TIME:` — `tokio::time::timeout` under executor saturation) does NOT carry that guarantee: the handler could complete `tx.try_send` successfully server-side even though the client sees an error, so blindly retrying could deliver the answer twice. An unrecognized error now falls through to the original conservative rollback instead of guessing.
- **Round 2 (reagent P1):** the fallback's own `.catch()` — meant to roll back if the follow-up delivery *itself* also failed — was dead code: `opts.sendMessage` (→ `handleSendMessage` → `useAgentCommands.ts`'s `deliverToBackend`) swallows an `AgentInputCommand` RPC failure in its own catch (dispatches `PendingMessageRejected`/`TurnStartFailed` for its own UI signal) and returns *without rethrowing*, by design — most callers fire it and forget. A `.catch()` on that promise therefore never runs, so a genuinely failed follow-up would have silently left the optimistic "success" state in place with the answer never delivered. Removed the false claim; the fix now honestly keeps the optimistic state once a fallback is attempted (matching the pre-existing Phase 2/`UNSUPPORTED_CONTROLLER` contract, which had this exact limitation already — not a regression). The follow-up path (`send_message` on the backend) still auto-spawns/resumes a dead process if one isn't running, so it self-heals a genuinely dead process too, not just the reopened-but-stale-in-memory-record case, for every error shape where retrying is structurally safe.
- `agentmux-srv/src/backend/blockcontroller/persistent.rs` — `answer_question`'s "not found" error message explicitly names the likely cause (process respawn) for `muxlog` diagnosability; its doc comment now correctly states the frontend matches on this error's text rather than claiming string-independence.
- Tests: `useAgentQuestions.test.ts` (4 cases — success path has no fallback; a known-safe backend error falls back and keeps the optimistic state; the optimistic state is kept even if the follow-up delivery itself fails, since that failure is undetectable through this API; an unrecognized/EC-TIME-shaped error rolls back immediately without attempting the fallback) and `answer_question_on_untracked_tool_use_id_is_descriptive` (backend, asserts the error message names the tool_use_id and the likely cause).

This directly answers "will future agents support it": **yes, for the answer-delivery half of the problem** — a question that survives a pane reopen (or any process respawn) can now actually be answered through the UI. It does **not** cover §4's separate gap (no automatic surfacing/recovery when a process goes fully `Dead` before the user notices) — that remains open, scoped below.

## 4. Surface (implemented) + auto-recover (deliberately NOT built) a `Dead` persistent agent

**Status: minimum-viable fix implemented and merged.** The auto-restart stretch below remains explicitly out of scope — it needs a product decision, not just engineering, per the original scoping.

**What shipped:** `HealthMonitor` (`agentmux-srv/src/backend/blockcontroller/health.rs`) now takes `wstore`/`event_bus` (threaded through all three call sites: `persistent.rs`, `subprocess/mod.rs`, `acp.rs`). `evaluate_and_transition()` — the single choke point for every health-state change — surfaces a `Dead` transition as a new `FailureClass::Unresponsive` `AgentFailure` (`agentmux-srv/src/agents/failure.rs`; `retryable: false`, since the process is alive-but-wedged, not exited — a plain "re-send the last message" retry wouldn't reach anything). Publishes via the exact same durable-persist-then-ephemeral-WPS-push pattern the exit-classification path already uses (`persist_last_failure` then `EVENT_AGENT_FAILURE`, `persist: 1`). The frontend (`failure-accessory.ts`) renders it with a new "Restart" action wired to `ControllerResync{forcerestart: true}` — the exact mechanism `forceControllerRefresh` already used internally for the post-login stale-process case, now exposed on `useAgentControllerStatus`'s public interface so the failure row can reuse it (`agent-view.tsx`'s `useAgentFailure({...})` call site wires `onRestart`).

**Self-heal handling (a gap identified during research, closed before shipping):** `compute_health()`'s silence-based Dead check has no hysteresis — late output arriving just after the 120s threshold tripped flips health directly back to `Healthy`, silently. Without handling this, a stale "Restart" button would linger over a process that's actually fine again. `evaluate_and_transition()` now also detects `Dead -> anything else` and publishes a `data: None` clearing event on the same `EVENT_AGENT_FAILURE` channel; the frontend (`useAgentFailure.ts`'s WPS handler) treats a null-data event as "clear the failure, but only if it's currently classed `unresponsive`" — so it can never clobber an unrelated concurrent failure (e.g. auth) that happens to be showing.

**Post-review fixes (3 rounds of reagent findings, all legitimate):**
- **Round 1, P1 — lock-ordering race:** `evaluate_and_transition` originally dropped `inner`'s lock before publishing/persisting. Since it's called concurrently from the 5s watchdog tick and `record_output`/`record_error` on the stdout-reader task, two interleaved invocations could race their publish calls out of order — e.g. a watchdog-observed Dead→failure publish landing on the wire *after* a stdout-reader-observed Dead→healthy clear, leaving a stale "Restart" banner over an already-recovered process (exactly the staleness the self-heal handling above was meant to prevent). Fixed by keeping `inner`'s guard held across every side effect the transition causes, not just the state mutation — makes the whole transition one atomic critical section, so concurrent invocations' publishes are strictly ordered the same way their state mutations are. Safe: none of the calls involved are async and none re-lock `self.inner`.
- **Round 1, P2 — misleading log message:** `forceControllerRefresh`'s catch-block log was hardcoded for its original login-recovery caller ("signed in, but couldn't refresh..."); reused unchanged by the new Restart action, an RPC failure during a restart click would have logged a message about signing in that had nothing to do with what actually happened. Added a `context: "login" | "restart"` parameter (defaults to `"login"`, so every pre-existing call site is unchanged) that picks the right message.
- **Round 2, P1 — wrong class for a recognized in-band error:** `Dead` has two root causes — genuine silence, and a fatal in-band error (e.g. an auth failure printed to stderr) that didn't make the process exit. The original code labeled BOTH cases "Unresponsive"/"Restart" — actively wrong for the second: a restart doesn't fix an auth problem, and none is even needed (the running process re-reads its credential per request), so showing "Restart" instead of "Login Again" would mislead the user for as long as the process stayed alive. Fixed by classifying `inner.last_error` through the same `agents::failure::classify()` used for exit-time classification whenever `errors.has_fatal()` is true, publishing the correctly-classified failure instead of a blanket Unresponsive.
- **Round 2, P1 — the feature never triggered for ACP-backed panes at all:** `AcpController` had `wstore`/`event_bus` wired into its `HealthMonitor` but never actually spawned the periodic watchdog (`send_input` called the plain `set_active_turn(true)`, not the watchdog-arming `mark_turn_active_returning_was_active`) and never called `record_output` from its stdout reader — so `evaluate_and_transition` could never observe silence, meaning a genuinely wedged ACP process would hang forever with zero signal, the exact bug this whole feature exists to close, just for a different controller type. Fixed by swapping to the atomic mark-and-arm idiom (mirroring `persistent.rs` exactly) and adding a `record_output(true)` call for every non-empty stdout line.

**Tests:** backend — `dead_transition_publishes_unresponsive_failure` (now exercises `publish_unresponsive_failure` directly — that class is only reachable in production via the 120s silence branch, not practical to wait out in a test), `dead_via_recognized_fatal_error_publishes_the_correct_class_not_unresponsive` and `dead_via_unrecognized_fatal_error_falls_back_to_classifys_own_default` (the round-2 classification fix), `dead_recovery_clears_the_unresponsive_failure`, and `concurrent_transitions_never_leave_a_stale_publish` (`health.rs` — the last hammers `evaluate_and_transition` from 8 concurrent OS threads and asserts the final published event always agrees with the final health state, deterministically true with the lock-ordering fix, not just probably). New `acp.rs` test module (`send_input_marks_the_turn_active`, `repeated_send_input_while_active_does_not_error`) pins the watchdog-arming call-site contract — full watchdog-tick coverage isn't practical without spawning a real process; `health.rs`'s own tests cover the watchdog mechanics once armed. Frontend — a new `failureToRow` case test (Restart is primary, no plain Retry offered) and two new `useAgentFailure.ts` tests (null-data event clears an `unresponsive` row; does NOT clear an unrelated one).

**Stretch, still explicitly NOT built (needs a product decision):** auto-restart without waiting for a click, for the specific case of a pane parked on an unanswerable `AskUserQuestion` with a `Dead` process — since in that state there's no in-flight work to lose, an automatic respawn is arguably safe by construction. Riskier for a `Dead` classification reached mid-generation, where a tool might genuinely still be slow rather than truly hung — if pursued, scope the auto-restart to the parked-on-a-question case specifically rather than all `Dead` transitions.

---

## 5. Key files

| Concern | File | Line(s) |
|---|---|---|
| `Done.completed` → `Streaming` re-promotion (§1) | `frontend/app/store/agent-pane-state/reducer.ts` | ~198-256 |
| Watchdog/liveness constants, `TurnPhase` shape | `frontend/app/store/agent-pane-state/types.ts` | full |
| AskUserQuestion queue + answer handler (§2) | `frontend/app/view/agent/hooks/useAgentQuestions.ts` | full |
| Question panel render | `frontend/app/view/agent/components/AgentQuestionPanel.tsx` | full |
| Tail-`awaiting_answer` preservation heuristic (§2.2) | `frontend/app/store/agent-document/reducer.ts` | 38-137 |
| Stream-parser: where `awaiting_answer` is first set | `frontend/app/view/agent/stream-parser.ts` | ~424-438 |
| `Dead` health classification + Unresponsive failure publish/clear (§2.7, §4) | `agentmux-srv/src/backend/blockcontroller/health.rs` | `evaluate_and_transition`, `publish_unresponsive_failure`, `clear_unresponsive_failure` |
| `FailureClass`/`AgentFailure` taxonomy (§4) | `agentmux-srv/src/agents/failure.rs` | `FailureClass::Unresponsive` |
| Restart action UI + row rendering (§4) | `frontend/app/view/agent/failure/failure-accessory.ts` | `unresponsive` case, `ICON` |
| Failure row hook — WPS subscribe, null-data clear handling (§4) | `frontend/app/view/agent/hooks/useAgentFailure.ts` | full |
| `forceControllerRefresh` — the restart mechanism, exposed for reuse (§4) | `frontend/app/view/agent/hooks/useAgentControllerStatus.ts` | full |
| Original AskUserQuestion spec | `docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md` | full |
| Sibling stuck-Working audit (9 other false-positive paths) | `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` | full |
| Sibling fire-once-event report (structurally same race shape as §2.3) | `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` | §5a |
| Pane close/reopen continuity guarantee — the mechanism that actually recovered this incident | `docs/specs/SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md` | full |

## 6. What this report does not do

- Does not implement §1's settled-grace fix or §4's `Dead`-recovery follow-up — both need a design decision confirmed with the user first.
- Does not re-litigate the 9 paths already cataloged in `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` — this report's §1 is a *tenth*, distinct path (a turn re-opening after reaching a real terminal state, not a turn that never terminates).
- Does not determine why Agent2's CLI subprocess actually hung (§2.7) — the process was closed and reopened before any CLI-side stderr/crash evidence could be captured, so the proximate trigger remains unknown. If it recurs, capture that evidence before recovering, if it's safe to wait.
