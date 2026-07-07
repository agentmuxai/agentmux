# Report: agent/swarm pane loading, ambient-call flood, and stale status

**Status:** Investigation complete — no code changed. Written to inform an
architecture decision before implementation.
**Author:** AgentX
**Date:** 2026-07-07
**Triggered by:** user report — "loading an agent started a flood of subagent
calls," "loading took much longer than [before the brain-spinner overlay was
built]," "there was no pulsating brain," "all the subagents are working even
though they should have been done long ago," "a big mess regarding opening
old agents and handling their state gracefully."

## tl;dr

Four separate, code-confirmed bugs, one of which is architecturally
significant enough to justify the user's "whole architecture rethink"
framing:

1. **(Architectural)** The frontend's only turn-status state machine
   (`TurnPhase`) is never seeded from the backend's actual process/session
   truth at pane mount — every reopened pane starts at a hardcoded `Idle`
   and self-corrects only by accident, when new live output happens to
   arrive. This is the root cause of the Idle-then-flips-to-Working glitch
   and feeds directly into finding 4.
2. **(Long-standing, unrelated to recent PRs)** Subagent completion detection
   in the backend matches a placeholder string that real Claude Code output
   almost never produces — subagent rows show "working" forever.
3. **(Real gap, recently introduced)** The ambient-model-call gateway has no
   cross-pane concurrency limit on its two user-triggered RPCs, unlike its
   own periodic sweep — many panes finishing turns around the same time can
   spawn unbounded concurrent Haiku subprocesses. Most likely explanation for
   "flood of calls" + slowdown.
4. **(Scope gap in #1992, not a regression)** Subagent panes render through a
   completely different component (`SubagentView`) that never got the
   brain-spinner overlay — explaining "there was no pulsating brain" for
   swarm-opened panes specifically.

None of these four are the same bug. All four are real. #1 is the one worth
treating as an architecture decision; #2–#4 are concrete, independently
fixable bugs.

---

## Finding 1 — TurnPhase is never reconciled with backend truth at mount

**This is the single most significant finding** and plausibly explains most
of what "a big mess... handling state gracefully" is pointing at.

### The gap, precisely

- `registerPane()` seeds every (re)mounted pane's `turnPhase` to
  `{kind:"Idle"}` unconditionally
  (`frontend/app/store/agent-pane-state/types.ts:287`, via
  `agent-pane-state-store.ts:154`) — regardless of whether the underlying
  agent process is mid-turn, idle, or something else.
- `agent-view.tsx`'s `onMount` **does** call a real reconciliation-shaped
  sequence — `startLaunchFlow()` →
  `launch-flow.ts:292-318`'s Phase 3: `ControllerResyncCommand` (hits
  `blockcontroller::resync_controller` in the backend) then
  `BlockService.GetControllerStatus`, which returns the real
  `shellprocstatus` (`init`/`running`/`done`).
- **But the result is only ever used for a log line.** There is no branch
  that dispatches anything into the `TurnPhase` reducer based on this
  status. `useControllerStatusEvents.ts` does the same thing for the live
  stream of subsequent `ControllerStatus` events — also log-only, never
  reducer-dispatched.
