# Retro — Agent2 (stable channel) stuck in a "Queued" state; held message never flushed (2026-07-16)

## TL;DR

Agent2 (host/persistent, `stable` channel, v0.53.6) showed a queued message
stuck in the "Queued — sends at the agent's next step" state for ~4 hours. Its
CLI turn had ended cleanly at 10:43 and the process went genuinely idle (0% CPU
over a 5s sample), so the "next step" that would flush the queued message
should have arrived immediately — but didn't. Root cause is the **same one as
the Agent1 incident, from the opposite direction**: the frontend `turnPhase`
got stuck in a working state (`Streaming`) because it missed the turn-end
signal, and PR #2005's mount reconciliation only promotes `Idle → Streaming`
(backend says busy) — it never demotes `Streaming → Idle` when the backend's
authoritative `turn_active` says the turn is done. Agent1 surfaced that gap as
a stuck busy *indicator*; Agent2 surfaces it as a stuck *held-message flush*,
because the auto-flush is gated on the turn reaching Idle/Done.

## Evidence

Live block meta (`db_block` oid `e5eb5b88-…`, from the running srv's actual
`--wavedata` dir `…\channels\stable\versions\0.53.6\data`):
- `controller: persistent`, `session:active_pid: 108692`.
- `session:last_activity_ms` frozen at **2026-07-16T10:43:07Z** (~4h before
  investigation) — the CLI stopped emitting output there.
- `term:ambient_summary`: "GPU policy PR 2182 done awaiting renderer reclaim or
  threshold" — the agent's own summary of a *completed* task.

Process: `claude.exe` PID 108692 alive; CPU sampled twice 5s apart → **0.0
delta**. The CLI is idle, not busy-looping.

Transcript (`…/projects/…agent2-0630f/20acf26e-….jsonl`, 6433 lines):
- Ends with assistant `[thinking]` then `[text]` at 10:43:07 ("Recorded and
  pushed to PR #2182…") — a clean turn wrap-up, **no trailing `tool_use`**, so
  the turn genuinely ended.
- Contains `type=queue-operation` entries at 10:42:16 — direct evidence the
  CLI's own message queue was exercised in this session (a send-while-busy).
- The only unresolved `tool_use` is a `TaskOutput{block:true}` from
  **2026-07-05** — 11 days stale, from prior session context, not the current
  turn.

## Root cause

For a **persistent** agent there is no backend-side message queue:
`PersistentSubprocessController::send_message`
(`agentmux-srv/src/backend/blockcontroller/persistent.rs:293-356`) writes the
user message straight to the CLI's stdin and emits `agent-message-accepted`
**immediately** (line 354). So "Queued" is purely a **frontend** state: a
message the user sends while the pane's `turnPhase` is a working kind is *held*
frontend-side (`heldQueue` in `useAgentCommands.ts`, shown by
`PendingMessagesPanel` as "Queued"), and only submitted to the backend when the
turn reaches a boundary.

That flush is an effect in `agent-view.tsx:704-713`:

```js
const turnIdle = phaseKind === "Idle" || phaseKind === "Done";
if ((newToolCall || turnIdle) && commands.hasHeldMessages()) {
    void commands.flushHeldMessages();
}
```

It delivers held messages at the next tool-call boundary **or** when the turn
goes Idle/Done. Agent2's turn ended (backend emitted the `result` event →
`health_monitor.set_active_turn(false)`, persistent.rs:845-846), so the backend
knew the turn was over. But the **frontend** `turnPhase` never transitioned to
Done/Idle — it missed the turn-end `session_end` (the pane was unmounted /
backgrounded across the transition, or the event was otherwise not observed).
With `phaseKind` stuck at `Streaming`, `turnIdle` stays false, `newToolCall`
never fires again (turn is over), so the held message is **never flushed** —
stuck "Queued" indefinitely.

PR #2005 added exactly the reconciliation that would fix this — except only in
one direction. `ReconcileTurnActive` (reducer.ts) promotes a mount-default
`Idle → Streaming` when backend `turn_active` is true; a `false` value is a
**no-op** (`if (!command.active || state.turnPhase.kind !== "Idle") return …`).
So a frontend stuck *above* Idle is never pulled back down to match a backend
that says the turn is done.

