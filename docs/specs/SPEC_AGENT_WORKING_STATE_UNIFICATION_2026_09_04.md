# Spec: unify the Working/Worked label with the long-running-process axis, and close the two live desync bugs

**Status:** active — Phase 1 (bug 1.1) and a Phase-0-adjacent telemetry
extension implemented and merged; Phase 2 (bug 1.2) deliberately NOT
re-attempted this pass — see the "What Phase 0 turned out to already be"
and "Phase 2 status" notes below. Phases 3/4 not started.
**Author:** Agent5
**Verified against:** `main` @ `445f879` (2026-09-04).

## What Phase 0 turned out to already be (correction, post-implementation)

Before implementing, I re-verified §2's Phase 0 against the live code and
found it was **already shipped** — by `SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md`
(2026-08-18, after the 07-27 telemetry audit this plan's Phase 0 was based
on), not by this pass. All three of §3.1-3.3's original design items are
live on `main` today: the `[wave-turn]` transition line and watchdog-
reasoning line (`agent-pane-state-store.ts`'s `dispatch()`), and the
backend `[health] turn_active flip` line (`health.rs`). There's also a
`muxlog phases` recipe (same 08-18 spec) that merges both sources
chronologically for exactly the "why did my pane say Working" question
this plan's Phase 0 was written to unblock. My original research (an
Explore-style grep for "Working"/"Worked"/"Queued message") missed the
08-18 spec because its filename doesn't contain any of those strings —
worth remembering next time: a title-string grep is not a substitute for
checking `docs/specs/INDEX.md` chronologically for the exact area.

**What I actually shipped instead**, once I found the real gap: the
attached-task axis (`AttachedTaskObserved`/`Cleared`/`RegistryAttachedTaskObserved`/
`Cleared`) had **zero** logging of its own — every other axis this file
logs got a `[wave-turn]` line, this one never did. Added it, same pattern,
same file. This directly extends `muxlog phases`' usefulness for
diagnosing bug 1.2 specifically (see below) without touching any rendering
code.

## Phase 2 status: investigated, NOT re-attempted this pass

I also re-verified the assumption behind §1.2/Phase 2 before touching
anything, and found the actual code is further along than the specs I'd
cited: `tool-adapter.ts`'s `isAcceptedBackgroundLaunch` + `<task-notification>`
completion tracking (issues #2490/#2518/#2520) mean a genuinely-backgrounded
Bash call (`run_in_background: true`, harness actually detaches it) gets a
dock row **immediately**, no 30s wait, and stays `running` until its real
completion notification lands — and the attached-task axis picks this up
"by construction" per its own doc comment. `SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md`'s
"still open" note on `run_in_background` threading is itself stale — the
work it describes as unbuilt has since shipped. (This is the SECOND time in
this investigation a spec's own status line under-reported what's actually
on `main` — worth treating any "still open"/"unbuilt" claim in this area as
needing a fresh grep, not a citation.)

Given that, PR #2495's original reasoning (the dock row is a sufficient,
non-redundant indicator, confirmed via live use) looks, if anything, better
supported now than when it was written — the data path it relies on is more
complete than either of us assumed. I did not find a live, reproducible gap
in this session to justify touching `AgentFooter`/the dock a second time,
and the project's own standing rule ("stop and ask before anything needing
live-human UI testing") plus the direct precedent of #2489 → #2495 argues
against a third guess without evidence. I also found a `swarm-view.tsx` /
`swarm-child-count.ts` fleet-level view (with its own long-running-tool-call
counting) I hadn't accounted for in the original research — a plausible
place bug 1.2 is actually being perceived (a coarser multi-agent overview,
not the single-pane footer) but not one I chased down this pass.

**What this means for Phase 2 going forward:** the new attached-task
`[wave-turn]` logging (above) is specifically there so the NEXT time this
is reported, `muxlog phases` can directly show whether the axis tracked the
attached task correctly through the window in question, instead of
guessing again. Do not restore the #2489 footer text, and do not assume the
Swarm view is or isn't the real gap, without that evidence in hand first.

