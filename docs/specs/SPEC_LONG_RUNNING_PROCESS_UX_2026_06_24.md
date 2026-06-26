# SPEC — Long-Running Process UX: Working Stuck + Red X

**Date:** 2026-06-24
**Status:** Analysis complete — fix coded, PR pending

---

## Symptom

When an agent pane's subprocess crashes or is killed mid-turn:
1. A red X (failure recovery banner) appears immediately.
2. The pane header stays in "Working…" indefinitely.
3. The only escape is navigating away from the pane (unmounting the component).

The user is stuck looking at a red X AND a spinning "Working" simultaneously.

---

## System map

The agent pane's turn lifecycle is owned by a pure reducer (`agent-pane-state/reducer.ts`) with these states:

```
Idle → Submitting → Streaming → Done
                ↘              ↗
              Interrupting
                ↓ (on StreamUnsubscribe while working)
            Disconnected
```

Two parallel subsystems feed into this:

| Subsystem | Source | Action on crash |
|---|---|---|
| `useAgentStream` | File subscription (subprocess stdout) | Fires `StreamUnsubscribe` on **component unmount** only |
| `useAgentFailure` | `AgentFailure` WPS event | Updates failure signal — drives the red X banner — but does NOT touch TurnPhase |
| `useControllerStatusEvents` | `ControllerStatus` WPS event | Logs to the activity panel — does NOT touch TurnPhase |

---

## Root cause

### The gap: Streaming has no bounded exit path on crash

The reducer has bounded timeouts for two states:
- `Submitting` → `Done.errored` after **30 s** (`SubmitTimeoutElapsed`)
- `Interrupting` → `Done.interrupted` after **5 s** (`InterruptTimeoutElapsed`)

**`Streaming` has no bounded timeout at all.**

Escape paths from `Streaming`:
1. `session_end` arrives → `finalizeTurn()` → `TurnEnd` → `Done.completed`  
2. User presses Esc → `RequestStop` → `Interrupting` → timeout or `session_end` → `Done`
3. Component unmounts → `onCleanup` fires → `subscription.unsubscribe()` + `StreamUnsubscribe` → `Disconnected`

**Missing: process crashes → `Streaming` stays forever.**

