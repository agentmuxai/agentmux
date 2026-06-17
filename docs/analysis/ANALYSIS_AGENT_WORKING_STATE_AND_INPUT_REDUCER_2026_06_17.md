# Analysis: agent-pane working-state desync, input-reducer Esc edge cases, and state cohesion

**Date:** 2026-06-17
**Author:** smike
**Status:** Analysis (no code) — findings + prioritized recommendations
**Trigger:** Live incident — an agent ("Smark") in a 0.46.1 pane showed **not "in progress"
while it was actively streaming output**. Also covers the conversation input reducer, Esc
semantics, animation, and architectural cohesion vs. the pane-accessories + AskUserQuestion work.
**Areas:** `frontend/app/store/agent-pane-state/*`, `frontend/app/view/agent/*` (composer, footer,
question panel, accessories), `agentmux-srv/src/backend/blockcontroller/health.rs`

---

## 0. The incident (what actually happened, from the live logs)

From the running 0.46.1 sidecar log for Smark's block (`236e085f`):
- Each turn: spawn `claude` CLI → emit the **identical init bytes** (1699+156+156+239) → **silence
  for minutes** → recover in a **burst** (e.g. `16:53:40` a dozen stream lines in ~170 ms; `16:55:46`
  a 16 KB line). **No error lines** — the process was alive and *waiting*.
- Backend health cycled `Healthy → Stalled (≈30 s) → Dead (≈120 s)` and back.
- **All three agents** (Naki, Smike, Smark) stalled in the *same* window → a **shared upstream
  cause**: the model API was overloaded / rate-limiting the account; the CLI sat in retry/backoff
  for minutes, then the response landed.

That upstream stall is benign and self-healing. The **bug** is what the UI did during it.

---

## 1. The working-state bug — root cause (verified in code)

The "in progress" indicator can read **false while the agent is actively streaming**, due to a
**phase-promotion gap on stream resume** (and a secondary indicator conflation).

> **Correction (verified):** an earlier draft blamed the streaming-idle watchdog
> (`StreamStalled → Done.errored("stream-stalled")`, reducer.ts:698-724). That code is **unwired
> dead code** — `schedule-stream-watchdog` / `StreamStalled` appear *only* in `types.ts`,
> `reducer.ts`, and `reducer.test.ts`; **nothing in production consumes the schedule event or
> dispatches `StreamStalled`**. So the watchdog never fires and is *not* the cause. The real cause
> is below. (Recommendation: delete or actually wire the dead watchdog — see §5.)

### 1.1 PRIMARY: resumed live content does not re-enter the working phase after a stream drop

The only production paths out of a working phase are `TurnEnd` (→ `Done`), `StreamUnsubscribe`
(→ `Disconnected`), and the interrupt path. During a long upstream stall the agent gets
**killed + respawned** (observed: health `Dead` → SIGINT → respawn on the next message), which drops
and re-opens the stream. On reconnect:

- `StreamUnsubscribe` from a working phase → **`Disconnected`** (`reducer.ts:167-214`), and clears
  `lastEventMs`.
