# Retro — Busy animation stuck indefinitely on 429 rate limit (2026-06-24)

## TL;DR

When Claude CLI hits a 429 and enters its own retry-backoff loop, the agent pane
busy animation (marching-ants progress bar) keeps running indefinitely with **no
user-visible feedback**. The CLI emits a `rate_limit_event` NDJSON line during
each backoff, but `claude-translator.ts` silently drops it — no event reaches the
reducer, `turnPhase` stays `Streaming`, and the animation never changes appearance.
The stuck-stream watchdog fires after 45 s of silence as a diagnostic but surfaces
nothing to the user. The fix is to translate `rate_limit_event` into a visible
transcript node or status annotation so the user knows why the pane is waiting.

---

## What was observed

User observed: submitted a message to an agent, the marching-ants progress bar
started animating, then kept running indefinitely after a 429 was hit. No error
banner, no "retrying…" indicator, no change in animation appearance.

---

## The event path

```
Claude CLI stdout
    │  {"type":"rate_limit_event", "retry_after_ms": …}
    │
    ▼
agentmux-srv blockcontroller/health.rs:353
    classify_output_line → (meaningful=false, Transient "Rate limited")
    [records health event; does NOT forward to frontend]
    │
    ▼
claude-translator.ts (frontend)
    — no case for "rate_limit_event" —
    → silently drops the line
    │
    ▼
reducer: no command dispatched
    lastEventMs NOT updated
    turnPhase remains Streaming
    │
    ▼
agent-view.tsx binding:
    workingFromPhase(Streaming) === true → animation stays ON
```

After 45 s of silence the `StreamWatchdogTick` interval fires and the reducer
emits a `stream-stuck` audit event — but this has no visible effect on the UI.
The animation keeps running until the CLI either succeeds (emits `session_end`)
or gives up and exits (triggering `agentfailure` + `TurnEnd`).

---

## Root causes

**R1 — `claude-translator.ts` drops `rate_limit_event`**
The translator has no case for this event type, so it falls through unhandled.
Every other provider translator has the same gap. The health subsystem correctly
classifies it at `blockcontroller/health.rs:353`, but that classification lives
only in the sidecar health ring — it never crosses the wire to the frontend pane
state.

**R2 — `stream-stuck` watchdog is diagnostic-only**
`StreamWatchdogTick` (every 5 s) triggers a `stream-stuck` reducer event after
45 s (`STUCK_THRESHOLD_MS`). The event is logged but dispatches no visible change:
no transcript node, no status annotation, no animation change. From the user's
perspective the pane looks identical at t=0 and t=120 s.

**R3 — No "waiting / retrying" visual state exists**
The progress bar has two modes: active (marching ants) and stopping (dimmed). There
is no "paused / waiting on provider" visual — so even if R1 and R2 were fixed,
there's no surface to show the retry state on.

---

## Why R1 happened

The `rate_limit_event` line type is a Claude CLI implementation detail — it wasn't
in scope when the translator was written (it was designed around `text`, `tool_use`,
`result`, `system`). The health subsystem picked it up later for server-side
monitoring without a corresponding frontend translation pass. The two subsystems
diverged silently: health knows about it, translator doesn't.

---

## Impact

- **UX**: User has no way to distinguish "agent thinking hard (no output yet)" from
  "agent waiting 60 s on a rate limit" from "agent genuinely stuck / hung." All three
  look identical: marching ants, no label, no ETA.
- **Trust**: A rate-limited pane that looks like it's working erodes trust —
  especially if the backoff is long (Claude's default is up to 60 s per retry, and
  it retries multiple times).
- **Support surface**: Users are likely to force-close or restart the pane during
  the backoff, which may create orphaned subprocess state (not confirmed, but a risk
  vector worth investigating).

---

## Fix

### Immediate (frontend only, no backend change)

Translate `rate_limit_event` in `claude-translator.ts` into a lightweight status
event that the reducer can consume:

```ts
// claude-translator.ts — add to the line-dispatch switch
case "rate_limit_event": {
    const retryMs = (parsed as any).retry_after_ms as number | undefined;
    events.push({
        type: "provider_waiting",
        reason: "rate_limited",
        retryAfterMs: retryMs ?? null,
    });
    break;
}
```

Add a `ProviderWaiting` command to the reducer that:
- Updates `lastEventMs` (keeps the watchdog quiet)
- Sets a `waitingReason` field on the `Streaming` turn phase
- Emits a `provider-waiting` audit event for diagnostics

In `agent-view.tsx`, when `turnPhase.kind === "Streaming" && turnPhase.waitingReason === "rate_limited"`,
show a status annotation below the progress bar: `"Rate limited — retrying in Xs…"`.

### Follow-up

- Extend the same pattern to `Overloaded` (HTTP 529, also emitted as
  `{"type":"system","subtype":"overloaded"}` by some CLI versions).
- Consider reducing `STUCK_THRESHOLD_MS` from 45 s → 20 s and making
  `stream-stuck` show a soft user-visible banner ("No response for 20 s — the
  agent may be retrying or the API may be slow") rather than just logging.
- Add the same `provider_waiting` translation to `codex-translator.ts` and
  `gemini-translator.ts` for their respective rate-limit event types.

---

## Files

| File | Relevance |
|------|-----------|
| `frontend/app/view/agent/providers/claude-translator.ts` | Missing `rate_limit_event` case — **primary fix site** |
| `frontend/app/view/agent/useAgentStream.ts:68` | `WATCHDOG_INTERVAL_MS = 5_000` |
| `frontend/app/store/agent-pane-state/types.ts:649` | `STUCK_THRESHOLD_MS = 45_000` |
| `frontend/app/store/agent-pane-state/reducer.ts:255–276` | `StreamWatchdogTick` — diagnostic only, no visible transition |
| `frontend/app/store/agent-pane-state/types.ts:318` | `workingFromPhase()` — drives animation |
| `agentmux-srv/src/backend/blockcontroller/health.rs:353` | Backend correctly classifies `rate_limit_event` |
| `docs/specs/SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` | Broader error-surface framework; `rate_limited` class is defined here |

---

## Lessons

1. **Health subsystem ≠ frontend awareness.** Classifying an event server-side is
   not the same as surfacing it. When a new event type is added to the health
   classifier, add a corresponding translator case in the same PR — otherwise the
   gap is invisible until a user hits it.

2. **"Active" animation needs a "waiting" sub-state.** The binary active/stopping
   progress bar can't communicate nuance. A rate-limit retry, a slow model, and a
   hung subprocess all look identical. Distinguishing "working" from "waiting on
   provider" is user-trust-critical.

3. **Diagnostic-only watchdog is a false safety net.** `stream-stuck` fires but
   doesn't act — giving false confidence that the system self-monitors, when in
   practice it only logs. A watchdog that doesn't change visible state or prompt
   recovery isn't a watchdog; it's a log sink.
