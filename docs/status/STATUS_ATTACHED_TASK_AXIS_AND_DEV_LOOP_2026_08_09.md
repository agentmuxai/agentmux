# Status: Long-Running Attached Tasks — Why "Working…" Is Still Broken and `task dev` Still Fights the Agent Loop

> **SUPERSEDED 2026-08-29 (docs-cleanup Phase 3) — read
> `docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md`
> instead.** That doc continues this one's rung numbering, re-audits it
> against later code, and records what shipped in the interim. This snapshot
> is kept for the record, not as a current diagnosis.
>
> Both of the "next unstarted rungs" this ladder pointed at have since
> shipped and their issues are closed: **#2491** (bashwrap idle-timeout
> killing declared long-running tasks) by **#2589**, and **#2492**
> (backgrounded tasks dying on session teardown) by **#2590**/**#2681**/**#2683**.

**Date:** 2026-08-09
**Author:** Camper (camper-0622h)
**Status:** Diagnosis current as of `main` @ `18f424828`. Partial wiring in progress in this working tree (see §6).
*(Accurate when written — superseded, see the banner above.)*
**Supersedes nothing** — this consolidates and updates the status across:
- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` (Agent1: 12h "Working…" over a healthy dev server)
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` (Agent2: original design direction)
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` (Agent3: verification + 5-step plan)
- `docs/retro/RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md` (bashwrap idle-kill of a successful `task dev`)
- `docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md` (Agent A: reducer slice, wiring deferred)

## 1. The goal, restated plainly

An agent should be able to run `task dev` (or any long-lived server/GUI/watch
process) and then **return to idle**, with the process:

1. **visible** in the pane's ActivityDock as a running row (name, elapsed, stop button),
2. **not** pinning the pane's status on "Working…" forever,
3. **not** getting killed by infrastructure (bashwrap idle-timeout, session teardown),
4. and the pane's footer showing an honest third state — "Running: N background task(s)" — distinct from both "Working…" (model turn in flight) and idle.

None of these four are fully true today. Each has had real work land against
it; every piece stopped one step short of the last mile, and the pieces
interact so that fixing one in isolation makes another worse.

## 2. What has actually shipped (all verified against `main` today)

| Piece | Where | State |
|---|---|---|
| Foreground tool-call promotion (≥30s Bash call → dock row) | `frontend/app/view/agent/activity/tool-adapter.ts`, `TOOL_PROMOTION_MS = 30_000` | ✅ shipped, works |
| Working-row suppression once dock shows the tool | `hasRunningPromotedTool` → `toolPromoted` prop, `agent-view.tsx:1670` → `AgentWorkingRow` | ✅ shipped, wired |
| `attachedTask` reducer slice (`AttachedTaskState`, `AttachedTaskObserved`/`Cleared`) | `agent-pane-state/types.ts:123,638-643`, `reducer.ts:1241-1252` + tests | ✅ shipped — **but pure dead state** (see §3.1) |
| Turn-phase watchdog (force-recover hung `Streaming` → `Idle` after 180s quiet) | `types.ts:870` `LIVENESS_RECOVERY_MS = 180_000` | ✅ shipped — and is *defeated by design* by the heartbeat pattern (see §4) |
| `[wave-turn]` transition telemetry | `agent-pane-state-store.ts` dispatch logging | ✅ shipped |
| Bashwrap idle-kill of pager-hung children | `agentmux-bashwrap` (PR #2156) | ✅ shipped — and is *the direct cause* of §4's trap |

## 3. Why it still doesn't work — the four last-mile gaps

### 3.1 The `attachedTask` axis has zero producers and zero consumers

`SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md` §6 explicitly deferred the
wiring ("needs live UI verification") and **nobody ever picked it up**.
Verified today by grep: `AttachedTaskObserved` / `AttachedTaskCleared` appear
*only* in `types.ts`, `reducer.ts`, and tests. No dispatch call site exists
anywhere; no projection setter existed in `AgentPaneProjections`; no atom in
`createAgentAtoms`; nothing in `AgentFooter`/`AgentWorkingRow` reads it. The
state machine's answer to this whole problem class has been sitting in the
codebase for a week as a perfectly-tested no-op.

### 3.2 A backgrounded Bash call is invisible to the dock (step 2 never built)

`BashParams` is still `{ command, timeout }` (`frontend/app/view/agent/types.ts:122-125`)
— no `run_in_background` field. The consequence chain, observed live today:

- An agent runs `task dev` with `run_in_background: true`.
- The harness returns the tool result ("Command running in background with ID …") in **under a second**.
- The `ToolNode` goes terminal immediately → `everCrossedThreshold` is false → **never promoted, never docked**.
- The actual process tree (task.exe → cargo → Vite → launcher → CEF) runs for hours, completely invisible to the dock.

The 30s duration-promotion (step 1) only ever covers *foreground* calls. The
backgrounded case — the one that matters for dev servers — was step 2 of
Agent3's plan and remains unbuilt. This is the single highest-leverage gap.

### 3.3 No OS-level fallback exists either

`AgentProcessRegistry` on Windows still hardcodes `started_at_ms: 0`
(`agentmux-srv/src/backend/process_tracker/windows.rs:198`, "deferred; uses
NtQueryInformationProcess — skip for v1"), so a process-age heuristic for
detached work has no data even if we wanted one. (The spec's §4 rejects raw
process counts as the axis's source anyway, so this only blocks a *fallback*,
not the primary design.)

### 3.4 Nothing renders a third state even if the axis fired

`AgentWorkingRow` today renders exactly two states: loading ("Working…"/tool
text/rate-limited/launch-phase) and done ("✓ Worked · stats"). There is no
"Running: dev-agent.cmd (14m) [stop]" state. Until `AgentFooter`/`AgentWorkingRow`
consume `attachedTask`, wiring §3.1's dispatchers would change nothing visible.

## 4. The operational trap: every way to run `task dev` from an agent is currently wrong

Confirmed again live in this session (2026-08-09), all three arms:

| Method | What happens | Evidence |
|---|---|---|
| Plain background Bash call | **Killed** ~10 min after the GUI goes quiet — bashwrap's idle-timeout can't distinguish "silent because done-and-running-GUI" from "silent because pager-hung". Build succeeds, Vite up, launcher up, then the whole tree dies. | Today's run 1 (task `bxm992iar`, exit 1, `[bashwrap] command produced no output for the idle timeout…` as the final line) — exact repeat of `RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md` |
| Heartbeat-wrapped (`while true; sleep 120; echo …` + the real command) | **Survives** the idle-kill (120s < 600s) — but the 120s heartbeat also refreshes the pane's liveness clock faster than the 180s watchdog threshold, so the pane shows **"Working…" for the entire lifetime of the dev server** (the Agent1 12h incident mechanism). Also: the dock still shows nothing (§3.2), and the process **dies on agent-session teardown** — observed today: task `byoiy4c2v` stopped with no completion record when the session restarted. | Today's run 2 + `retro-persistent-agent-working-status-stuck-2026-07-16.md` |
| Fully detached (`Start-Process`) | Survives everything — but invisible to **both** the dock and `AgentProcessRegistry`; the user cannot discover or stop it from the app at all. Explicitly demoted to last-resort in the 07-31 retro. | 07-31 retro, verified then |

The coupling is the point: **the bashwrap idle-kill forces the heartbeat, and
the heartbeat pins the busy indicator.** These two "fixes" from different
weeks are fighting each other. No amount of additional detection polish fixes
this — the resolution has to make the *attached-task state first-class* so
that (a) bashwrap can know "this is a declared long-running task, don't
idle-kill it," and (b) the pane can know "this quiet is healthy, show
Running-not-Working."

## 5. Why prior passes each stopped short (honest accounting)

1. **07-16 (Agent2 report):** correct diagnosis, design-only — nothing shipped.
2. **07-26 (Agent3 report):** verified the diagnosis, wrote the 5-step plan; shipped step 1 (foreground promotion) + the suppression signal. Steps 2–5 deferred.
3. **07-31 (idle-kill retro):** fixed the *operational* symptom with the heartbeat pattern; explicitly documented (its own §"Known side effect") that this pins the Working indicator — deferred the fix to the status-model work.
4. **08-02 (Agent A spec):** shipped the reducer slice; deferred all wiring per the project rule "stop and ask before anything needing live/human UI testing." The ask apparently never got answered, so the wiring never happened.
5. **08-09 (this session):** started the deferred wiring (§6), interrupted mid-change by session teardown — which itself killed the running dev instance, adding "background tasks don't survive session restarts" to the evidence pile.

Pattern: each pass fixed the layer it was scoped to and correctly identified
the next layer — but no pass ever owned the vertical slice end-to-end, and
the "needs live verification" gate has acted as a permanent stop sign rather
than a checkpoint.

## 6. Current working-tree state (uncommitted, this clone)

Two **separate concerns** are currently mixed in one dirty tree — they must
land as two PRs:

**A. Agent-pane new-tab fix (complete, verified, unrelated to this doc's topic):**
- `frontend/app/view/agent/agent-view.tsx` — tab strip moved from
  `AgentPresentationView` up to `AgentViewWrapper`; tab list now driven by the
  pane's own `blockStack` (mirroring `term.tsx`) merged with cross-pane fork
  tabs. Fixes "+ makes the whole pane vanish into AgentPicker with no way back."
- Typecheck clean; 1052/1053 vitest (1 unrelated flake, passes in isolation).
- **Not yet live-verified** (the dev instance died at session teardown before
  the user could click it). Needs: branch, live check, PR.

**B. attachedTask wiring (STARTED, incomplete):**

Done in-tree:
- `frontend/app/view/agent/activity/attached-task.ts` (new) —
  `hasLiveAttachedActivity(nodes, allSubagents, blockId, now)`: pure helper
  mirroring ActivityDock's shell+subagent+tool aggregate, per spec §4.
- `agent-pane-state-store.ts` — `attachedTask?` projection setter added to
  `AgentPaneProjections` + projected in `dispatch()`.
- `frontend/app/view/agent/state.ts` — `attachedTaskAtom` added to
  `AgentAtoms` + initialized in `createAgentAtoms`.

**Not yet done** (the actual remaining work, in order):
1. `agent-view.tsx` registration: pass `attachedTask: a.attachedTaskAtom[1]` in
   the `projections` object (~line 460).
2. The dispatch call site: an effect in `agent-view.tsx` (or a small hook)
   watching `hasLiveAttachedActivity(...)` over `documentAtom` +
   `allSubagentsAtom`, dispatching `AttachedTaskObserved`/`AttachedTaskCleared`
   on the 0→1/1→0 edges. Needs the same one-shot-timer scheduling discipline as
   the dock's `toolPromotionNonce` (a running Bash call crosses the 30s
   threshold on wall-clock, not on a document event).
3. `AgentWorkingRow`/`AgentFooter` rendering: when `!workingFromPhase(turnPhase)`
   && `attachedTask != null`, render the calm third state ("Running · Ns")
   instead of nothing; when the turn IS working, attachedTask adds nothing.
4. Typecheck + unit tests + **live verification** (a promoted `sleep 60`
   foreground call, and a real `task dev`).

## 7. What "actually fixed" requires beyond this tree (the full ladder)

In dependency order — each rung is independently shippable:

1. **Wire the axis end-to-end** (§6.B above — frontend only, no backend). Gets the honest third status for everything the dock can already see.
2. **Thread `run_in_background` into `BashParams` + a dock adapter for live harness background tasks** (step 2 of Agent3's plan). This is what makes `task dev`-style tasks *visible* — today they're invisible no matter what the axis does. Requires the translator/stream-parser to preserve the flag from the tool_use input JSON (it's already in the transcript; it's just dropped at parse time).
3. **Teach bashwrap the difference** — once a task is *declared* long-running (backgrounded), its idle-timeout should not apply (or apply a much longer/none policy). Kills the need for the heartbeat hack entirely, which in turn un-pins the Working indicator without touching the watchdog.
4. **Survive session teardown** — a declared-long-running background task should be reparented/adopted (launcher Job Object or srv-owned) rather than dying with the agent session. Today's `byoiy4c2v` death is this gap. (Design needed; interacts with isolation invariants I1–I6.)
5. **Windows `started_at_ms`** via `NtQueryInformationProcess` — enables the last-resort process-age heuristic and correct elapsed display in Swarm. Independent of 1–4.

Rungs 1+2 together deliver the user-visible promise ("task dev runs inside a
dock and the agent returns"); rung 3 removes the footgun; rung 4 makes it
durable; rung 5 is polish.

## 8. Immediate next actions for this tree

1. Stash/branch concern A (new-tab fix) → live-verify → PR.
2. Finish concern B items 1–4 (§6) on its own branch → PR referencing spec §6.
3. File rungs 2–4 as separate tracked issues so they stop being rediscovered
   from scratch by each successive agent session — that rediscovery loop is
   itself the meta-failure this doc exists to break.