- `StreamSubscribe` from `Disconnected` → **`Idle`** (`reducer.ts:141-142`, PR F — "the lost turn is
  gone"), and re-sets `lastEventMs`.

Then the resumed response streams in — but the **live-activity commands only promoted to `Streaming`
from `Submitting`**, never from `Idle`/`Disconnected`:

`reducer.ts` `StreamFlushObserved` (before fix) and `bumpEvent` (before fix)
```ts
// Other phases (Idle, Done, Disconnected, Interrupting) keep their shape —
// "flushes outside a turn are ambient."
... : state.turnPhase.kind === "Submitting" ? { kind: "Streaming", ... } : state.turnPhase;
```

So content flushed into the document while the phase sat at `Idle`/`Disconnected`. `isWorking()` is
`Submitting | Streaming | Interrupting` (`types.ts:288-302`) → `isWorking(Idle) === false` → **the
"in progress" indicator stayed off while output streamed.** Exact match for the incident.

Crucially, `StreamFlushObserved`/`ToolStart`/`TokensIn`/`TokensOut` are dispatched **only for live
stream events** (`useAgentStream.ts`), never for history replay — so "a flush arrived" unambiguously
means the agent is producing output. The fix (§1.3) promotes to `Streaming` from `Idle`/`Disconnected`
too: **observable live content ⇒ working.**

### 1.2 SECONDARY: the indicator conflates two orthogonal lifecycles

The spinner/working row reads the **OR of controller-launch state and turn-execution state**:

`agent-view.tsx:1185` (and the `AgentWorkingRow` at :1056)
```ts
loading={ status.isLoading() || workingFromPhase(agentAtoms().turnPhaseAtom[0]()) }
```
`status.isLoading()` is `flowRunning() || !agentReady()` (`hooks/useAgentControllerStatus.ts:100`) —
**controller launch/auth/spawn**, unrelated to whether a *turn* is running. This causes a brief
**off-flicker during turn startup** (launch settled `false`, phase not yet `Submitting/Streaming`),
and conceptually muddies "is a turn in progress." The working indicator should derive from
**`turnPhase` only**.

### 1.3 The fix (implemented)

1. **Live activity re-enters `Streaming` from `Idle`/`Disconnected`** (not just `Submitting`), in
   both `StreamFlushObserved` and `bumpEvent` (`ToolStart`/`TokensIn`/`TokensOut`). Since these
   commands fire only for live stream content, observable content is proof of work — promote to
   `Streaming`. `Done` (completed turn) and `Interrupting` (user stopping) are intentional and kept.
   Reducer tests added for the reconnect→resume path.
2. **(Optional, not shipped) indicator derives from `turnPhase` only** — drop `status.isLoading()`
   from the working indicator. This is a *secondary* polish (a brief startup flicker, §1.2), not the
   reported bug; deferred to avoid touching the launch-spinner UX.
3. **(Follow-up) delete or wire the dead `StreamStalled` watchdog** — it's emitted but never
   consumed; either remove it or actually schedule the timer if a bounded give-up is wanted.

---

## 2. Esc / clear / cancel — the input reducer's edge cases

Esc is **layered** (good): autocomplete-Esc → question-panel-Esc → composer-Esc. The composer is an
**uncontrolled textarea** (`AgentFooter.tsx:438`); turn-phase + pending messages live in the reducer.

### 2.1 Esc behavior matrix (current)

| Surface / state | Esc does | Source |
|---|---|---|
| Autocomplete dropdown open | dismiss dropdown (guards the rest) | `AgentFooter.tsx:671-675` |
| Composer has text | clear text + exit history nav | `AgentFooter.tsx:732-743` |
| Composer empty | `onStopAgent()` → SIGINT/`RequestStop` | `AgentFooter.tsx:746` |
| Question panel visible | **minimize/defer** (not answer, not close) | `AgentQuestionPanel.tsx:150-161` |

### 2.2 Kinks (real, low-severity)

- **Empty-Esc fires stop even in `Submitting` (pre-ack).** `onStopAgent` dispatches `RequestStop` +
  `ControllerInputCommand(SIGINT)` regardless of phase; in `Submitting` the backend hasn't spawned the
  subprocess, so the SIGINT lands nowhere (spurious "stop failed"). Esc should **consult the phase**:
  no-op on `Idle`/`Submitting`-pre-spawn, SIGINT only on `Streaming`/`Interrupting`.
- **Question-panel "Other" field has no IME guard**, and Esc there **minimizes the whole panel**
  rather than clearing the field first — surprising while typing a free-text answer.
- **Held-queued messages aren't guaranteed to flush on `Interrupting → Done`.** `flushHeldMessages()`
  drains at tool boundaries; if a turn is interrupted/stalls out, queued messages can linger with no
  defined drain point (`useAgentCommands.ts` held-queue).
- **Double-submit during `Submitting`** isn't blocked at the composer; it relies on the caller gating
  the send button. The composer should disable/guard send while `workingFromPhase(turnPhase)`.

---

## 3. Animation — solid

- **Working spinner** (`AgentComposerStrip.tsx:210` / `_composer-strip.scss:61-91`): clean keyframe
  spin with a deliberate **decelerating** variant on stop; `prefers-reduced-motion` respected. The
  animation is correct — it's the `loading` *input* (§1.2) that's wrong.
- **New-message enter** (PR #1212): `@starting-style` + an `animateEnabled` gate that forces a style
  resolution (`void scrollRef.scrollTop`) so **history rows don't animate** and only **streaming rows
  do** (`AgentDocumentVirtualList.tsx:131-141`). Careful and correct.
- **Context-fill pulse** at ≥90% — fine.

Verdict: animation quality is good and reduced-motion-aware. No changes needed beyond fixing the
state that gates the spinner.

---

## 4. Architectural cohesion — strong core, frayed edges

### 4.1 What's genuinely cohesive
- **Reducer-first, derive-don't-duplicate.** `turnPhase` + document nodes are reducer-owned;
  accessories (`ActivityDock`, `ForkBar`, decision/question panels) **derive from document nodes** —
  the forks spec's "a pin source is a pure function of a source of truth, never a parallel store"
  rule (`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §5.3`) is *actually followed*
  (`ActivityDock.tsx:46` → `shellActivities(nodes())`).
- **AskUserQuestion plugs into the same model** — it's the `awaiting_answer` **ToolNode status** +
  `node.question` (`types.ts:233,249`), derived by `pendingQuestions()` scanning the document
  (`agent-view.tsx:496-505`). No parallel store. Answer delivery is an *intentional* optimistic
  update→RPC→reconcile/rollback (`agent-view.tsx:752-804`), documented in
  `SPEC_ASK_USER_QUESTION_2026_06_15.md`.
- **Reducer discipline holds** — composer, question panel, and dock own only *transient* local
  signals (`elapsedMs`, `selected[]`, `minimized`, `dismissed`) and route real mutations through
  `dispatch*` (the `recordDispatch`/"no `set*` from components" rule, `state.ts` header).

### 4.2 Where it's ad hoc (the seams)
1. **"Is the agent busy?" has two derivations** — `workingFromPhase(turnPhase)` vs. the indicator's
   `|| status.isLoading()`. This OR is the §1.2 bug surface and the one place "busy" isn't single-source.
2. **The frontend turn-phase machine and the backend health machine disagree on "stalled"** —
   backend `Stalled` is recoverable; frontend makes it terminal `Done.errored` (§1.1). They should
   share one mental model (stalled = paused-but-live).
3. **Esc isn't phase-aware** (§2.2) — the layered handlers consult *surface* state but not turn phase.
4. **Three independent pending queues** — decisions (`pending_approval`), questions
   (`awaiting_answer`), and queued user messages (`pending[]`) are scanned separately. Consistent, but
   there's no unified "pending interactions" derivation, and the alert region (decision/question/
   disconnected) arbitrates ad hoc.
5. **Per-alert-surface transient state is duplicated** — question and decision panels each reinvent
   `minimized` + selection signals; a shared alert-surface helper would dedupe.

### 4.3 Verdict
The architecture is **cohesive at its core** — the pane-accessories and AskUserQuestion work both
respect the reducer-owned, derive-from-source-of-truth pattern. The problems are **localized to the
working-state edge**: a watchdog that *terminates* instead of *pausing*, an indicator that mixes
launch state with turn state, and Esc semantics that ignore the turn phase. This is a **contract
tightening, not a rewrite**.

---

## 5. Prioritized recommendations

| # | Severity | Change | Status | Files |
|---|---|---|---|---|
| 1 | **High (the bug)** | Live activity re-enters `Streaming` from `Idle`/`Disconnected`, so resumed content after a stream drop shows working. | **✅ shipped** | `agent-pane-state/reducer.ts` |
| 3 | Medium | Phase-guard `stopAgent`: empty-Esc on a non-working phase is a quiet no-op (no SIGINT / "stop failed"). | **✅ shipped** | `useAgentCommands.ts` |
| 2 | Low | Working indicator derives from `turnPhase` only (drop `status.isLoading()`) — fixes the startup flicker, not the reported bug. | deferred | `agent-view.tsx:1056,1185` |
| W | Low (cleanup) | Delete or wire the dead `StreamStalled` / `schedule-stream-watchdog` watchdog (emitted, never consumed). | deferred | `agent-pane-state/{reducer,types}.ts` |
| 4 | Medium | Guaranteed drain point for held-queued messages on `Interrupting → Done` / stall-out. | deferred | `useAgentCommands.ts` |
| 5 | Low | Question-panel "Other" field: IME guard + Esc clears field before minimizing the panel. | deferred | `AgentQuestionPanel.tsx:150-161` |
| 6 | Low | Disable composer send while `workingFromPhase(turnPhase)` (double-submit window). | deferred | `AgentFooter.tsx`, `agent-view.tsx` |
| 7 | Low | Shared alert-surface transient-state helper + one "pending interactions" derivation. | deferred | new helper; `agent-view.tsx` |

**This pass ships #1 (the reported bug) + #3 (the Esc edge case you emphasized).**

---

## 6. Key references
- `agent-pane-state/types.ts:288-302` — `isWorking` / `workingFromPhase` (`Submitting|Streaming|Interrupting`)
- `agent-pane-state/types.ts:713` — `STREAMING_IDLE_TIMEOUT_MS = 60_000`
- `agent-pane-state/reducer.ts:698-724` — `StreamStalled` → terminal `Done.errored` (the one-way door)
- `agent-pane-state/reducer.ts:269-290` — `StreamWatchdogTick` (diagnostic `stream-stuck`, no transition)
- `agent-view.tsx:1056,1185` — working indicator = `status.isLoading() || workingFromPhase(...)`
- `hooks/useAgentControllerStatus.ts:100` — `isLoading = flowRunning() || !agentReady()`
- `AgentFooter.tsx:671-675,727-748` — autocomplete / composer Esc handlers
- `AgentQuestionPanel.tsx:150-161` — question-panel Esc = defer/minimize
- `AgentComposerStrip.tsx:210` + `_composer-strip.scss:61-91` — spinner render + animation
- `ActivityDock.tsx:46` + `activity/shell-adapter.ts:36` — derive-from-document accessory pattern
- `agent-view.tsx:496-505,752-804` — `pendingQuestions()` derivation + `handleAnswer()` optimistic/RPC
- backend: `agentmux-srv/src/backend/blockcontroller/health.rs` — recoverable `Healthy↔Stalled↔Dead`
- specs: `SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`, `SPEC_ASK_USER_QUESTION_2026_06_15.md`,
  `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`