## User's request (verbatim, for traceability)

> we need to refine how the "Working..." indicator works .. whats the code look like? It is very janky. can you write a refactor/cleanup plan?
>
> one issue that is obvious is if the state is in "Worked" and not "Working" you should never see the "Queued message" but we still see it
>
> there are many times when the agent is clearly working, but says "Worked" .. this has to do with long running processes. I think we need a wholistic approach that bring the state reducer and the long-running system together clearnly.

## Prior art this builds on (not re-derived)

- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` — the original Agent1 incident. Its own lesson: *"turn-phase and attached-process-liveness are orthogonal — a live process is not the same signal as a live turn."*
- `docs/specs/SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md` — the watchdog force-recovery model. Mostly shipped; one item (tame `bumpEvent`/`StreamFlushObserved` re-promotion) not shipped.
- `docs/specs/SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` — named five-plus independent "is this still going" trackers and recommended unification. This spec is effectively that consolidation's follow-through for the two trackers the user is pointing at (`TurnPhase` and the attached-task axis).
- `docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md` — added `attachedTask`/`registryAttachedTaskSince` as a sibling field to `TurnPhase`, deliberately not folded into it. Reducer slice + dispatch shipped in PR #2489.
- **PR #2495 (2026-08-10)** — reverted #2489's `AgentFooter` render of that axis. Rationale, quoted directly from the PR body: *"the ActivityDock's running row — same data source, pinned right above the composer — already IS the indicator for a live attached task, with strictly more affordance (full command, live log, stop button). The footer copy was redundant by construction."* **This is a load-bearing fact for this plan** — the obvious-looking fix ("put the running-task text back in the footer") was already tried and deliberately rejected on live-UX grounds, not abandoned by neglect. Any new plan has to either explain why that reasoning no longer holds or design something that isn't just the same footer text again.
- `docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` — diagnosed `Done.completed → Streaming` re-promotion as a "design tension," proposed a `settled: boolean` grace flag. Never implemented.
- `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` — catalogued **9 distinct ways "Working" can read true/false incorrectly**, and designed 4 logging-only telemetry additions to make them diagnosable via `muxlog` instead of by reasoning/guessing. **None of the 4 were ever implemented.**

## 1. What's actually broken (root-caused, not guessed)

### 1.1 Bug — "Queued message" visible while the pane reads "Worked"

`PendingMessagesPanel.tsx:41-53` gates its own visibility purely on `state.pending` (filtered by `enqueuedWhileBusy`) — it does not read `turnPhase`/`isWorking` at all. A pending entry is cleared only by an **async** ack: `PendingMessageAccepted` (backend `agent-message-accepted` WPS event), `PendingMessageRejected`, or `PendingMessageExpired`. Separately, `agent-view.tsx:1921-1936` watches `turnPhase.kind` and, only once it reaches `Idle`/`Done` (i.e. the moment the label flips to "Worked"), fires `commands.flushHeldMessages()` — a fire-and-forget RPC. The panel doesn't actually clear until that RPC's ack lands, minutes or milliseconds later depending on backend latency.

**Root cause:** the reducer has no single command that says "a turn just ended AND something is queued — resolve both together." Two independently-timed systems (a synchronous state flip, an asynchronous ack) are each individually correct, but nothing couples them, so the panel and the label can disagree for the entire ack round-trip. This is not a rare race; it's the designed shape of every turn-end-with-something-queued flow.

### 1.2 Bug — "Worked" shown while a long-running process is clearly still active

The data layer for this is **already built correctly** — `attachedTask` (transcript-derived) and `registryAttachedTaskSince` (DB-durable) are both populated and available on `state` today. The gap is not missing data; it's that **the one UI surface that would announce it was tried once (PR #2489) and removed nine hours later (PR #2495)** because the ActivityDock's own running-row was judged sufficient and the footer copy was "redundant by construction."

Two possibilities follow from that history, and this plan needs to resolve which one is true before prescribing a fix:

- **(a)** The dock row genuinely is sufficient in the cases #2495's author tested, but the user is now hitting cases the dock row doesn't cover — e.g. it can be scrolled out of view, or it only reflects `PinnedActivity` (promoted ≥30s Bash calls, MCP Shell tasks) and doesn't cover other shapes of "clearly still working" (an orphaned turn with no `TurnEnd` ever arriving, a stalled rate-limit retry loop, a stray `StreamFlushObserved` silently re-arming `Streaming` and then dropping back to `Done` again, etc. — see the 9-path catalog below).
- **(b)** The dock row itself has degraded or was never fully wired for every activity shape.

**Post-implementation update:** (b) is ruled out — re-verified directly against `tool-adapter.ts` before touching any code. `run_in_background` threading (`BashParams.run_in_background`, `isAcceptedBackgroundLaunch`, `<task-notification>` completion tracking) is fully built (issues #2490/#2518/#2520) and the attached-task axis picks up a genuinely-detached Bash call immediately, no 30s wait, "by construction" per the adapter's own doc comment. The `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` "still open" note citing this as unbuilt is itself stale. (a) remains open and unresolved — see the "Phase 2 status" note at the top of this document for what was and wasn't chased down this pass, including an unaccounted-for `swarm-view.tsx` fleet-level view.

Telemetry audit report (07-27) independently catalogued **9 distinct paths** to a false "Working"/"Worked" read, only one of which (dock-promotion delay before 30s) is about the attached-task axis at all. The others are watchdog/orphaned-turn/rate-limit/re-promotion paths that have nothing to do with the dock row and would not be fixed by touching it. **Without knowing which path the user is actually hitting, prescribing "wire the dock more" vs. "fix the watchdog" vs. "fix the re-promotion rule" is a guess** — and the previous team already spent a footer round-trip on a guess that turned out redundant.

### 1.3 The structural theme underneath both bugs

Both bugs are instances of the same shape: **two independently-updated signals that are supposed to describe one coherent story, with no shared mechanism forcing them to resolve together** — `turnPhase` vs. `pending` (1.1), `turnPhase` vs. `attachedTask`/dock visibility (1.2). The `pendingCompactionPing` section of the reducer (~200 lines, 11 documented rounds of review findings across PRs #2378/#2928) is the same failure shape recurring a third time, just patched far more times because it's been hit far more. `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` already named this pattern generally; this plan is where it gets addressed for the two trackers the user is actually looking at.

## 2. Plan

Ordered by risk/payoff — each phase is independently shippable and the later phases depend on evidence the earlier ones produce, not just on code.

### Phase 0 — Ship the telemetry that already has a design (near-zero risk)

**DONE — turned out to be two thirds already shipped by someone else, the remaining third landed this pass.** `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` §3.1–3.3 designed three logging-only additions; all three were actually already live on `main` via `SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md` (see the correction note at the top of this document) — the `[wave-turn]` transition/watchdog-reasoning lines and the backend `[health] turn_active flip` line, plus a `muxlog phases` recipe merging both. What this pass actually added: a `[wave-turn]` line for the attached-task axis (`AttachedTaskObserved`/`Cleared`/registry variants), which had none — the one gap left in "every axis this file tracks gets a diagnostic line." Implemented in `agent-pane-state-store.ts`, covered by 4 new tests in `agent-pane-state-store.test.ts`.

### Phase 1 — Fix bug 1.1 (Queued message under Worked) at the reducer level

**DONE — one round of review correction.** First version marked `flushing: true` on `TurnEnd`, unconditionally, for every still-`enqueuedWhileBusy` `pending` entry. Codex P2 on PR #2970 caught a real gap: `flushHeldMessages` (`useAgentCommands.ts`) can bail without draining `heldQueue` at all when a controller refresh is still deferred (`~1433-1455`) — in that case `TurnEnd` had already fired, so the entry would read "Sending — reaching the agent any moment" while delivery hadn't actually started, for however long the deferred refresh took, and the recall/edit guidance would be hidden the whole time even though recall was still meaningful.

Fixed by moving the flag to a new, dedicated command — `PendingMessageFlushStarted` — dispatched from `flushHeldMessages` right before the actual `deliverToBackend` call (i.e. the moment delivery genuinely begins, past every reject-check in that function). `TurnEnd` no longer touches `pending` at all. In the common case (no deferred refresh) delivery starts essentially immediately after `TurnEnd`, so the visible "Queued" copy window collapses to the same near-zero gap the original bug's own visible manifestation had — just triggered at the true right moment instead of guessed from `TurnEnd`'s timing. On an eventual delivery failure, `deliverToBackend`'s own catch block dispatches `PendingMessageRejected`, which removes the entry outright regardless of `flushing` — no stale "Sending" state possible. Implemented in `reducer.ts`/`types.ts`/`useAgentCommands.ts`/`state.ts`/`PendingMessagesPanel.tsx`, covered by 4 reducer tests (rewritten for the corrected design) + 5 component tests + the full pre-existing 53-test `useAgentCommands.test.ts` suite (no regressions, confirming the one-line additive dispatch doesn't disturb its dense auth/refresh-ordering assertions).

### Phase 2 — Diagnose, then fix, bug 1.2 (informed by Phase 0's telemetry)

**Investigated this pass, deliberately NOT re-attempted — see the "Phase 2 status" note at the top of this document.** Do not default to "restore the #2489 footer text" without live evidence — that was already tried and rejected once (dock row judged sufficient, and re-verification this pass found the data path behind it is now MORE complete than assumed, not less). The new attached-task telemetry from Phase 0 is what should inform this phase next time it's reported — pull `muxlog phases` for the affected pane and see whether the axis tracked the attached task correctly through the window in question, before picking a fix.

### Phase 3 — Ship the already-designed `settled` grace flag

`REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` §1.3 already designed this: a short (400–600ms) grace window after `Done` during which a stray `StreamFlushObserved`/`bumpEvent` re-promotion doesn't silently flip the label back to "Working" without at least a beat of "Worked" being visibly true first. Directly relevant to the user's "clearly working but says Worked" complaint in the *opposite* direction (label instability, not staleness) and has been sitting designed-but-unshipped for over a month.

### Phase 4 — Structural consolidation (the "wholistic" ask)

Once 0–3 are live and stable:

- **One canonical "is something long-running attached" selector**, not two fields independently combined by whichever component happens to need it. Today `agent-view.tsx` computes `Math.min` of `attachedTask`/`registryAttachedTaskSince` inline (types.ts:370-386 documents this is deliberate — the Codex P1 finding on PR #2685 requires the two data sources stay independently owned). Keep that data-layer independence, but add one shared pure selector (e.g. `earliestAttachedTaskStart(state): number | null`) so every consumer reads one function instead of re-deriving the combination logic itself.
- **One documented timeout-policy table** collecting the scattered magic constants (`STUCK_THRESHOLD_MS`, `LIVENESS_RECOVERY_MS`, `INTERRUPT_TIMEOUT_MS`, `SUBMIT_TIMEOUT_MS`, `COMPACTION_HEURISTIC_SUPPRESS_MS`, `TOOL_PROMOTION_MS`, bashwrap's 600s idle-kill) — not necessarily unified into one timer, but auditable from one place instead of tribal knowledge scattered across a dozen specs.
- **Retire or fold `isBackendTurnActive`/`wasTurnActive`** (`useAgentCommands.ts`) — a third, narrower "is a turn active" tracker alongside `turnPhase` and the backend's own health monitor, used only to gate one destructive-restart RPC. Worth checking whether `ReconcileTurnActive`'s existing backend-authoritative reconciliation already makes this redundant.

This phase is explicitly lower-urgency cleanup, not bug-fixing — sequence it after the two user-reported bugs are actually closed and verified live.

## 3. What this plan deliberately does not do

- Does not re-attempt "put the running-task text back in the AgentFooter" without first getting telemetry evidence — that specific fix has a documented rejection on live-UX grounds (§0 above) and repeating it without new evidence would be reproducing a decision the team already reversed.
- Does not touch `run_in_background` Bash-call threading or bashwrap's 600s idle-kill exemption — both remain their own, already-scoped, unbuilt follow-ups (`REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` steps 2/5, `REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md`) — Phase 2's diagnosis may point at these as the actual root cause, in which case they get pulled forward, but this plan does not assume that in advance.
- Does not propose collapsing `attachedTask` and `registryAttachedTaskSince` into one field — the Codex P1 finding on PR #2685 that keeps them independently owned is treated as a real constraint, not an oversight to fix.
- Does not touch the `pendingCompactionPing`/compaction-detection section of the reducer — flagged in §1.3 as the same failure shape recurring, but compaction has its own extensive, separately-tracked history (PRs #2378/#2928, `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`) and is out of scope here.

## 4. Key files

| Concern | File | Line(s) |
|---|---|---|
| `TurnPhase` state machine | `frontend/app/store/agent-pane-state/types.ts` | 195-233, 457-471 |
| Reducer transitions (`StreamFlushObserved`, `StreamWatchdogTick`, `ReconcileTurnActive`, `TurnStart`/`TurnEnd`, `PendingMessage*`, `bumpEvent`) | `frontend/app/store/agent-pane-state/reducer.ts` | 101-146, 223-395, 397-507, 523-710, 959-1129, 1515-1562 |
| Working/Worked label source | `frontend/app/view/agent/agent-view.tsx` | 1620-1621, 2443 |
| `AgentWorkingRow` render | `frontend/app/view/agent/components/AgentFooter.tsx` | 25-31, 57-121 |
| Ungated "Queued message" render (bug 1.1) | `frontend/app/view/agent/components/PendingMessagesPanel.tsx` | 41-53 |
| Turn-end flush trigger (async race, bug 1.1) | `frontend/app/view/agent/agent-view.tsx` | 1921-1936 |
| Pending-message ack consumer | `frontend/app/view/agent/hooks/usePendingMessageAcceptance.ts` | 63-129 |
| `attachedTask`/`registryAttachedTaskSince` axis | `frontend/app/store/agent-pane-state/types.ts` | 123-125, 359, 368-386 |
| Attached-task dispatch site (`Math.min` combination) | `frontend/app/view/agent/agent-view.tsx` | 1641-1683 |
| ActivityDock running row (current sole indicator for bug 1.2) | `frontend/app/view/agent/activity/tool-adapter.ts` | full |
| Durable background-task registry | `agentmux-srv/src/backend/storage/background_tasks.rs` | full |
| `dispatch()` — where Phase 0's telemetry lands | `frontend/app/store/agent-pane-state-store.ts` | 169-249 |
| `[wave-title]` precedent to mirror | `frontend/app-init.ts` | 857-867 |
| `settled` grace-flag design (Phase 3) | `docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` | §1.3 |
| 9-path false-Working catalog | `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` | §1 |
| PR #2489 (attached-task axis + footer render, shipped) | GitHub | — |
| PR #2495 (footer render reverted, same day) | GitHub | — |
| `PendingMessage.flushing` (Phase 1) | `frontend/app/view/agent/state.ts` | 157-188 |
| `PendingMessageFlushStarted` dispatch site (Phase 1, corrected design) | `frontend/app/view/agent/hooks/useAgentCommands.ts` | `flushHeldMessages`, just before `deliverToBackend` |
| `[wave-turn]` attached-task logging (Phase 0 extension) | `frontend/app/store/agent-pane-state-store.ts` | ~403-430 |
| `muxlog phases` recipe (already-shipped Phase 0) | `docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md` | §1 |
| Backgrounded-Bash dock wiring (already-shipped, ruling out §1.2(b)) | `frontend/app/view/agent/activity/tool-adapter.ts` | `isAcceptedBackgroundLaunch`, `parseTaskNotification`, `toolActivities` |
| Fleet-level Swarm view (unaccounted-for §1.2(a) lead, not chased down) | `frontend/app/view/swarm/swarm-view.tsx`, `swarm-child-count.ts` | `agentChildRowCount` |
