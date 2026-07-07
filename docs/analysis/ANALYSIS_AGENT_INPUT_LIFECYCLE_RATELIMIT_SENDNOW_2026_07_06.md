# Analysis: Agent-pane input lifecycle — stuck "Waiting", false-positive "Rate Limiting", and "Send now" removal (2026-07-06)

**Status:** Root causes identified for all three issues; fixes described but not implemented (analysis-only per request).
**Reported by:** user — "If we are interrupted with a rate limit, I see the error, but the prompt stays in 'Waiting'. There are also times where it says 'Rate Limiting' when it clearly is not. Finally, we want to get rid of the 'Send Now' — instead, just queue send the message."
**Related prior analysis:** `docs/analysis/ANALYSIS_SEND_NOW_FLASH_2026_05_28.md` (the `isInterruptibleTurn` selector that gates "Send now" today was itself a fix for an earlier flashing bug in this same button — see Issue 3 below), `docs/analysis/ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER_2026_06_17.md`, `docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`.

---

## Files involved

**Composer / input UI**
- `frontend/app/view/agent/components/AgentFooter.tsx` — the composer textarea and `AgentWorkingRow`, the busy/status row above it (spinner + "Rate limited…"/"Stopping…" text).
- `frontend/app/view/agent/components/PendingMessagesPanel.tsx` — the amber "queued" strip between the conversation and the composer, including the "Send now" button.
- `frontend/app/view/agent/styles/_pending-footer.scss` — "Send now" button styling.

**State / reducer**
- `frontend/app/store/agent-pane-state/types.ts` — `TurnPhase` union (`Idle | Submitting | Streaming | Interrupting | Done | Disconnected`); `waitingReason`/`retryAfterMs` live on the `Streaming` variant; `workingFromPhase`/`isInterruptibleTurn` selectors; `LIVENESS_RECOVERY_MS`.
- `frontend/app/store/agent-pane-state/reducer.ts` — pure reducer; `ProviderWaiting`, `StreamFlushObserved`, `bumpEvent`, `StreamWatchdogTick`, `TurnEnd` cases.

**Streaming / failure surfaces**
- `frontend/app/view/agent/useAgentStream.ts` — dispatches `ProviderWaiting`/`StreamFlushObserved`/`TurnEnd`; owns the 1.5s process-exit grace timer.
- `frontend/app/view/agent/providers/claude-translator.ts` — turns raw CLI `rate_limit_event` frames into `provider_waiting` stream events.
- `frontend/app/view/agent/hooks/useAgentFailure.ts` + `frontend/app/view/agent/failure/failure-accessory.ts` — the **separate** failure-banner subsystem (retry/login/rate-limited/etc.), driven by its own `AgentFailure` wave event.
- `frontend/app/view/agent/hooks/useAgentCommands.ts` — `sendMessage`, `heldQueue`, `flushHeldMessages`, `stopAgent`.
- `frontend/app/view/agent/agent-view.tsx` — wires all of the above into JSX; owns the auto-flush `createEffect`.

---

## Issue 1 — Prompt stuck in "Waiting" after a rate-limit interruption

**Symptom:** the failure banner correctly shows a rate-limit error, but the composer's busy indicator (`AgentWorkingRow`, spinner + "Rate limited — retrying…" text) never clears — the input area stays in its "working" look indefinitely.

**Why:** there are **two independent state machines** in this code path, and only one of them is wired to backend failure classification:

1. **Turn-phase state machine** (`reducer.ts` / `types.ts`) — drives `AgentWorkingRow`. `AgentWorkingRow`'s `loading` prop is:
   ```tsx
   // agent-view.tsx:865
   loading={status.isLoading() || workingFromPhase(agentAtoms().turnPhaseAtom[0]())}
   ```
   `workingFromPhase` (`types.ts`) is `true` for `{Submitting, Streaming, Interrupting}`. The row's rate-limit text comes from `AgentFooter.tsx` (~line 155-158):
   ```tsx
   props.waitingReason === "rate_limited"
       ? (props.retryAfterMs != null
           ? `Rate limited — retrying in ${Math.ceil(props.retryAfterMs / 1000)}s`
           : "Rate limited — retrying…")
   ```
   `waitingReason` is only ever set by the `ProviderWaiting` reducer case (`reducer.ts:812-827`), and only while `turnPhase.kind === "Streaming"`. The *only* things that move `turnPhase` out of `Streaming` are: a `session_end` frame → `TurnEnd` (`useAgentStream.ts`, `finalizeTurn`); the 1.5s process-exit grace timer gated on `ControllerStatus: "done"` (`useAgentStream.ts`); the bounded `Submitting`/`Interrupting` timeouts; or `StreamWatchdogTick`'s liveness recovery — which for a rate-limited phase is **deliberately stretched to `retryAfterMs + LIVENESS_RECOVERY_MS` (180s)** (`reducer.ts:294-299`, `types.ts`), so a legitimately-still-retrying CLI isn't force-recovered too early.

