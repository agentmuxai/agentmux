# SPEC — agent-pane failure state: fold `useAgentFailure` into the turn-phase reducer

**Date:** 2026-07-06
**Author:** Agent2
**Status:** Draft
**Scope:** `frontend/app/store/agent-pane-state/` (the turn-phase reducer), `frontend/app/view/agent/hooks/useAgentFailure.ts`, `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/view/agent/hooks/useControllerStatusEvents.ts`. This spec **builds on** existing work — it is not a green-field design.
**Related (must-read first):**
`docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` (the `TurnPhase` discriminated union this spec extends),
`docs/specs/SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md` (the liveness-recovery watchdog this spec's fix intersects with),
`docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` (the failure-recovery banner UI this spec's hook currently drives — kept presentation-only by design at the time),
`docs/specs/SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md` (the backend `AgentFailure` classifier),
`docs/analysis/ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md` (the bug report that motivated this spec — read first for the concrete symptom).

---

## 1. Summary

The agent pane's turn lifecycle is already reducer-based and, per its own header comment, is meant to be **the single source of truth** for "is the agent working" (`docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`, `types.ts` line 99: *"Since PR G this is the only place where 'is the agent working'... is encoded"*). In practice it isn't quite that, because there is a second, fully independent state machine covering agent **failures**: `useAgentFailure.ts` — a Solid hook with its own local signals (`failure`, `expanded`, `retrying`, `autoRetryIn`) that never writes back into `AgentPaneState`.

This spec proposes folding that hook's *correctness-relevant* state (which failure is active, whether the turn actually ended because of it, the auto-retry countdown/budget) into `AgentPaneState` and the reducer's `update()` function, as new fields and commands following the exact same `caller-schedules-a-timeout, reducer-owns-the-transition` pattern already used for `SubmitTimeoutElapsed`/`InterruptTimeoutElapsed`. The hook keeps existing only for pure view wiring (subscribing to the wave event, calling the passed-in recovery effects) — it stops holding any state that the turn-phase machine needs to agree with.

This directly fixes two of the three bugs in `ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md` (stuck "Waiting" after a rate-limit interruption; the false-positive "Rate Limiting" label is a related but separately-fixable reducer gap, see §6) and removes a whole class of future bugs shaped exactly like them, because it removes the only place they could arise: two independently-mutated stores describing the same turn.

## 2. Where we are today

**Three independent subscribers to the same two backend signals.** `ControllerStatus` and `AgentFailure` are each per-block wave events; today they are each consumed by three unrelated call sites, none of which coordinate:

| Subscriber | File | Owns | Writes to `AgentPaneState`? |
|---|---|---|---|
| Process-exit grace timer | `useAgentStream.ts` (~471-523) | 1.5s timer; forces `Streaming`/`Submitting` → `Disconnected` → `Idle` if the subprocess reports `done` with no `session_end` | Yes — `StreamUnsubscribe`/`StreamSubscribe` |
| Failure-recovery banner | `useAgentFailure.ts` (all 206 lines) | `failure`, `expanded`, `retrying`, `autoRetryIn`, the auto-retry countdown/budget, a `awaitingVerdict` mini state machine that infers turn success/failure from the *absence* of a following `AgentFailure` | **No** |
| Diagnostic logging | `useControllerStatusEvents.ts` | Formats both events into log lines | No (harmless, read-only) |

The comment at `useAgentStream.ts:508-510` names the split explicitly and treats it as intentional:

> "Net phase: Idle (Disconnected → Idle via StreamSubscribe). **The AgentFailure banner drives the crash UX independently of turn phase.**"

That was a reasonable simplification when the only failure path was "the subprocess actually exits" — the grace timer's own 1.5s window already handles turning `turnPhase` back to `Idle` in that case, and the AgentFailure banner just has to show up alongside it with no coupling required. The gap is the case the same comment block documents two lines above (`useAgentStream.ts` ~480): **persistent-mode agents, where the process never exits between turns**, so `ControllerStatus: done` never fires and the grace timer never arms. If the backend classifies a rate-limit (or any other) failure for a persistent-mode agent that stays alive, `AgentFailure` fires, the banner correctly appears — and *nothing* ever tells `turnPhase` the turn is over. It sits in `Streaming` (with a stale `waitingReason: "rate_limited"`, if that's what triggered it) until the `StreamWatchdogTick` liveness-recovery backstop eventually force-clears it, up to `retryAfterMs + LIVENESS_RECOVERY_MS` (3 minutes) later per `types.ts:294-299`. That backstop was designed for a different failure mode (a hung stream with no terminal signal at all) — here it's papering over the fact that an authoritative terminal signal (`AgentFailure`) *did* arrive, just on the wrong bus.

**`useAgentFailure`'s own internal duplication.** The hook additionally re-derives "did the turn succeed or fail" from raw `ControllerStatus` transitions (the `awaitingVerdict` flag, `useAgentFailure.ts:62-68, 149-176`) — a bespoke turn-outcome classifier that exists only because the hook has no visibility into `state.turnPhase`/`TurnOutcome`, which already encodes exactly this (`Done{outcome: "completed"|"stopped"|"interrupted"|"errored"}`, `types.ts:67-75`). It is inferring, from the outside, a fact the reducer already knows on the inside.

## 3. Motivation

Three concrete, currently-open bugs (`ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md`):

1. **Stuck "Waiting"** — direct consequence of the split described above: an authoritative `AgentFailure` for a persistent-mode agent has no path into `turnPhase`.
2. **False-positive "Rate Limiting"** — a narrower, single-reducer-branch bug (`StreamFlushObserved`'s `Streaming` arm doesn't clear `waitingReason`/`retryAfterMs` the way `bumpEvent` does). This is *not* fixed by this spec directly — it's a bug within the existing single-reducer turn-phase machine, not a cross-store drift bug — but §6 shows why unifying makes it easier to reason about and test alongside the failure-clearing fix, since both touch the same `Streaming` arm's field-clearing discipline.
3. **"Send now" removal** — unrelated to this spec; tracked separately as a subtractive UI change in the analysis doc.

Beyond the three reported symptoms: every future feature that needs to know "is this pane in a failure state" from outside `useAgentFailure.ts` (there is already one internal consumer doing exactly this awkwardly — the hook's own `awaitingVerdict` inference) has no way to ask `AgentPaneState` and has to either duplicate the wave-event subscription or thread the hook's `row()` accessor around. A `state.failure` field makes it a normal reducer-state read like everything else in the pane.

## 4. Goals / non-goals

**Goals**
- `AgentPaneState` becomes the sole place that can answer "is there an active failure for this pane, and did it end the current turn."
- A backend `AgentFailure` for a pane whose `turnPhase` is `Submitting`/`Streaming`/`Interrupting` **always** force-transitions that phase to `Done{outcome: "errored"}` on receipt — no dependency on the CLI process actually exiting, no 3-minute backstop needed for this case.
- The auto-retry countdown (a bounded ladder — `AUTO_RETRY_BACKOFF_S` in `useAgentFailure.ts` is the source of truth; `5s → 10s` capped at 2 when this spec was written, `5s → 15s → 30s → 60s → 120s` jittered as of 2026-08-31) moves onto the established `schedule-*-timeout` / `*TimeoutElapsed` pattern already used for `SubmitTimeoutElapsed`/`InterruptTimeoutElapsed`, so it's driven by dispatched commands and testable the same way (fake timers + assert on `update()` output), instead of a hook-local `setInterval`.
- `useAgentFailure.ts` shrinks to: subscribe to the wave event, dispatch a command, read `state.failure` back out, wire the passed-in recovery effects (`onRetry`, `onLoginAgain`, etc.) — genuinely presentation-only, with no state of its own that anything else needs to agree with.
- Fix the `StreamFlushObserved` clear-on-activity gap (Issue 2) as part of the same reducer touch-up, since it's in the immediately adjacent code.

**Non-goals**
- No change to the `AgentFailure` taxonomy, the backend classifier, or the wave-event transport (`SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md` territory — untouched).
- No change to the recovery *actions* themselves (retry/login/trust-center/new-session effects) — still caller-supplied functions, still invoked the same way.
- No change to the visual design of the failure banner (`failure-accessory.ts`'s `failureToRow` — still consumes the same shape of data, just sourced from `state.failure` instead of a hook-local signal).
- Not attempting to also fold in `InitPhase` or the `waiting-for-input`/`waiting-ended` sound-notification state into one mega-reducer — those are legitimately separate concerns per the existing "no god-reducer" convention (`types.ts:24`); this spec only merges the two stores that are already about the *same* thing (was this turn's outcome a failure).

## 5. Proposed evolution

### 5.1 New state

```ts
// types.ts — AgentPaneState
failure: PaneFailure | null;
```

```ts
export interface PaneFailure {
    /** The classified failure, verbatim from the backend event. */
    data: AgentFailure;
    /** Wall-clock ms the failure landed. */
    at: number;
    /** Auto-retry countdown state, present only for transient classes
     *  (rate_limited / overloaded / network — see isTransient()). */
    autoRetry: {
        /** Seconds remaining until the next scheduled retry, or null
         *  if no auto-retry is armed (budget exhausted / non-transient). */
        remainingS: number | null;
        /** How many auto-retries have fired this episode (capped at the
         *  `AUTO_RETRY_BACKOFF_S` ladder length). */
        count: number;
    } | null;
    /** True while a user-initiated or auto-fired retry is in flight
     *  (mirrors the hook's current `retrying` signal). */
    retrying: boolean;
}
```

Everything here is either already backend-authoritative (`data`) or was already being computed by `useAgentFailure.ts` — this is a relocation, not new information.

### 5.2 New commands

```ts
/** Backend classified a failure for the current/just-ended turn. */
| { type: "FailureObserved"; failure: AgentFailure; at: number }
/** User dismissed the banner, or a fresh turn started (episode over). */
| { type: "FailureCleared" }
/** User clicked Retry (manual) — clears the row, keeps the auto-retry budget. */
| { type: "FailureRetryRequested"; at: number }
/** The auto-retry countdown's caller-scheduled timer ticked down to 0. */
| { type: "AutoRetryElapsed"; at: number }
```

### 5.3 New events (for the dispatch layer / telemetry, same shape as existing `schedule-*`/`*-timed-out` pairs)

```ts
| { type: "failure-observed"; code: AgentFailure["code"]; turnWasEnded: boolean }
| { type: "failure-cleared" }
/** Caller arms a countdown timer; mirrors schedule-submit-timeout /
 *  schedule-interrupt-timeout. deadlineMs = at + seconds*1000. */
| { type: "schedule-auto-retry"; deadlineMs: number; seconds: number }
| { type: "auto-retry-fired" }
```

### 5.4 Reducer transitions

**`FailureObserved`** — the core fix:
```ts
case "FailureObserved": {
    const turnWasEnded = workingFromPhase(state.turnPhase);
    const nextPhase: TurnPhase = turnWasEnded
        ? { kind: "Done", outcome: "errored", finishedAt: command.at }
        : state.turnPhase; // pane was already idle (e.g. a stray late event) — leave phase alone
    const autoRetry = isTransient(command.failure.code)
        ? { remainingS: null, count: 0 } // armAutoRetry decides the first countdown; see below
        : null;
    const next: AgentPaneState = {
        ...state,
        turnPhase: nextPhase,
        currentTool: null,
        currentToolArg: null,
        failure: { data: command.failure, at: command.at, autoRetry, retrying: false },
    };
    const events: AgentPaneEvent[] = [
        { type: "failure-observed", code: command.failure.code, turnWasEnded },
    ];
    if (autoRetry) {
        events.push({ type: "schedule-auto-retry", deadlineMs: command.at + 5_000, seconds: 5 });
    }
    return { state: next, events };
}
```
This is the direct fix for Issue 1: a `FailureObserved` command *unconditionally* ends a working turn, regardless of whether the underlying process ever exits. The dispatch layer fires this command from the exact same `AgentFailure` wave-event handler that `useAgentFailure.ts` already has — it just also calls `model.dispatchPane(...)` now, instead of only calling `setFailure(...)`.

**`AutoRetryElapsed`** and the auto-retry budget (replaces the hook's `setInterval` + `AUTO_RETRY_BACKOFF_S` array):
```ts
case "AutoRetryElapsed": {
    if (!state.failure) return { state, events: [] }; // already cleared/retried manually
    const count = state.failure.autoRetry?.count ?? 0;
    const events: AgentPaneEvent[] = [{ type: "auto-retry-fired" }];
    // Ladder length comes from AUTO_RETRY_BACKOFF_S, then manual-only —
    // same budget semantics as the hook, whatever the rungs currently are.
    if (count >= AUTO_RETRY_BACKOFF_S.length) {
        return { state: { ...state, failure: { ...state.failure, autoRetry: null } }, events };
    }
    const seconds = AUTO_RETRY_BACKOFF_S[count];
    events.push({ type: "schedule-auto-retry", deadlineMs: command.at + seconds * 1000, seconds });
    return {
        state: { ...state, failure: { ...state.failure, autoRetry: { remainingS: seconds, count: count + 1 } } },
        events,
    };
}
```
(The 1-second tick for the visible countdown label stays a `setInterval` in the dispatch layer purely for the `remainingS` display value between `schedule-auto-retry` and `AutoRetryElapsed` — same as today's UI-only tick — but the *decision* to retry, and the budget that caps it, is now reducer state instead of hook-local variables.)

**`FailureCleared`** / **`FailureRetryRequested`** — straightforward, mirror the hook's existing `clear()`/`endEpisode()`/`doRetry()` split: `FailureRetryRequested` sets `failure.retrying = true` and lets the caller invoke `onRetry()` (unchanged effect), `FailureCleared` sets `failure = null` (episode over, budget reset — enforced by simply not carrying `autoRetry.count` forward since the whole `failure` object is replaced).

**Existing transitions that gain one line:** `TurnStart` already resets various per-turn fields (`reducer.ts`) — it should also set `failure: null` when it observes a fresh turn beginning while `state.failure` is non-null (this replaces the hook's `awaitingVerdict`/`endEpisode()` inference at `useAgentFailure.ts:158-174` — the reducer already knows a `TurnStart` command is "a fresh task," it doesn't need to reconstruct that from `ControllerStatus: running` transitions).

### 5.5 `StreamFlushObserved` fix (Issue 2, same-touch)

Since this spec already has the reducer file open for the `Streaming`-arm clearing discipline, fold in the one-line fix identified in `ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md` §Issue 2:
```ts
// reducer.ts, StreamFlushObserved, Streaming arm
? { ...state.turnPhase, bufferSize: newBuf, lastEventMs: command.at, waitingReason: undefined, retryAfterMs: undefined }
```
matching what `bumpEvent` already does. Not required by the failure-unification goal, but it's the same "does this activity-observing branch clear stale transient-wait fields" bug class, in the same function, and shipping it separately would just mean touching this file twice.

### 5.6 `useAgentFailure.ts` after the change

```ts
export function useAgentFailure(opts: UseAgentFailureOptions): UseAgentFailureResult {
    // No local failure/expanded/retrying/autoRetryIn signals anymore —
    // `expanded` is the one piece of pure ephemeral UI state with no
    // correctness implications (whether the banner body is expanded
    // doesn't need to agree with anything else), so it can stay a plain
    // local signal or move to state.failure.expanded — either is fine;
    // recommend keeping it local to avoid a reducer round-trip on every
    // click of a UI-only toggle.
    const [expanded, setExpanded] = createSignal(false);

    onMount(() => {
        const unsubFailure = waveEventSubscribe({
            eventType: WpsEvent.AgentFailure,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                setExpanded(false);
                opts.model.dispatchPane({ type: "FailureObserved", failure: f, at: Date.now() });
            },
        });
        onCleanup(unsubFailure);
    });

    const row = (): FailureRow | null => {
        const f = paneSnapshot(opts.blockId)?.failure; // or a dedicated failureAtom, same pattern as turnPhaseAtom
        if (!f) return null;
        return failureToRow(
            f.data,
            { expanded: expanded(), autoRetryIn: f.autoRetry?.remainingS ?? null, retrying: f.retrying, canSeed: opts.canSeed?.() },
            {
                retry: () => opts.model.dispatchPane({ type: "FailureRetryRequested", at: Date.now() }),
                loginAgain: opts.onLoginAgain,
                useExistingLogin: opts.onUseExistingLogin,
                loginViaTerminal: opts.onLoginViaTerminal,
                openArmory: opts.onOpenArmory,
                newSession: opts.onNewSession,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: () => opts.model.dispatchPane({ type: "FailureCleared" }),
            },
        );
    };

    // The dispatch layer (agent-view.tsx, alongside its existing schedule-submit-timeout /
    // schedule-interrupt-timeout handling) arms a setTimeout on "schedule-auto-retry" events
    // and dispatches AutoRetryElapsed when it fires — same pattern, third instance.

    return { row };
}
```
The persisted-across-reload behavior (`getBlockMetaKeyAtom(opts.blockId, "agent:last_failure")`, `useAgentFailure.ts:123-125`) is unaffected — it seeds `state.failure` via the same `FailureObserved`-shaped dispatch on mount instead of seeding a local signal.

## 6. How this closes the reported issues

- **Issue 1 (stuck "Waiting")** — closed directly. `FailureObserved` unconditionally ends a working turn the instant the backend classifies a failure, regardless of whether the CLI process ever exits. The 3-minute liveness-recovery backstop remains as a safety net for the *different* failure mode it was built for (a hung stream with no terminal signal of any kind — no `session_end`, no `AgentFailure`, nothing), but stops being the only recovery path for the persistent-mode-rate-limit case.
- **Issue 2 (false-positive "Rate Limiting")** — closed by the same-touch `StreamFlushObserved` fix (§5.5). Not a consequence of the failure-unification itself, but landing in the same PR since it's the same file and the same "clear transient fields on activity" discipline.
- **Issue 3 ("Send now")** — out of scope for this spec (pure UI subtraction, no state-machine dependency); tracked as-is in the analysis doc.

## 7. Rollout

Suggested as three independent, separately-reviewable PRs (each shippable/revertable alone, per the repo's general preference for small PRs over one large one):

1. **Reducer additions** — `PaneFailure` type, `FailureObserved`/`FailureCleared`/`FailureRetryRequested`/`AutoRetryElapsed` commands + their `update()` cases, `TurnStart`'s one-line `failure: null` reset, the `StreamFlushObserved` fix. Pure reducer change — testable in isolation with the existing `reducer.test.ts` fake-clock harness, no UI wiring yet (dead code until step 2 dispatches into it).
2. **Dispatch-layer + hook rewrite** — `useAgentFailure.ts` shrinks per §5.6; the `AgentFailure` wave-event handler moves its work into a `dispatchPane({type:"FailureObserved",...})` call (likely relocated to live alongside `useAgentStream.ts`'s other wave-event subscriptions, since that's where every other backend-event-to-reducer-command translation already lives); the auto-retry `setTimeout` arming moves to wherever `schedule-submit-timeout`/`schedule-interrupt-timeout` are currently armed (`agent-view.tsx`, matching the existing pattern).
3. **Cleanup** — delete the now-fully-redundant `awaitingVerdict` logic and the hook's old local signals; `useControllerStatusEvents.ts`'s pure-logging `AgentFailure`/`ControllerStatus` subscriptions are unaffected (they were never state, nothing to migrate) but worth a pass to confirm no duplicate work remains.

Each step leaves the pane fully functional — step 1 alone changes nothing observable; step 2 is the actual behavior fix; step 3 is pure deletion.

## 8. Tests

- `reducer.test.ts`: `FailureObserved` while `Streaming`/`Submitting`/`Interrupting` → `Done.errored`; `FailureObserved` while `Idle`/`Done` → phase untouched (no spurious re-entry); auto-retry budget exhausts at 2 and does not re-arm; `TurnStart` clears a pre-existing `state.failure`; `StreamFlushObserved` clears `waitingReason`/`retryAfterMs` on the `Streaming` arm (Issue 2's regression test).
- `useAgentFailure` (or its replacement dispatch-layer test): the wave-event handler dispatches `FailureObserved` with the right `at`; `dismiss`/`retry` dispatch `FailureCleared`/`FailureRetryRequested` respectively.
- One integration-shaped test simulating the exact reported scenario: a persistent-mode agent (`ControllerStatus` never reports `done`) that receives a rate-limit `AgentFailure` mid-`Streaming` — assert `turnPhase.kind` becomes `Done` within the same reducer tick (not 3 minutes later).

## 9. Open questions

- Should `expanded` (banner-body-open) live in `state.failure.expanded` for full consistency, or stay a local hook signal as recommended in §5.6? Leaning local — it's the one piece of this hook's state that is genuinely presentation-only with zero correctness coupling to anything else, and round-tripping every expand/collapse click through `dispatchPane` adds no value.
- `useControllerStatusEvents.ts` remains a third independent subscriber to both events post-migration, purely for log lines. Worth asking separately (not blocking this spec) whether that logging could instead be driven off the new `failure-observed`/`turn-ended` reducer *events* (which already carry everything needed) rather than re-subscribing to the raw wave events a third time — would shrink the "how many places listen to ControllerStatus" count from 2 (post-migration) to 1, but is a bigger blast radius since it also logs the plain `running`/`done`/exit-code lines that have no reducer-event equivalent today.

## 10. Why this is worth doing

The three bugs in the motivating analysis doc are not really three unrelated bugs — Issue 1 is what happens when two state machines describing the same turn disagree, and it will recur in a different shape (a different failure code, a different persistent-mode edge, a different future feature that needs to read failure state) every time someone touches either side without remembering the other exists, because nothing in the type system enforces that they agree. Folding the failure state into the same reducer that already owns `turnPhase` removes the possibility of that class of bug entirely, the same way the original `TurnPhase` discriminated union (`SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`) removed the illegal-state class that the old `turnActive`/`stopping`/`streaming.active` boolean trio allowed. It's the same fix shape, one layer further out.