- So `turnPhase` only ever leaves `Idle` via **new live stream events**
  arriving after mount (`useAgentStream.ts` → `StreamFlushObserved`, which
  the reducer explicitly promotes `Idle`→`Streaming`,
  `reducer.ts:243-253` — a comment there already acknowledges this is
  filling an accidental gap: *"After a stream drop + resubscribe... the
  phase lands in Idle/Disconnected, and without this promotion the 'in
  progress' indicator stays OFF while output streams in"*).

### Why this matters more for persistent-mode agents (the common case)

For `PersistentSubprocessController` (Claude's default), `shellprocstatus`
is set to `running` once at process spawn and only flips to `done` on
process exit/crash (`persistent.rs:651,853,917`) — it never distinguishes
"mid-turn" from "idle between turns," since the process doesn't exit between
turns. This is already documented in-repo
(`useAgentStream.ts:480-481`). So even a full wire-up of "seed `turnPhase`
from `shellprocstatus` at mount" would need real turn-level truth (e.g. last
NDJSON event type / whether a session is still "open"), not just
process-alive status — this is not a one-line fix, it's a design question:
**what backend signal actually answers "is a turn currently in flight," and
how does a reopened pane query it before first paint?**

### The user-visible symptom sequence

1. Pane mounts → `turnPhase = Idle` → loading overlay fades once history
   *painting* settles (via `scheduleOnSettle`, unrelated to `turnPhase`) —
   **the pane now visually looks fully idle/settled.**
2. If the agent is actually mid-turn but has been quiet for a while (Claude
   commonly pauses seconds-to-tens-of-seconds between chunks/tool calls —
   the reducer's own `STUCK_THRESHOLD_MS` (45s) and `LIVENESS_RECOVERY_MS`
   (180s) constants are calibrated around exactly this kind of quiet
   window), the UI can sit in a confidently-wrong "done" state for that
   entire span.
3. Only when new output actually arrives does `StreamFlushObserved` correct
   it — from the user's perspective, a pane that "looked finished" suddenly
   starts "working" again with no explanation.

### Every liveness signal found (none reconciled against each other)

| # | Signal | Scope | Populated by | Reconciled? |
|---|---|---|---|---|
| 1 | `TurnPhase` reducer | root pane, turn-level | live stream events only | Sole source for composer UI; never seeded at mount |
| 2 | `SubagentStatus` | subagent, turn-level | JSONL text match (buggy — Finding 2) | No |
| 3 | `shellprocstatus`/`ControllerStatus` | block, process-level | `resync_controller`/`get_block_controller_status` | Fetched at mount, discarded (log-only) |
| 4 | `SwarmViewModel.agentStatusesAtom` | block, process-level | mirrors #3 in the frontend | Computed, then discarded by #5 |
| 5 | `phaseToDisplayStatus` (rendered Swarm chip) | block | prefers #1, else hardcoded `"unknown"` | Deliberately ignores #4 (see below) |
| 6 | `agentActivity` busy registry | global (taskbar/dock) | aggregates #1 across *mounted* panes only | Blind to unmounted/background agents |
| 7 | `useProcessCount` | block | OS subprocess add/exit events | Different concept (helper-process count), coincidentally read as activity |

Item 5 deserves a specific callout: `swarm-view.tsx:163-182`'s
`phaseToDisplayStatus` receives the real backend-verified status (#4) as a
parameter but the parameter is underscore-prefixed and **never read** — a
deliberate choice from PR #1947 to stop conflating "not tracked in this
renderer" with "confirmed idle," which fixed one bug but means a perfectly
good, already-fetched signal is thrown away in exactly the "reopening a
background agent" scenario this report is about.

### What a fix would need to decide (not resolved here — needs a design call)

1. What's the actual backend-computable "turn in flight" signal for a
   persistent-mode agent? (Last NDJSON event type + whether the session
   considers itself "open"? A live flag the controller already tracks
   internally? Something new?)
2. Where does reconciliation happen — a one-shot correction at mount
   (cheapest, still leaves the same gap for backgrounded/unmounted agents,
   item 6/7 above), or a standing subscription so Swarm-tracked-but-unmounted
   agents also stay accurate?
3. Should `phaseToDisplayStatus` go back to consuming the real backend
   status (#4) instead of `"unknown"`, now with a `TurnPhase`-level fix
   making #1 trustworthy at mount time too — i.e. can items 1 and 5 be fixed
   together so neither needs to guess?

---

## Finding 2 — Subagent completion detection is broken (long-standing, unrelated to recent PRs)

**`agentmux-srv/src/backend/subagent_watcher.rs:575-581`:**
```rust
if let Some(last) = new_events.last() {
    if matches!(&last.event_type, SubagentEventType::Text { content } if content == "Subagent completed") {
        completed = true;
        state.info.status = SubagentStatus::Completed;
    }
}
```
This is the **only** place `SubagentStatus::Completed` is ever set. The
literal string `"Subagent completed"` is produced by `parse_event_type`'s
`"result"` branch **only as a fallback** when a JSONL `"result"` event has no
`result`/`content` field:
```rust
"result" => {
    let content = value.get("result").or_else(|| value.get("content"))
        .map(|v| { ... })
        .unwrap_or_else(|| "Subagent completed".to_string());
    Some(SubagentEventType::Text { content })
}
```
Real Claude Code result events (verified against
`docs/specs/SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md:28`, a live-run-
confirmed shape) carry a populated `result` field — so the fallback almost
never triggers, `completed` stays `false`, and `subagent:completed` is never
broadcast. `swarm-view.tsx:273` renders straight from `SubagentStatus`, so
the row shows **"working" forever**.

Traced via `git log -p --follow` to the original introduction
(`7c3fc58b`, PR #146) — **unchanged since**, through the Swarm two-level
redesign (`2ff8d28d`, #1597) and workflow tracking (`dd524cb7`, #1976). Not
a regression from any PR in the recent list (#1722/#1752/#1793/#1809/#1947/
#1987 all touch the *separate* TurnPhase mechanism for root panes, confirmed
via diff review of #1987 — zero overlap with `subagent_watcher.rs` or
anything under `frontend/app/view/swarm/` / `subagent/`).

**Fix shape:** key completion off `parse_event_type` recognizing a
`"result"`-typed line directly (e.g. return a tag/bool alongside
`SubagentEventType`, or check `value.get("type") == Some("result")` in
`process_jsonl_change` before the text is derived) — not off the derived
`Text.content`, which loses the discriminant the fallback exists to paper
over.

---

## Finding 3 — Ambient-gateway pull RPCs have no cross-pane concurrency cap

`agentmux-srv/src/ambient/mod.rs`'s `admit(key, generation)` is a real,
correctly-implemented per-key single-flight gate (fencing-token staleness
rejection, supersedes in-flight calls) — but "per key" means per
`(entity_id, purpose)`, i.e. per pane. There is **no limiter across
different panes.**

- The periodic pushed-summary sweep (`activity_watcher.rs`) is properly
  capped: `MAX_CONCURRENT_SUMMARIES = 2`
  (confirmed: `activity_watcher.rs:49`), semaphore-gated, skip-if-
  output-unchanged.
- The two user-turn-triggered pull RPCs
  (`session:activity_summary` / `session:next_prompt_suggestion`,
  `agentmux-srv/src/server/app_api/session.rs:174-324`, wired from
  `useAgentActivitySummary.ts`/`useNextPromptSuggestion.ts` on a real
  `Done`-transition effect — not a mount-time fire, confirmed) have **no
  equivalent cap.** If N different panes finish a turn around the same
  moment, each independently spawns its own Haiku CLI subprocess,
  unbounded.

This is the most plausible mechanism behind "flood of subagent calls" and
the associated slowdown — not literally "opening one agent floods calls,"
but "many agents/subagents finishing turns close together floods calls,"
which reads the same from a user's chair, especially right after a Swarm
reopen where several subagents may complete in a tight window.

**Fix shape:** route the two pull RPCs through the same (or an equivalently
sized) semaphore/budget the sweep loop already uses, rather than leaving
them uncapped.

---

## Finding 4 — Subagent panes never got the brain-spinner overlay (scope gap, not a regression)

`view: "subagent"` renders `SubagentView`
(`frontend/app/view/subagent/subagent-view.tsx`) — a wholly separate,
lightweight component from `AgentPresentationView`
(`frontend/app/view/agent/agent-view.tsx`), confirmed via grep: `BrainSpinner`
and `scheduleOnSettle` appear only in `agent-view.tsx` (and generically in
`block.tsx`'s per-view-type fallback from #1992).

`block.tsx`'s generic `<BrainSpinner/>` fallback (added in #1992) *does*
cover every view type's "stage-one blank" (before the block object +
viewModel resolve) — but for `SubagentViewModel` that resolves near-
instantly, long before `SubagentViewModel.loadHistory()`'s two RPCs
(`subagent.GetHistory` + `subagent.ListActive`, `subagent-model.ts:147-176`)
actually return. `SubagentView` shows only a static `"Loading subagent
activity..."` text during that window
(`subagent-view.tsx:83-85`) — no spinner, no settle-detector.

This is exactly the scenario #1992's own report doc named as motivating
(swarm sub-agent click) but didn't fully cover — the report recommended
wiring at `block.tsx` specifically to be universal, which happened for
stage-one, but stage-two (the actual perceptible loading window) was only
wired into `agent-view.tsx`. **Confirmed gap in #1992's coverage, not
evidence #1992 broke anything for its actual target.**

Secondary, smaller finding in the same file: `subagent-model.ts:166` fetches
`subagent.ListActive` (**every** active subagent across **every** session)
on every subagent pane mount, in addition to its own `GetHistory` — wasteful
fan-out that compounds when several subagent panes are open at once. Worth
trimming to a targeted lookup, or dropping it since `subagent:spawned`/
`subagent:completed` event handlers already populate `info` reactively.

---

## What's NOT the cause (ruled out, so they don't get re-investigated)

- **#1987** ("unify failure state into the pane reducer") — diff-reviewed,
  touches only root-agent TurnPhase/failure files. Zero overlap with the
  subagent watcher, the Swarm/subagent view files, or the ambient gateway.
- **Tab switching** is not a remount — `workspace.tsx` keeps every tab
  mounted (`display:none` when inactive), confirmed against
  `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`. `useHistoryPagination`,
  `startLaunchFlow`, `registerPane` etc. fire once per pane lifetime, not
  per tab revisit. (Side note: two recently-written hooks'
  comments — `useBlockActivity.ts:76-84`, `useAgentActivitySummary.ts:17-31`
  — incorrectly assert tab-switch causes a remount and defensively code
  around a scenario that doesn't happen. Harmless, but a small symptom of
  the same underlying issue: the pane lifecycle isn't uniformly understood
  across the code that's been layered onto it.)
- **Mounting an agent pane does not, by itself, trigger an ambient call** —
  `registerPane()` resets `turnPhase` to `Idle` on every mount, and the
  ambient hooks fire only on a real `Idle`→...→`Done` transition effect, not
  on mount.

## Genuine remount triggers (for scoping any fix)

- Window layout restore on app launch (every persisted block).
- "Replace With…" view swap (`block.tsx:283-293`'s `viewType` effect).
- First Swarm subagent click this session (`subagent-pane-manager.ts:28-87`
  always creates a new split block unless already tracked in the in-memory,
  session-scoped `openPanes` map).
- Reopening any agent not currently present in any open tab's layout
  (`agent_open.rs`'s idempotent-reopen path, `:47-84`).

Each of these re-runs the *entire* launch-flow round trip (CLI detect, auth
check, resync, `GetControllerStatus`) even when the process is provably
already alive (`resync_controller`'s `force=false` guard prevents an actual
relaunch, but the RPC round trips still happen, add latency, and — per
Finding 1 — still don't inform `turnPhase`).

## Suggested priority order

1. **Finding 2** (subagent completion detection) — small, isolated, no
   architectural dependency on anything else. Fix first; it's pure upside.
2. **Finding 3** (ambient concurrency cap) — small, isolated, mirrors an
   existing pattern (`activity_watcher.rs`'s semaphore) already in the
   codebase. Fix second.
3. **Finding 1** (TurnPhase reconciliation) — the real architecture
   decision. Needs the three open questions above answered before writing
   code, not a quick patch. This is the one worth deliberate design time.
4. **Finding 4** (subagent pane loading overlay) — cosmetic/UX polish,
   lowest urgency, natural follow-up once 1–3 are settled since the
   underlying "how long is this actually loading" story will have changed.
