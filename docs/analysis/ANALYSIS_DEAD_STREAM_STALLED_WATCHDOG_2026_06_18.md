# Analysis: the dead `StreamStalled` streaming-idle watchdog

**Date:** 2026-06-18
**Author:** smike
**Status:** Analysis (no code) — verified against `main` @ `f782755e` (post-v0.46.2)
**Area:** `frontend/app/store/agent-pane-state/{reducer,types}.ts`, `…/agent-pane-state-store.ts`,
`frontend/app/notification/sound/sound-service.ts`, `frontend/app/view/agent/useAgentStream.ts`
**Context:** Flagged in `ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER_2026_06_17.md` while diagnosing
the "Smark stuck / in-progress not showing" incident. That analysis first mis-blamed this watchdog,
then corrected to the real cause (fixed in #1523). This is the focused deep-dive the cleanup needs.

---

## TL;DR

The agent-pane "bounded streaming-idle watchdog" (intended: a `Streaming` turn that goes silent for
60 s is force-ended as `Done.errored("stream-stalled")`) is **dead code — it never fires in
production.** The reducer *emits* the schedule request and *has* the terminal transition, but **the
middle link — a dispatcher that turns the schedule into a timer and fires `StreamStalled` — was never
built.** Confirmed on latest `main`. Recommendation: **delete it** (or, if a bounded give-up is
genuinely wanted, *wire it* — but as a *recoverable* long-timeout, not the current 60 s kill).

---

## 1. What it was designed to do (PR E)

The intended pipeline (a 4-link chain):

```
reducer emits  ─►  a dispatcher schedules  ─►  timer fires  ─►  reducer transitions
schedule-          setTimeout(deadline)        StreamStalled    Streaming → Done.errored
stream-watchdog                                command          ("stream-stalled")
                                                                 + emits stream-stalled event
                                                                 ─► sound-service plays a sound
```

- On every stream-activity command inside `Streaming`, `streamWatchdogEvent()` emits a
  `schedule-stream-watchdog` event carrying `deadlineMs = lastEventMs + STREAMING_IDLE_TIMEOUT_MS`
  (`reducer.ts:55-63`, `STREAMING_IDLE_TIMEOUT_MS = 60_000` at `types.ts:713`). Call sites:
  `StreamSubscribe`, `StreamFlushObserved`, `ToolStart`, `ToolEnd`, `TokensIn`, `TokensOut`.
- A dispatcher was supposed to consume that event, `setTimeout` to the deadline (cancelling any prior
  arm), and dispatch the `StreamStalled` command when it fires.
- `StreamStalled` → the reducer arm at `reducer.ts:678` force-transitions `Streaming →
  Done.errored("stream-stalled")` and emits a `stream-stalled` event.
- The `stream-stalled` event → `sound-service.ts:156` plays `agent.stream.stalled`.

Type/contract docs for the chain live at `types.ts:425,445,613,623,628,699`.

---

## 2. The wiring gap (why it's dead) — verified

**Link 2 (the dispatcher) does not exist.** Across the whole `frontend` tree (excluding
`reducer.ts`/`types.ts`/`*.test.ts`):

- `schedule-stream-watchdog` — **zero consumers.** It is emitted by the reducer and dropped: the
  `EventSink` that fans reducer events out (`agent-pane-state-store.ts:208`,
  `for (const ev of result.events) …`) reaches the sound-service, whose `switch` handles
  `stream-stalled`, `submit-timed-out`, etc. but has **no `schedule-stream-watchdog` case**.
- `StreamStalled` — **never dispatched.** No `setTimeout`, no `deadlineMs` consumer, no
  `dispatchPane({ type: "StreamStalled" })` anywhere in production (only the reducer arm + unit tests
  reference it).

So `StreamStalled` is never sent → the reducer arm at `reducer.ts:678` **never runs** → the turn is
never force-ended → the `stream-stalled` event is **never emitted** → the `agent.stream.stalled`
sound (`sound-service.ts:156`) **never plays**. The whole watchdog and its only consumer are
unreachable.

### What IS alive (don't confuse it)
The **5-second `StreamWatchdogTick`** is wired (`useAgentStream.ts:590` dispatches it every 5 s) and
its reducer arm emits a *diagnostic* `stream-stuck` event when idle ≥ `STUCK_THRESHOLD_MS` (45 s).
But `stream-stuck` **also has no consumer** (`useAgentStream.ts:36,585` are just comments) — so the
tick runs and emits diagnostics into the void. It performs **no transition** either way. The tick is
*alive but toothless*; the `StreamStalled` watchdog is *dead*.

---

## 3. Still applicable? Yes — and slightly worse than before

Re-verified on `main` @ `f782755e`:
- The dead chain is intact (no PR wired the dispatcher).
- A consumer for the *event* was since added — `sound-service.ts:156` plays a sound on
  `stream-stalled` — but it's **unreachable** because the event is never emitted. So the dead code
  now has a dead *consumer* hanging off it too (more surface to mislead a reader).
- `#1523` (just merged) changed adjacent reducer arms (resumed live content re-enters `Streaming`
  from `Idle`/`Disconnected`) but did not touch the watchdog.

---

## 4. Impact of it being dead

- **No false termination** (good): because it never fires, it is *not* what suppressed the
  working indicator during the Smark stall — that was the disconnect/reconnect promotion gap, fixed
  in #1523. (This corrects the first hypothesis in the 06-17 analysis.)
- **No bounded give-up for a truly-wedged subscribed stream** (the gap it was meant to fill): if a
  stream stays *subscribed* but silent forever (process hung, no disconnect), nothing force-ends the
  turn. In practice this is covered: a killed/dead agent closes the stream → `StreamUnsubscribe →
  Disconnected` (terminal, clears working); and after #1523 a resumed stream correctly re-enters
  working. The only uncovered case is "subscribed + permanently silent + never disconnects" — rare,
  and the user can always Stop. So the missing give-up is low-impact.
- **Maintenance cost / confusion** (the real cost): ~5 reducer call-sites, a helper, a command, an
  event, a constant, a sound-service case, and a block of unit tests all describe a feature that does
  nothing. It actively misled this very investigation.

---

## 5. Recommendation

**Option A — delete it (recommended).** Lowest risk; removes a feature that has never run and the
confusion it causes. Remove:
- `StreamStalled` command type (`types.ts:445`) + its reducer arm (`reducer.ts:678-~730`).
- `schedule-stream-watchdog` event type (`types.ts:623`) + the `streamWatchdogEvent()` helper
  (`reducer.ts:55-63`) + its 6 call sites (`StreamSubscribe`/`StreamFlushObserved`/`ToolStart`/
  `ToolEnd`/`TokensIn`/`TokensOut`).
- `STREAMING_IDLE_TIMEOUT_MS` (`types.ts:713`) — referenced only by the dead path.
- The `stream-stalled` event type + the unreachable `sound-service.ts:156` case.
- The associated reducer unit tests (PR E "bounded streaming watchdog" block).
- *Optional, related:* the toothless `stream-stuck` path too, unless a consumer is planned.
Net: a pure deletion, no behavior change (the code never executed).

**Option B — actually wire it (only if a bounded give-up + "stalled" sound is wanted).** Build the
missing dispatcher (consume `schedule-stream-watchdog` → `setTimeout(deadline)` with cancel-on-rearm
→ dispatch `StreamStalled`). **But do not ship the current 60 s kill** — real upstream stalls (API
overload/backoff) run for *minutes* (see the Smark incident), so a 60 s `Done.errored` would falsely
kill live turns. If wiring: make it **recoverable** (a `stalled` flag on `Streaming`, cleared by the
resumed-activity re-entry #1523 already added) and/or raise the terminal give-up to several minutes.
More code, reintroduces the exact risk the 06-17 analysis warned about — only worth it for a real UX
need.

**Verdict:** delete (A) unless product wants the bounded-give-up UX, in which case wire it as a
recoverable long-timeout (B).

---

## 6. Reference index
- `frontend/app/store/agent-pane-state/types.ts:445` — `StreamStalled` command (dead)
- `…/types.ts:623` — `schedule-stream-watchdog` event (emitted, **unconsumed**)
- `…/types.ts:713` — `STREAMING_IDLE_TIMEOUT_MS = 60_000` (used only by the dead path)
- `…/reducer.ts:55-63` — `streamWatchdogEvent()` (emits the unconsumed schedule)
- `…/reducer.ts:678` — `StreamStalled` arm → `Done.errored("stream-stalled")` (never runs)
- `…/reducer.ts:151,252,464,476,494,512` — the 6 `streamWatchdogEvent()` call sites
- `…/agent-pane-state-store.ts:208` — `EventSink` fan-out (no `schedule-stream-watchdog` handler)
- `frontend/app/notification/sound/sound-service.ts:156` — `stream-stalled` consumer (unreachable)
- `frontend/app/view/agent/useAgentStream.ts:585,590` — the 5 s `StreamWatchdogTick` (alive) emitting
  the diagnostic `stream-stuck` (also unconsumed)
- Prior context: `docs/analysis/ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER_2026_06_17.md`,
  `docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` (PR E)