## Relationship to the Agent1 incident (same root cause)

| | Agent1 (retro-persistent-agent-working-status-stuck) | Agent2 (this) |
|---|---|---|
| Backend state | turn done / long-running bg process | turn done (`turn_active=false`) |
| Frontend `turnPhase` | stuck working | stuck `Streaming` |
| Visible symptom | stuck "Working…/Waiting…" indicator | stuck "Queued" held message |
| Missing mechanism | downward reconciliation + no attached-process axis | **downward reconciliation** |

Both are the frontend turn-phase failing to follow the backend down to Idle.
The **single fix that resolves both** is completing #2005's symmetry: when the
backend's authoritative `turn_active` is `false` and the frontend phase is a
stuck working kind with no live local turn activity, reconcile it to `Idle`.
For Agent1 that clears the indicator; for Agent2 that makes `turnIdle` true so
the held-message flush finally fires.

## Fix direction

1. **Complete the reconciliation symmetry (primary).** Extend
   `ReconcileTurnActive` (or add a companion command) so `active === false`
   demotes a stuck `Streaming`/`Submitting` phase to `Idle` — guarded so it
   never interrupts a genuinely-live local turn (only acts when the frontend
   has seen no stream activity within a small window, mirroring the liveness
   watchdog's own guard). Drive it from the same live `ControllerStatus`
   subscription #2005 already wired for the Swarm view, not just at mount, so
   a backgrounded pane reconciles as soon as it remounts (and ideally while
   unmounted, via the fleet-level status path).
2. **Backstop the flush.** Make the held-message flush also trigger on the
   backend `turn_active=false` signal directly, not solely on the derived
   frontend `turnIdle` — defense in depth so a queued message can't be
   stranded even if the phase reconciliation is delayed.
3. Both are the concrete, mechanism-level realization of the "two liveness
   signals (backend `turn_active` vs frontend `turnPhase`) must not silently
   disagree" seam called out in the Agent1 retro and the consolidation report
   (REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md).

## Files

| File | Relevance |
|------|-----------|
| `frontend/app/view/agent/agent-view.tsx:704-713` | Held-message auto-flush, gated on `turnIdle` (Idle/Done) — never fires while phase stuck at Streaming (**primary symptom site**) |
| `frontend/app/store/agent-pane-state/reducer.ts` (`ReconcileTurnActive`) | Reconciles Idle→Streaming only; needs the Streaming→Idle direction (**primary fix site**) |
| `frontend/app/view/agent/hooks/useAgentCommands.ts` (`heldQueue`, `flushHeldMessages`) | Frontend-only held-message queue |
| `agentmux-srv/src/backend/blockcontroller/persistent.rs:293-356,845-867` | Persistent send writes straight to stdin + emits accepted immediately; `result` event flips backend `turn_active=false` and publishes it |
| `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` | Agent1 incident — same root cause, other symptom |
| `docs/specs/REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md` | Consolidation report; this strengthens its two-axis-status / reconciliation direction |

## Lessons

1. **A "flush at the next step" that depends on a possibly-stuck phase is a
   deadlock waiting to happen.** The held-message flush trusts the frontend
   `turnPhase` to reach Idle/Done; when that phase can silently get stuck, the
   queue silently strands. Gate such flushes on the *authoritative* signal
   (backend `turn_active`), or provide a reconciliation that guarantees the
   phase converges.

2. **Reconciliation must be symmetric.** #2005 fixed "frontend stuck Idle while
   backend busy" but left "frontend stuck busy while backend idle" — and the
   latter has at least two distinct victims (the busy indicator and the
   held-message flush). One-directional reconciliation reads as "fixed" until
   the other direction bites.

3. **Two agents, two symptoms, one bug.** Agent1 and Agent2 looked like
   different problems ("stuck Working" vs "stuck Queued") but are the same
   frontend-phase-doesn't-follow-backend-down defect. Worth fixing at the
   shared cause, not per-symptom.