2. **Failure-banner state machine** (`useAgentFailure.ts`) — drives the visible error banner. It subscribes independently to the `AgentFailure` wave event and only touches its own local `failure`/`retrying`/`autoRetryIn` signals. **It never calls `model.dispatchPane(...)`** — confirmed by grep: no `dispatchPane` call exists anywhere in `useAgentFailure.ts`. Nothing in this file (or the sibling `useControllerStatusEvents.ts`, which also subscribes to the same `AgentFailure` event purely for logging) ever dispatches `TurnEnd` or anything else that would clear `turnPhase`.

**Root cause, concretely:** when the backend classifies an interruption as an `AgentFailure{code:"rate_limited"}` (which is what makes the error banner appear) but the underlying CLI process either (a) never emits a terminating `result`/`session_end` frame, or (b) is a persistent-mode controller whose OS process doesn't actually exit (so `ControllerStatus` never flips to `"done"` and the 1.5s grace timer never arms) — **nothing ever tells the turn-phase reducer "this turn is over."** `turnPhase` sits in `Streaming` with a now-stale `waitingReason: "rate_limited"` / `retryAfterMs`, and `AgentWorkingRow` keeps rendering "Rate limited — retrying…" until the ~3-minute watchdog safety net eventually force-recovers it. Meanwhile the failure banner is fully visible and interactive the whole time — exactly the reported symptom of "error shown, but the prompt area stuck Waiting."

**Minimal fix (described):** bridge the two state machines. When the `AgentFailure` wave event fires in `useAgentFailure.ts`'s handler (or a shared listener in `agent-view.tsx`), also force-end the turn if `turnPhase.kind` is still `Submitting`/`Streaming`/`Interrupting` at that moment — e.g. dispatch `TurnEnd` (or a new dedicated `AgentFailureObserved` reducer case that transitions straight to `Done{outcome:"errored"}`). That makes an authoritative backend failure classification immediately authoritative for the turn-phase state too, instead of depending on the CLI's own stdout framing or the long liveness-recovery window as the only backstop.

---

## Issue 2 — "Rate Limiting" label shown when the agent is not actually rate-limited

**Symptom:** the same `AgentWorkingRow` "Rate limited — retrying…" text (and, when present, its countdown) appears even though the agent has resumed normal streaming and is not currently rate-limited.

**Root cause: an asymmetric "clear on activity" implementation.** Two reducer paths represent "real stream activity happened" — only one of them clears the rate-limit flag:

- **`bumpEvent`** (`reducer.ts:863-877`, used by tool-start/tool-end/token-count events) explicitly clears it:
  ```ts
  if (next.turnPhase.kind === "Streaming") {
      next.turnPhase = {
          ...next.turnPhase,
          lastEventMs: nowMs,
          toolsActive: Math.max(0, next.turnPhase.toolsActive + toolsDelta),
          waitingReason: undefined,   // cleared
          retryAfterMs: undefined,    // cleared
      };
  }
  ```
- **`StreamFlushObserved`** (`reducer.ts:197-256`) — dispatched from `useAgentStream.ts` on every RAF batch flush of new streamed text/thinking content, i.e. the normal "the agent is producing plain output" signal — does **not**:
  ```ts
  // reducer.ts:222-228
  const nextPhase: TurnPhase =
      state.turnPhase.kind === "Streaming"
          ? {
                ...state.turnPhase,      // <- spreads waitingReason/retryAfterMs forward, unchanged
                bufferSize: newBuf,
                lastEventMs: command.at,
            }
          : /* ...promotion from Submitting/Idle/Disconnected/Done.completed... */;
  ```
  This branch spreads the entire previous `Streaming` phase object and only overrides `bufferSize`/`lastEventMs` — `waitingReason`/`retryAfterMs`, if set, ride along unchanged.

**Consequence:** once a `provider_waiting` (rate-limit) event has set `waitingReason: "rate_limited"`, if the CLI's very next real activity is plain streamed text/thinking (no intervening tool call, and no token-usage event arriving first — plausible for a short reply) the reducer updates `bufferSize`/`lastEventMs` but leaves the stale `waitingReason`/`retryAfterMs` in place. `AgentWorkingRow` keeps showing "Rate limited — retrying…" (with a frozen, no-longer-meaningful countdown) even though the agent is actively and successfully streaming — the literal false positive the user described. There is existing reducer test coverage asserting `StreamFlushObserved` mirrors `bufferSize`/`lastEventMs` correctly, but no test asserts it clears `waitingReason` — this looks like an overlooked gap in the `bumpEvent` vs. `StreamFlushObserved` symmetry, not an intentional design choice.