When the subprocess exits (crash, OOM, signal), `ControllerStatus: done` fires. But:
- The frontend file subscription (`fileSubject`) is STILL open (it watches a file, not the process directly)
- The component is NOT unmounted
- No `session_end` comes (crash doesn't emit one)
- `AgentFailure` fires → failure banner shows red X → but TurnPhase is still `Streaming`
- The 45-second stuck-stream watchdog fires a **diagnostic** `stream-stuck` event but does NOT force any phase transition

Result: the pane is stuck in `Streaming` until the user navigates away (unmounts the pane).

### Why `session_end` doesn't come on crash

For persistent/interactive mode (Claude Code between turns): the fix in PR #1757 emits `session_end` when a non-partial assistant message has no tool_use. That covers the normal inter-turn case.

On a CRASH (process killed, OOM, non-zero exit), no `session_end` is emitted at all. The file output just ends mid-stream or ends without the result event. There is no graceful teardown.

---

## The fix

In `useAgentStream.ts`, inside `onMount`, add a `ControllerStatus: done` listener with a 1.5-second grace-period timer:

```typescript
let procExitGraceTimer: number | null = null;
const procExitUnsub = waveEventSubscribe({
    eventType: WpsEvent.ControllerStatus,
    scope: WOS.makeORef("block", blockId),
    handler: (event) => {
        const status = (event as any)?.data?.shellprocstatus;
        if (status !== "done") return;
        if (procExitGraceTimer != null) return; // already armed
        procExitGraceTimer = window.setTimeout(() => {
            procExitGraceTimer = null;
            const phase = paneSnapshot(blockId)?.turnPhase?.kind;
            if (phase === "Streaming" || phase === "Submitting") {
                model.dispatchPane({ type: "StreamUnsubscribe", at: Date.now() });
            }
        }, 1500);
    },
});
```

**Why 1.5s grace period?** Buffered output in the IPC pipe arrives within milliseconds of process exit. 1.5s matches the existing stop-fallback-timer timeout for Esc presses — same reasoning: any `session_end` that's coming will have arrived by then.

**Safety analysis for each scenario:**

| Scenario | Phase when timer fires | Action |
|---|---|---|
| Clean exit — `session_end` arrived | `Done.completed` | No-op ✓ |
| Process crash mid-Streaming | `Streaming` | `StreamUnsubscribe` → `Disconnected` ✓ |
| Process crash during submit | `Submitting` | `StreamUnsubscribe` → `Disconnected` ✓ |
| Auto-retry — new process started | `Idle` (file truncate → `TurnReset`) | No-op ✓ |
| Persistent mode (process never exits between turns) | `Done.completed` (via `session_end`) | No-op ✓ |
| User pressed Esc within 1.5s | `Done.*` or `Idle` | No-op ✓ |

**Why not `Done.errored` instead of `Disconnected`?**

`Disconnected` is the right choice because:
1. Auto-retry will immediately (or after countdown) start a new process → `StreamSubscribe` from `Disconnected` → `Idle` → new turn. The `Disconnected` → `Idle` transition via `StreamSubscribe` is cleaner than `Done.errored` → `Idle` (which requires a `TurnReset`).
2. The failure banner (`AgentFailure` → `useAgentFailure`) already owns the "what failed and what to do" UX. The TurnPhase only needs to stop showing "Working."

---

## Secondary issues found

### 1. `ControllerStatus: done` fires for ALL exits, including clean

The timer fires on every process exit, including normal turns. The phase check (`Streaming` or `Submitting`) prevents false positives, but we still schedule a 1.5s timer on every clean exit. Cost: one setTimeout per turn. Acceptable.

### 2. The 45s watchdog is diagnostic-only

`StreamWatchdogTick` emits `stream-stuck` but the event has no consumers that force a phase transition. The Swarm view's "still working" indicator does NOT use this. This is a separate issue — the watchdog is useful for telemetry but doesn't unblock the user.

Potential improvement: route `stream-stuck` to the failure system (create a synthetic `AgentFailure` with `code: "stream-stuck"`) so the user gets a dismissible banner after 45s of silence even without a crash. Out of scope for this fix.

### 3. No visibility into "working" for very long tool calls

When Claude Code runs a long bash command (e.g., `npm install`, compilation), the pane shows "Working…" with the tool name. There's no indication of elapsed time or progress. The only feedback is the live log stream in the tool block.

Potential improvement: add an elapsed-time counter to the Streaming phase display (e.g., "Working… 2m 30s"). Tracked separately.

### 4. Disconnected vs. Crashed UX confusion

When `StreamUnsubscribe` fires after a crash, the pane enters `Disconnected`. If the agent-view shows "Reconnecting…" text for `Disconnected`, this is misleading for a crash. For a crash, the `AgentFailure` banner should be the primary signal (it already shows correctly). The `Disconnected` state's "Reconnecting" text (if any) should be suppressed when a failure row is active.

---

## Files changed

| File | Change |
|---|---|
| `frontend/app/view/agent/useAgentStream.ts` | Add `ControllerStatus: done` → 1.5s grace-period → `StreamUnsubscribe` |
| `.changesets/…patch…` | Changeset for release tracking |

---

## Testing

1. Start an agent turn with a long-running bash command (e.g., `sleep 10`).
2. Kill the Claude Code subprocess (find its PID via muxlog, `taskkill /PID <pid> /F`).
3. **Before fix:** pane stuck in "Working…" indefinitely.
4. **After fix:** within 1.5s, pane transitions to `Disconnected`, "Working…" clears, failure banner shows.

Also verify clean exits still work:
1. Send a simple message, wait for response.
2. "Working…" should clear at `session_end` as before — no regression.

---

## Related specs

- `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` — failure banner (§4–§6)
- `SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` — AgentFailure classification
- `docs/analysis/agent-pane-document-reducer-2026-05-03.md` — reducer design
- `RETRO_REPLACECHILD_CRASH_2026-06-06.md` — why SessionEnd is deferred to microtask
