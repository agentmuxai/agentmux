# Report: long-running processes, subagents, and the Swarm pane — consolidated state (2026-07-16)

**Status:** Report — current-state inventory + refinement direction, verified
against `main` @ `7ce6ab6b` (pulled 2026-07-16) and a full sweep of
issues/PRs/discussions. Written to anchor the "refine long-running processes +
subagents + swarm pane" initiative under one consolidated tracker.
**Author:** Agent3
**Related:** #1814 (Area-1 tracker, partly stale), #101 (stale swarm epic),
#1549 (arch refactor, items A6/A9), PR #2177 (Agent1 retro),
`docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md`.

## Why these three are one initiative

All three areas answer the same user question — **"what is this agent actually
doing right now?"** — from three partial vantage points that don't currently
compose:

- The **agent pane** answers it for the model turn (TurnPhase reducer:
  Submitting/Streaming/Interrupting/…) but has no representation of a
  long-running attached process, so a healthy 12-hour `task dev` reads as an
  ambiguous "Waiting…/Working…" (the Agent1 incident, PR #2177's retro).
- The **subagent watcher** answers it for Task-tool/workflow children, with
  its own lifecycle (active/completed/abandoned) reconciled only at pane
  reopen (SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md, Open Q1).
- The **Swarm pane** is the fleet roll-up of both — and inherits every gap:
  it shows turn-precise status (post-#2005) and nested subagents, but has
  zero visibility into background/long-running processes, and its top-level
  agent rows can't collapse, so multi-agent fleets with busy subagent trees
  don't scale visually.

The "flakey status" complaints keep recurring because each fix lands in one
vantage point while the others keep their own, subtly different notion of
busy. Consolidation = one initiative, one tracker, one status model with two
orthogonal axes (turn phase × attached-process liveness) rendered consistently
in all three surfaces.

## Where we are now, per area

### 1. Long-running processes started by agents

**Shipped:** ActivityDock (`frontend/app/view/agent/components/ActivityDock.tsx`)
shows a block-scoped strip of long-running activities — persistent shells
(`shell-adapter.ts`) + subagents (`subagent-adapter.ts`) — per
SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md and
SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24.md. Shell phases/dev-agent trail:
#1338→#1422→#1450→#1816→#1860. Bashwrap wrapper-leak fixed (#2156).

**Gaps (verified in code @ 7ce6ab6b):**
- No per-agent tracking of `run_in_background` Bash tasks anywhere:
  `ShellNode` has no background flag (`activity/types.ts:376-387`); a
  backgrounded dev stack surfaces (if at all) as an undistinguished shell.
- The pane status model has no attached-process axis — `workingFromPhase`
  (`agent-pane-state/types.ts:332`) keys only on turn phase; the Agent1
  incident (12h ambiguous "Waiting…") is the canonical failure. A 120s
  heartbeat in the wrapped command also defeats the 180s liveness watchdog.
- The `cron` activity kind is declared but has no adapter
  (`activity/types.ts:11`).
- Tracker **#1814** is the right consolidation point but stale (last touch
  2026-06-27): its Phase-3b and cron items have since shipped (#1860, #1794);
  still genuinely open there: `dev:running?` probe, Phase-3c pane-close/
  srv-shutdown shell cleanup. Issue #870 (dev:serve TOCTOU) also open.

### 2. Subagents / subagent watcher

**Shipped:** backend watcher (`agentmux-srv/src/backend/subagent_watcher.rs`)
tracks both Task-tool subagents and Workflow runs from `subagents/*.jsonl` +
`subagents/workflows/<run-id>/`, emitting `subagent:spawned|activity|completed`
and `workflow:updated`. Consumed by the Swarm tree (nested per-agent) and the
ActivityDock (#2062). Workflow tool runs tracked (#1976); Haiku activity
summaries (#1978) behind the ambient-gateway caps (#2005 finding 3 et al).
Lifecycle: Abandoned status + reopen-time reconciliation (#2131, #2134).

**Gaps:**
- **No open issue tracks the watcher at all.** The de-facto spec is merged PR
  #2126 ("subagent lifecycle has no reducer, no liveness"); its remaining
  items — notably **real-time** (not reopen-time) Abandoned reconciliation —
  have no tracking issue. The Swarm view carries a client-side display
  backstop for exactly this gap (`subagentDisplayStatus`,
  swarm-view.tsx:196-215).
- Subagent liveness and agent-turn liveness are separate mechanisms solving
  the same problem at two levels (spec PRs #1841 vs #2126) — candidates for
  one model.

### 3. Swarm pane

**Shipped:** two-level agent→subagent tree (#1597), turn-precise per-agent
status — TurnPhase registry when the pane is mounted locally, backend
`turn_active` fallback when not (#1722, #2005; `phaseToDisplayStatus`,
swarm-view.tsx:165-194) — workflow/name grouping into collapsible units
(#2018/#2019, REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md), identity-
preservation so expand state survives status-tick remounts
(`SwarmViewModel._expandedIds`, swarm-model.ts:515-527), live feed
(SPEC_SWARM_LIVE_FEED_UI_2026_07_05.md).

**Gaps:**
- **Top-level agent rows are always-expanded** (`AgentRow`'s
  `.swarm-children`, swarm-view.tsx:277-285, has no toggle) — the first
  refinement this initiative lands (this report ships alongside it).
- No long-running/background-process visibility (Area 1's gap surfaces here
  doubly: neither the per-agent card nor any child row represents an attached
  dev stack).
- Epic **#101** ("Swarm Orchestration", March) describes a filesystem
  task-queue design unrelated to the shipped pane — stale, supersession
  candidate.

## Existing tracking — what consolidation should do

| Item | State | Disposition |
|------|-------|-------------|
| #1814 (long-running commands tracker) | OPEN, stale | Refresh: check off shipped items (#1860, #1794); fold into the consolidated tracker or make it the consolidated tracker |
| #101 (Swarm Orchestration epic) | OPEN, divergent | Close as superseded (shipped Swarm pane ≠ its design) or explicitly re-scope |
| #2126 spec remainder (real-time subagent reconciliation) | Merged PR, untracked | Give it a tracking issue under the consolidated umbrella |
| #1549 items A6/A9 (pane state system, "is busy?" selector) | OPEN | Cross-link; A9 is exactly the two-axis status-model work |
| #870 (dev:serve TOCTOU), #942/#1569 (service supervision overlap) | OPEN | Cross-link under Area 1 |
| PR #2177 (Agent1 retro) | OPEN | The motivating incident writeup for the attached-process axis |

**Recommendation:** one new consolidated tracking issue ("agent activity
visibility: long-running processes, subagents, swarm") with the three areas as
sections, absorbing #1814's still-open items, closing #101 as superseded, and
enumerating the phased work below. Alternative (cheaper): retitle/refresh
#1814 to cover all three areas.

## Refinement direction (phased)

1. **Swarm: collapsible top-level agent rows** — shipped with this report.
   Default-expanded; collapse hides the subagent subtree; state lives on the
   ViewModel (`collapsedAgentIds`, inverse of `_expandedIds` semantics since
   agent rows default open) so it survives status-tick remounts.
2. **Model background tasks as first-class activity** — a `background` flag on
   `ShellNode` (set from the Bash tool's `run_in_background` path), so the
   ActivityDock and Swarm pane can distinguish "attached long-running process"
   from a foreground shell. This is the data prerequisite for everything else.
3. **Two-axis pane status** — surface "Running: <task> (elapsed)" in
   `AgentFooter` when ≥1 live background task exists while the turn is Idle;
   exempt such panes from the liveness watchdog's "quiet = hung" recovery.
   (Fix direction detailed in the Agent1 retro.)
4. **Swarm: show the same attached-process chip** on the per-agent card, so
   the fleet view answers "which agents hold live dev stacks" at a glance.
5. **Real-time subagent reconciliation** (spec #2126's remainder) — retire the
   Swarm view's client-side Abandoned backstop once the backend reconciles
   mid-session.
6. **Tracker hygiene** — file/refresh the consolidated tracker per the table
   above.

## Key files

| File | Role |
|------|------|
| `frontend/app/view/swarm/swarm-view.tsx` / `swarm-model.ts` / `swarm-view.scss` | Swarm tree, status derivation, expand-state model |
| `frontend/app/view/agent/components/ActivityDock.tsx` + `activity/{types,shell-adapter,subagent-adapter,subagent-source}.ts` | Long-running activity strip (shells + subagents; no background flag yet) |
| `frontend/app/store/agent-pane-state/{reducer,types}.ts` | Turn-phase state machine (mature; single-axis) |
| `agentmux-srv/src/backend/subagent_watcher.rs` | Subagent/workflow tracking backend |
| `agentmux-srv/src/backend/blockcontroller/{health,session_stats}.rs` | Backend turn_active + activity ground truth |