**Minimal fix (described):** make the `Streaming` branch of `StreamFlushObserved` (`reducer.ts:222-228`) clear `waitingReason`/`retryAfterMs` the same way `bumpEvent` does:
```ts
? { ...state.turnPhase, bufferSize: newBuf, lastEventMs: command.at, waitingReason: undefined, retryAfterMs: undefined }
```
so *any* observable stream activity — not only tool/token events — clears the rate-limit indicator.

**Note on a second, related-but-distinct "retrying" surface:** `useAgentFailure.ts`'s own `autoRetryIn` countdown (`AUTO_RETRY_BACKOFF_S = [5,10]`) drives a *different* "Retry now (Ns)" label inside the failure banner itself (`failure-accessory.ts`), shown for `rate_limited`, `overloaded`, and `network` failure classes alike (`isTransient`). This is a second place where "rate limited" text can appear somewhat generically across related-but-different error types — it isn't the mechanism behind the bug reported here (that's the composer's own `waitingReason`, above), but it's worth knowing about in case some observed "clearly not rate limited" reports are actually this banner showing its shared transient-retry copy for an `overloaded` or `network` failure rather than a true rate limit.

---

## Issue 3 — Remove "Send now"; always just queue

**Where it lives today:**
- Button: `PendingMessagesPanel.tsx` (~line 55-65), inside the queue header:
  ```tsx
  <Show when={props.showSendNow?.()}>
      <button class="agent-send-immediately-btn" onClick={() => props.onSendImmediately?.()}>
          <span class="agent-send-immediately-icon">⏭</span>
          <span>Send now</span>
      </button>
  </Show>
  ```
- Visibility wiring: `agent-view.tsx` (~line 927-944):
  ```tsx
  <PendingMessagesPanel
      pendingMessages={pendingMessagesAtom[0]}
      showSendNow={() =>
          isInterruptibleTurn(agentAtoms().turnPhaseAtom[0]()) &&
          pendingMessagesAtom[0]().some((m) => m.enqueuedWhileBusy)
      }
      onSendImmediately={() => { commands.stopAgent(); }}
  />
  ```
  `isInterruptibleTurn` (`types.ts`) is `true` only for `{Streaming, Interrupting}` — it deliberately excludes `Submitting` because, per `ANALYSIS_SEND_NOW_FLASH_2026_05_28.md`, using the broader `workingFromPhase` here previously caused the button to flash on every single send (the ~50-200ms window between `TurnStart` and `agent-message-accepted`). It has no other call site.
- Styling: `_pending-footer.scss` (~lines 43-83), the `.agent-send-immediately-btn`/`.agent-send-immediately-icon` rules.

**What clicking it actually does — and why it's a bit of a misnomer:** `onSendImmediately` calls `commands.stopAgent()`, which is `useAgentCommands.ts`'s `stopAgent`:
```ts
const stopAgent = (): void => {
    const phase = paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" as const };
    if (!workingFromPhase(phase)) return;
    opts.model.dispatchPane({ type: "RequestStop", at: Date.now() }, "user");
    RpcApi.ControllerInputCommand(TabRpcClient, { blockid: opts.blockId, signame: "SIGINT" }) /* ... */
};
```
This is the **same function** wired to Esc/Ctrl-C "stop the agent" elsewhere in the composer. "Send now" does not send the queued message directly at all — it sends `SIGINT` to the running CLI process, aborting/truncating whatever the agent is currently doing, which drives `turnPhase` toward `Interrupting` → `Done`, which in turn is one of the conditions that triggers the queue auto-flush (see below) sooner than it otherwise would. In other words: today's "Send now" is really "kill the current turn early so the queue flushes early" — an indirect, destructive route to an earlier send.

**The "queue then auto-send" path (already exists, becomes the only path):**
1. `sendMessage` (`useAgentCommands.ts`) — while a turn is in flight, the message is dispatched as a `PendingMessageQueued{enqueuedWhileBusy:true}` (this is what makes it appear in `PendingMessagesPanel`) and pushed onto an in-memory `heldQueue` array instead of being sent to the backend immediately.
2. A `createEffect` in `agent-view.tsx` (~line 624-640) watches `currentToolAtom` and `turnPhaseAtom.kind`, calling `commands.flushHeldMessages()` as soon as **a new tool call starts** or the turn **becomes idle/done** — i.e. it auto-delivers the queued message(s) at the agent's next natural breakpoint with zero user action.
3. `flushHeldMessages` drains `heldQueue` FIFO via `deliverToBackend`, which (for held messages) skips the normal 30s pending-expiry timer — the message just waits until flushed.
4. Backend emits `agent-message-accepted`; the entry is promoted from the pending zone into a normal `user_message` node.

