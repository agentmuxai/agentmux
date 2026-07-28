# Report: "Worked" silently reverting to "Working…" with no user input, and a pane stuck on an unanswerable AskUserQuestion

**Date:** 2026-07-27
**Author:** Agent1
**Status:** Audit only — root causes identified/hypothesized from code, one live-confirmed via direct observation (Agent2's pane), fixes proposed, nothing implemented yet.
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

---

## 3. Key files

| Concern | File | Line(s) |
|---|---|---|
| `Done.completed` → `Streaming` re-promotion (§1) | `frontend/app/store/agent-pane-state/reducer.ts` | ~198-256 |
| Watchdog/liveness constants, `TurnPhase` shape | `frontend/app/store/agent-pane-state/types.ts` | full |
| AskUserQuestion queue + answer handler (§2) | `frontend/app/view/agent/hooks/useAgentQuestions.ts` | full |
| Question panel render | `frontend/app/view/agent/components/AgentQuestionPanel.tsx` | full |
| Tail-`awaiting_answer` preservation heuristic (§2.2) | `frontend/app/store/agent-document/reducer.ts` | 38-137 |
| Stream-parser: where `awaiting_answer` is first set | `frontend/app/view/agent/stream-parser.ts` | ~424-438 |
| Original AskUserQuestion spec | `docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md` | full |
| Sibling stuck-Working audit (9 other false-positive paths) | `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` | full |
| Sibling fire-once-event report (structurally same race shape as §2.3) | `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` | §5a |

## 4. What this report does not do

- Does not implement either fix — both need a design decision confirmed with the user first (§1.3's grace window + "new episode" presentation; §2.5 pending live confirmation of the actual root cause).
- Does not re-litigate the 9 paths already cataloged in `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` — this report's §1 is a *tenth*, distinct path (a turn re-opening after reaching a real terminal state, not a turn that never terminates).
- Does not confirm §2's root cause live — flagged explicitly as the next required step before sizing that fix.
