# Report: why "Working…" appears with no reason, and what telemetry would let an agent self-diagnose it

**Date:** 2026-07-27
**Author:** Agent3
**Verified against:** `main` @ `128633ced`.
**Status:** Audit + telemetry design — root-cause catalog complete, concrete instrumentation sketched, not yet implemented.
**Triggered by:** a live incident this session — my own agent pane appeared to still be "Working…" after I'd finished all requested work. Root cause turned out to be a leftover `task dev` background bash process (heartbeat-wrapped to survive the sandbox's idle-output timeout) — structurally the exact same shape as the Agent1 incident below. I had no quick way to confirm this from logs; I had to reason it out and go check running processes directly.

## User's request (verbatim, for traceability)

> write we squash it .. figure out how we can get the right telemetry hooked in so agents can quickly debug it when it comes up. the issue is a "Working..." appears for no reason .. it related to long running processes too .. lets do an audit and write report to file

## Prior art this report builds on (not re-derived)

- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` — Agent1's pane read "Working…/Waiting…" for ~12 hours; root cause was a genuine `run_in_background: true` heartbeat-guarded dev stack, not a crash. The retro's own §3 lesson: *"turn-phase and attached-process-liveness are orthogonal — a live process is not the same signal as a live turn."*
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` — 5-step fix plan. **Step 1 shipped** this session (PRs #2315, #2317 — `frontend/app/view/agent/activity/tool-adapter.ts`, promotes a Bash call to the `ActivityDock` after 30s and suppresses `AgentWorkingRow`'s "tool · arg" text once promoted). **Steps 2–5 (`run_in_background` threading, two-axis pane status + watchdog exemption, feeding Swarm, generalizing beyond Bash) remain unimplemented** — confirmed fresh via `grep -r run_in_background` (zero hits outside `docs/`) and `git log` on the Swarm-touching files since 07-16 (all row-grouping/UX polish, no new status axis).
- `docs/specs/REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md` — identifies `agentmux-bashwrap`'s own 600s idle-kill as a *seventh*, structurally isolated liveness mechanism that can silently terminate a healthy long-running task with zero dock entry or trace anywhere in the UI.

This report's job is narrower than those: **not** "fix long-running-process visibility" (already scoped in the report above) but **"when a pane says Working and it's not obvious why, what can an agent actually grep to find out right now — and what's missing."**

## 1. Every distinct way "Working" can be true with nothing actually happening

All confirmed directly against `frontend/app/store/agent-pane-state/{types,reducer}.ts`.

`TurnPhase` (`types.ts:133-171`): `Idle | Submitting | Streaming | Interrupting | Done | Disconnected`. `workingFromPhase` (`types.ts:332-335`) is `true` for `Submitting | Streaming | Interrupting`. Watchdog constants: `STUCK_THRESHOLD_MS = 45_000` (diagnostic-only, `types.ts:720`), `LIVENESS_RECOVERY_MS = 180_000` (force-recovers, `types.ts:733`).

The dominant gate (`reducer.ts:308-314`, `StreamWatchdogTick`):
```ts
const phase = state.turnPhase;
if (phase.kind === "Streaming" && phase.toolsActive === 0) {
    const recoverThresholdMs = phase.waitingReason === "rate_limited"
        ? (phase.retryAfterMs ?? 0) + LIVENESS_RECOVERY_MS
        : LIVENESS_RECOVERY_MS;
    if (idleSinceMs >= recoverThresholdMs) { /* force → Idle */ }
}
```
**Any tool call in flight (`toolsActive > 0`) unconditionally exempts the pane from recovery, forever** — correct in principle (a genuinely-running tool must not be force-cleared), but nothing distinguishes "tool genuinely progressing" from "tool silently hung."

Nine distinct false-positive paths, ranked roughly by how often they'd actually bite:

1. **A `run_in_background: true` (or plain long) Bash call** (the Agent1 shape, and this session's own incident). `ToolStart`/`ToolEnd` (`reducer.ts:588-608`) only mutate `currentTool`/`toolsActive` on the `Streaming` payload — never end the phase. Per the gate above, the watchdog can never intervene while `toolsActive > 0`, however long the tool runs.
2. **The 30s dock-promotion window itself.** `TOOL_PROMOTION_MS = 30_000` (`tool-adapter.ts:35`) — before a Bash call crosses it, `AgentWorkingRow` still shows raw "`tool · arg`" with no dock row, no elapsed-time framing: an early, real instance of "why is this still Working" with nothing to point at yet.
3. **A bashwrap idle-kill with zero surfaced signal** (per the sibling report §1.3/§3.1) — a 600s zero-stdout timeout can terminate a process tree with no dock entry and no notification; whatever turn-phase state existed at that moment has no mechanism to notice the process is simply gone.
4. **A rate-limit retry loop that itself stalls.** `reducer.ts:293-305`'s own comment: recovery waits `retryAfterMs + LIVENESS_RECOVERY_MS`, assuming the CLI re-emits `rate_limit_event` on schedule (refreshing `lastEventMs`). If the CLI emits one and then goes fully silent — no follow-up, no `session_end`, no exit — the pane is pinned for that full window, indistinguishable from a genuine hang the whole time.
5. **An orphaned turn that never receives `TurnEnd`** — the explicit design premise of the watchdog (`reducer.ts:283-291`'s comment: *"For persistent agents no `ControllerStatus: done` will ever arrive… the watchdog itself must transition out of Working"*). Indistinguishable from real work until 180s+ elapses.
6. **`ReconcileTurnActive` disagreement window** (`reducer.ts:345+`, `types.ts:380-406`) — frontend can read `Streaming` after the backend's authoritative `turn_active` already flipped false; resolves only on the next live `controllerstatus` event or the watchdog, whichever comes first.
7. **`StreamFlushObserved`'s broad re-promotion rule** (`reducer.ts:243-253`) — intentionally promotes `Idle`/`Disconnected`/`Done.completed` back into `Streaming` on any flush (needed for multi-round tool continuations), so a late/stray flush after what looked like completion can silently re-arm "Working."
8. **Rate-limit UI collapse masking a stalled retry.** `waitingReason: "rate_limited"` renders "Rate limited — retrying…" (`AgentFooter.tsx`) — visually distinct, but internally still just `Streaming`; if the retry loop dies (#4), there's no separate "the retry itself looks stuck" affordance.
9. **Composer-perception mismatch** — not a false "Working" itself, but compounds every case above: the composer is never actually locked (`PendingMessageQueued` already lets you type/send mid-turn), yet the persistent banner visually reads as "don't touch this."

What currently drives the visible banner (`AgentFooter.tsx`'s `AgentWorkingRow`, already modified this session): `loading`, `currentTool`/`currentToolArg`, and the new `toolPromoted` prop (from step 1 above). `loadingLeftText` precedence: stopping → rate-limited → launch-phase label → `currentTool` (only if `!toolPromoted`) → generic cycling "Working…" phrase. **This changes what text is shown once a tool crosses 30s — it does not change whether "Working" shows at all.** Every path above is still fully reachable; step 1 only improved case #2's presentation, not cases #1, #3–9.

## 2. What's grep-able today — confirmed almost nothing

### 2.1 The reducer itself: zero logging

`reducer.ts` is a pure function. Confirmed via grep: **zero** `console.*`/`Logger.*` calls anywhere in `frontend/app/store/agent-pane-state/`. Every transition only *returns* an event — it never emits anything observable.

### 2.2 The one place events get consumed (`agent-pane-state-store.ts`)

`dispatch()` (`agent-pane-state-store.ts:169-249`) is the sole consumer. Its default `eventSink` (`:100-106`) logs **exactly one** event type via `console.warn`: `turn-start-suppressed`. Every other event — `turn-ended`, `working-recovered`, `stream-stuck`, `tool-started`, `tool-ended`, `provider-waiting`, `failure-observed`, `turn-active-reconciled-at-mount`… — produces **no console output at all**. (The file's other `console.warn`, `CASCADE_DETECTED` at `:219-224`, is an unrelated reactive-teardown guard.)

There's an in-memory ring buffer (`recordDispatch()`, `command-source.ts`, `RING_CAPACITY = 500`) feeding a dev-only diagnostic panel (`app/devtools/diag-panel.tsx`, Ctrl+Shift+D inside the live CEF window). It logs command **type** only (no phase payload, no reasoning), requires a live window + keyboard shortcut + manual cross-referencing, and is invisible to `muxlog` entirely.

### 2.3 The one thing that already does this right — `[wave-title]`

`app-init.ts:857-867`, `installWindowTitleEffect`'s closing line:
```ts
console.debug(
    "[wave-title]",
    "windowId=" + windowId, "label=" + (myLabel() ?? "<unknown>"), "idx=" + idx,
    "idxSource=" + idxSource, "displayName=" + (displayName ?? "<none>"),
    "workspaceName=" + (workspaceName ?? "<none>"), "tab=" + (tab?.name ?? "<none>"),
    "→ title=" + JSON.stringify(title),
);
```
Its own comment: *"Goes through frontend's `[fe]` log pipe → host log; tail with `muxlog host '\[fe\] \[wave-title\]'`."* Every input the resolution used is dumped on every re-run. This is exactly the pattern this report recommends generalizing — **and no equivalent exists for turn-phase.** An agent debugging "why does this say Working" today has nothing to grep in `muxlog host`, nothing in `muxlog srv`, and only the dev-only diag panel in-browser (command types, no reasoning).

### 2.4 Tool-call visibility never reaches the backend at all

Confirmed via the auto-detect report's §2.1 (re-verified): the Bash command string arrives via `event.params.command` on the CLI's own `tool_call` NDJSON event, parsed entirely client-side (`useAgentStream.ts`'s `extractToolArg`). There's no backend RPC boundary per tool call — srv never sees "a Bash call started/ended," only the CLI's raw stdout, which lands in the sidecar's transcript log (excluded from `muxlog` by default as "transcript noise" per `docs/MUXLOG.md`). **This is correctly scoped as-is — no protocol change needed here** — but it means backend-side telemetry can never directly see individual tool calls, only process-level and turn-level signals (below).

### 2.5 Backend-side signals exist but are logged too coarsely (or not at all)

- `agentmux-srv/src/backend/process_tracker/registry.rs`: `poll_and_emit()` (every ~2s) diffs PID membership and publishes straight to the WPS broker — **no `tracing::` call inside it at all**. Only one-time lifecycle events (`ensure_tracker`, `remove`) log. `agent:process-added`/`-exited` are confirmed invisible to `muxlog srv`. Also: `TrackedProcess.started_at_ms` is hardcoded `0` on Windows (`windows.rs:198`, "deferred; skip for v1") — process *age* isn't even populated on this platform yet.
- `agentmux-srv/src/backend/blockcontroller/health.rs`: `mark_turn_active_returning_was_active`/`set_exited`/`record_output` log nothing directly. Only `evaluate_and_transition`'s coarse derived `AgentHealth` enum (Idle/Stalled/Dead/Exited) logs, and only on a state *change* — not a raw `turn_active` flip, not a duration.

Net: real backend-side data exists (process membership, health-state transitions) but at too coarse a grain to answer "how long has this turn actually been active" from `muxlog` alone.

## 3. Recommended telemetry (sketch, not implemented)

Four additions, ordered by value ÷ blast-radius. All are logging-only — no reducer logic changes, no new state, reading data these systems already compute.

### 3.1 Watchdog reasoning — highest priority

The reducer already computes exactly why it did or didn't recover a pane (`reducer.ts:308-332`) and currently discards that reasoning. Special-case the two watchdog event types where `dispatch()` already iterates `result.events` (`agent-pane-state-store.ts:227`):

```ts
for (const ev of result.events) {
    if (ev.type === "stream-stuck") {
        const p = slot.state.turnPhase;
        const exempt = p.kind === "Streaming" && p.toolsActive > 0;
        console.debug(
            "[wave-turn]", `pane=${blockId.slice(0, 7)}`,
            `watchdog: no recovery — idleSinceMs=${ev.idleSinceMs} thresholdMs=${ev.thresholdMs}`,
            exempt ? `EXEMPT toolsActive=${p.toolsActive} currentTool=${slot.state.currentTool ?? "?"}` : "",
        );
    } else if (ev.type === "working-recovered") {
        console.debug(
            "[wave-turn]", `pane=${blockId.slice(0, 7)}`,
            `watchdog: FIRED — force-recovered to Idle, idleSinceMs=${ev.idleSinceMs}`,
        );
    }
    eventSink(blockId, ev);
    /* ... existing multicast loop ... */
}
```
This directly resolves cases #1, #4, #5, #6 above by inspection — e.g. `EXEMPT toolsActive=1 currentTool=Bash` immediately followed by no further `[wave-turn]` lines for 10 minutes *is* the Agent1/this-session signature, visible from `muxlog host '\[fe\] \[wave-turn\]'` with no CEF window needed.

### 3.2 A `[wave-title]`-shaped transition line on every `TurnPhase` change

Same function, comparing `prev.turnPhase` (already captured at `agent-pane-state-store.ts:180`) against the post-dispatch value:
```ts
if (prev.turnPhase !== slot.state.turnPhase) {
    console.debug(
        "[wave-turn]", `pane=${blockId.slice(0, 7)}`,
        `${prev.turnPhase.kind} → ${slot.state.turnPhase.kind}`, `cmd=${command.type}`,
        `toolsActive=${slot.state.turnPhase.kind === "Streaming" ? slot.state.turnPhase.toolsActive : "-"}`,
        `currentTool=${slot.state.currentTool ?? "-"}`,
    );
}
```
Mirrors the `[wave-title]` precedent exactly (same tag-prefix convention `muxlog` already documents filtering on) — one call site, no new dependency, answers "which command caused this transition and what was the tool state at that moment" for every case in §1, not just the watchdog ones.

### 3.3 Backend: log the `turn_active` flip directly

`health.rs`'s `mark_turn_active_returning_was_active`/`set_exited` currently only cause a log line indirectly (via the coarse derived health enum, and only on a category change). Add a direct `tracing::debug!(block_id, was_active, "[health] turn_active flip")` at both call sites — one line per turn boundary per pane, not per poll. Gives `muxlog srv` a backend-authoritative timeline to cross-reference against §3.2's frontend line, closing the "two clocks disagreeing" blind spot case #6 names and the Agent1 retro's own §3 lesson calls out as previously unconfirmable without a live repro.

### 3.4 Backend: process-registry long-lived logging — blocked on a prerequisite

`registry.rs`'s `poll_and_emit` could log once (not per-2s-tick — same one-shot discipline `tool-adapter.ts`'s `nextToolPromotionAt` already uses) when a tracked process crosses a duration threshold. **Blocked on `started_at_ms` being hardcoded `0` on Windows** (`windows.rs:198`) — the actual prerequisite is fixing process-age tracking, not the logging itself. Lowest priority of the four; note as a dependency, not a next step.

## 4. What this report does *not* do

- Does not re-litigate or re-implement the long-running-process report's steps 2–5 (`run_in_background` threading, two-axis status, Swarm feed, generalize-beyond-Bash) — those remain their own, already-scoped follow-up.
- Does not propose moving tool-call parsing into the backend (§2.4) — correctly frontend-only today; the backend telemetry gap is specifically about process/turn-level signals it *already* computes, not new protocol surface.
- Does not implement anything — this is the audit + design the user asked for before squashing; §3's four items are sized to land as one small, low-risk PR (logging-only, no behavior change) once reviewed.

## 5. Key files

| Concern | File | Line(s) |
|---|---|---|
| `TurnPhase` state machine, watchdog constants | `frontend/app/store/agent-pane-state/types.ts` | 133-171, 720, 733 |
| Watchdog gate (`toolsActive === 0`) | `frontend/app/store/agent-pane-state/reducer.ts` | 272-343 |
| `dispatch()` — where all four telemetry additions land | `frontend/app/store/agent-pane-state-store.ts` | 169-249 |
| Existing single-event-type `eventSink` (precedent for the pattern, narrower) | `frontend/app/store/agent-pane-state-store.ts` | 100-106 |
| In-memory dispatch ring (dev-only, not muxlog-visible) | `frontend/app/store/command-source.ts` | 57-87 |
| Dev-only diagnostic panel (Ctrl+Shift+D) | `frontend/app/devtools/diag-panel.tsx` | full |
| `[wave-title]` precedent to mirror | `frontend/app-init.ts` | 797-872 |
| `AgentWorkingRow` — what currently renders the banner | `frontend/app/view/agent/components/AgentFooter.tsx` | 57-121 |
| Step-1 dock-promotion fix (already shipped) | `frontend/app/view/agent/activity/tool-adapter.ts` | full |
| Backend process registry (no per-poll logging) | `agentmux-srv/src/backend/process_tracker/registry.rs` | 150-186 |
| Backend health monitor (coarse-only logging) | `agentmux-srv/src/backend/blockcontroller/health.rs` | 192-279 |
| `started_at_ms` hardcoded 0 on Windows (blocks §3.4) | `agentmux-srv/src/backend/process_tracker/windows.rs` | 198 |
| Prior incident retro | `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` | full |
| Long-running-process report (steps 2-5 still open) | `docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` | full |
| Sibling bashwrap-idle-kill report | `docs/specs/REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md` | full |