The separate `recallLatestHeld` / "ArrowUp to un-queue" gesture (`useAgentCommands.ts`, wired in `AgentFooter.tsx`) lets the user pull a queued message back out for editing before it auto-sends — this is independent of "Send now" and is unaffected by removing it.

**What to remove for the requested behavior** (typing + submitting while busy should *always* just queue for auto-send, with no separate force-send affordance):
- `PendingMessagesPanel.tsx` — the `showSendNow`/`onSendImmediately` prop declarations and the `<Show>`/button block.
- `agent-view.tsx` — the `showSendNow`/`onSendImmediately` props passed into `<PendingMessagesPanel>` (keep the `pendingMessages` prop).
- `isInterruptibleTurn` (`types.ts`) and its dedicated tests — no other consumer exists, safe to delete outright.
- `_pending-footer.scss` — the `.agent-send-immediately-btn`/`.agent-send-immediately-icon` rules (and the stale "Send now" comment referencing the button's positioning).
- The comment in `AgentFooter.tsx` referencing "'Send now' now renders inside PendingMessagesPanel…" becomes stale and should go too.

**What NOT to remove:**
- `stopAgent` / `RequestStop` / SIGINT-on-Esc itself — that's the legitimate, independent "stop the agent" feature; it simply stops being reachable from the queue panel.
- `heldQueue`, `flushHeldMessages`, the `agent-view.tsx` auto-flush effect, and `recallLatestHeld` — these are exactly the "queue then auto-send" mechanism that becomes the sole behavior once "Send now" is gone.

---

## Other related lifecycle states noticed in the same area (context, not in scope)

- **`TurnPhase` union**: `Idle → Submitting → Streaming → {Interrupting | Done | Disconnected}`. `AgentWorkingRow` shows "Stopping…" for `Interrupting` — unaffected by the above.
- **`Disconnected`** phase drives a distinct "stream dropped mid-turn" banner (`AgentDisconnectedBanner.tsx`) with its own recovery path — a different stuck-state class from Issue 1, not implicated here.
- **Bounded force-transitions**: `InterruptTimeoutElapsed` (5s) and `SubmitTimeoutElapsed` (30s) exist as safety nets, but `SubmitTimeoutElapsed` only fires out of `Submitting`, not `Streaming` — it cannot help recover a stuck rate-limited `Streaming` phase either; only the ~3-minute `StreamWatchdogTick` liveness recovery covers that today (Issue 1's actual current backstop).
- **`waiting-for-input` / `waiting-ended`** reducer events are an unrelated ambient-sound feature (the agent asked a question and is idle) tied into a separate sound-notification subsystem — shares the word "waiting" by coincidence, not implicated in Issues 1-2.
- **`InitPhase`** (`InitPending`/`InitReady`/`InitFailed`) is a third, separate "loading" gate blocking `TurnStart` while history loads — unrelated to rate limiting, but shares the general "state machine gating the composer" shape and is worth keeping in mind if a future refactor tries to unify these into one status model.

---

## Summary of proposed fixes

| Issue | Fix location | Change |
|---|---|---|
| 1. Stuck "Waiting" after rate-limit failure | `useAgentFailure.ts` (or a shared listener) | On `AgentFailure` event, force-end the turn (dispatch `TurnEnd` or a new `AgentFailureObserved` case) if `turnPhase.kind` is still `Submitting`/`Streaming`/`Interrupting`. |
| 2. False-positive "Rate Limiting" label | `reducer.ts`, `StreamFlushObserved` case (~line 222-228) | Clear `waitingReason`/`retryAfterMs` in the `Streaming` branch, matching what `bumpEvent` already does. |
| 3. Remove "Send now" | `PendingMessagesPanel.tsx`, `agent-view.tsx`, `types.ts` (`isInterruptibleTurn`), `_pending-footer.scss`, `AgentFooter.tsx` (stale comment) | Delete the button, its wiring, and the now-unused selector; leave `heldQueue`/`flushHeldMessages`/the auto-flush effect/`recallLatestHeld` untouched as the sole queue-then-auto-send behavior. |

All three are small, localized changes with no architectural rework required — Issues 1 and 2 are one-branch reducer/handler fixes, Issue 3 is a subtractive UI change with clear boundaries already drawn by the existing (separately-selectored) queue-flush mechanism.
