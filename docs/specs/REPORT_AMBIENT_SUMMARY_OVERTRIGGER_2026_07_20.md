# Report: Haiku ambient-summary ghost text populates outside genuine turn completion

**Date:** 2026-07-20
**Author:** AgentX
**Type:** Investigation report + fix, shipped in the same PR.
**Purpose:** The agent pane header's Haiku-generated "ghost text" mini-summary (`term:ambient_summary`) is supposed to populate once per completed agent turn. In practice it populates "at all sorts of times." This report traces the exact over-trigger paths and fixes them by switching the trigger from the frontend's own `TurnPhase.kind === "Done"` to the backend-authoritative `turn_active` edge.

---

## 1. What's supposed to happen (and mostly does, at the UI layer)

`frontend/app/store/activitySummary.ts:19-25` (`readActivitySummary`) resolves `term:ambient_summary` (Haiku-derived) over `term:osc_title` (free, CLI-emitted, no LLM call) for both the agent pane header (`agent-model.ts:108-125`, `viewText()`) and the Swarm tree row (`swarm-model.ts:1012`). Nothing wrong with the read/render side — the bug is entirely in when a fresh value gets written.

A sibling feature, the composer's "predicted next message" ghost text (`term:next_prompt_suggestion`, read in `AgentFooter.tsx`), shares the identical trigger architecture and had the identical bug — both are fixed together here.

## 2. Root cause: `TurnPhase.kind === "Done"` is not 1:1 with "the agent finished responding"

`useAgentActivitySummary.ts` and `useNextPromptSuggestion.ts` both fired their Haiku RPC on any `TurnPhase.kind === "Done"` transition, `createEffect(on(turnPhase, ...))`. `Done` fires on five distinct paths, only one of which is a genuine turn completion:

1. **The dominant cause — a premature per-round `session_end`.** `frontend/app/view/agent/providers/claude-translator.ts:224-233`: *"a non-partial assistant message with no tool_use blocks is always the final text response of a turn. Emit session_end so TurnPhase transitions to Done → idle."* This heuristic assumes the CLI process never exits between turns, so it treats every interim text-only assistant message (which can legitimately precede more tool calls in the same turn) as if it were the real end. `frontend/app/store/agent-pane-state/reducer.ts:213-221` documents the exact failure mode this causes: *"session_end fires after every model API round, so Done.completed can mean 'first round of a multi-round tool-continuation finished'"* — and compensates by re-promoting `Done.completed → Streaming` on the next output flush (`reducer.ts:246`). **Verified directly:** the `TurnEnd` reducer arm (`reducer.ts:537-541`) sets `outcome: "completed"` for this premature case exactly the same as a real completion — there is no way to distinguish a premature per-round `Done` from a genuine one by reading `phase.outcome` alone. A naive fix that only gated on `outcome === "completed"` would not have closed this path.
2. `InterruptTimeoutElapsed → Done.interrupted` (`reducer.ts:752-782`) — a bounded 5s force-transition when a user-requested stop's ack never arrives.
3. `SubmitTimeoutElapsed → Done.errored` (`reducer.ts:784-819`) — a bounded 30s force-transition when the backend never acks a send.
4. `FailureObserved → Done.errored` (`reducer.ts:923-950`) — any classified backend failure (rate limit, etc.) force-ends a working turn.
5. The genuine path: real `session_end` from the CLI's actual turn-boundary event (`useAgentStream.ts:463-467`, `TurnEnd` dispatched from the real event at `useAgentStream.ts:850-854`).

Codex and Gemini's translators (`codex-translator.ts:47-67`, `gemini-translator.ts:39-57`) don't have path 1's problem — they only emit `session_end` on the provider's real turn-boundary event. **Claude Code, the default and most common provider, is the one with the per-round misfire**, making it the dominant real-world contributor to "populates at all sorts of times."

## 3. Why not fix the premature `session_end` synthesis directly?

Considered and rejected. `claude-translator.ts`'s premature-`Done` behavior is load-bearing elsewhere — `reducer.ts:198-253`'s `StreamFlushObserved` handler explicitly special-cases `Done.completed` (re-promoting it to `Streaming` on the next flush) specifically *because* this premature transition is expected and relied upon to clear the "Working…" indicator between tool-call rounds. Changing the synthesis itself risks a much wider blast radius across the whole turn-lifecycle state machine for a fix that only needs to change when two ambient-summary hooks fire. Fixing the *consumers* instead (§4) is strictly narrower and lower-risk.

## 4. The fix: trigger off the backend-authoritative `turn_active` edge instead

The backend already has, and already surfaces to the frontend, a signal that genuinely is 1:1 with turn completion: `BlockControllerRuntimeStatus.turn_active` (`agentmux-srv/src/backend/blockcontroller/mod.rs`), flipped `false` by the health monitor **only** on the CLI's real `"result"` line (`agentmux-srv/src/backend/blockcontroller/persistent.rs:918-923`) — not per tool-call round — and re-armed per turn by `send_message`'s `mark_turn_active_returning_was_active()` (`persistent.rs:356-359`). This is already streamed live to the frontend via the `controllerstatus` WPS event and already consumed by `useControllerStatusEvents.ts`'s `onTurnActive` callback, which `agent-view.tsx` already used to dispatch `ReconcileTurnActive` (added for a different bug — stuck `Streaming`/`Idle` phases, `docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md`) but never consulted for the ambient-summary trigger.

**Change:**
- `useControllerStatusEvents.ts` gains a new pure, exported, unit-tested function `didTurnJustEnd(prev: boolean | undefined, next: boolean): boolean` — true exactly on a `true → false` edge; `undefined → false` (a pane opened onto an already-idle agent) is explicitly NOT a turn-end, since there's nothing new to summarize.
- `agent-view.tsx` adds a `turnJustEndedAtom` signal (`createSignal<number>(0)`) and a `reconcileTurnActive(active)` helper that both the mount-time one-shot (`onControllerStatus`) and every live `controllerstatus` event (`useControllerStatusEvents`'s `onTurnActive`) now route through — replacing their previous direct `dispatchPaneIfRegistered({type: "ReconcileTurnActive", ...})` calls with a single shared helper that also bumps `turnJustEndedAtom` on `didTurnJustEnd`.
- `useAgentActivitySummary.ts` / `useNextPromptSuggestion.ts` each split their single `createEffect(on(turnPhase, ...))` into two: one unchanged (still watches `turnPhase` for `Submitting`, to bump the local `activeTurnId` staleness guard and, for the suggestion hook, clear the stale suggestion — guard 1 in that hook's own doc comment), and a new one, `createEffect(on(turnJustEndedAtom, ..., { defer: true }))`, which fires the Haiku RPC. `defer: true` skips the effect's initial run at mount (the atom starts at 0).

This closes all five over-trigger paths in §2 at once: none of the frontend's synthetic/forced `Done` transitions (premature per-round, both bounded timeouts, `FailureObserved`) correspond to a real backend `turn_active` flip, so none of them bump `turnJustEndedAtom`. Only a genuine CLI `"result"` line does.

## 5. Explicitly out of scope: the unconsumed 20s pushed-summary sweep

`agentmux-srv/src/backend/reactive/activity_watcher.rs::run_agent_summary_loop` runs a flat 20-second timer, independent of turn state entirely — its only gating is `shellprocstatus == STATUS_RUNNING` (true for a persistent-mode process's entire life, not just mid-turn) and whether the block's output changed since the last summary (lines 99-119). It calls the same Haiku primitive via `generate_pushed_activity_summary` and publishes a `WaveEvent{event: "agent:summary", ...}` (`activity_watcher.rs:155-166`) — **verified directly: it does not write `term:ambient_summary` itself**, only publishes that event.

**Verified directly: this event has zero frontend consumers.** `swarm-model.ts` (the documented intended consumer per `docs/specs/SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md` §4) derives its activity-summary field purely by re-reading `term:ambient_summary`/`term:osc_title` off the reactive block-meta atom, with no `agent:summary` WPS subscription anywhere in the frontend tree. So today this sweep is not the cause of the reported symptom — it's a separate, currently-inert cost leak: a real Haiku CLI subprocess spawned every 20 seconds for every running agent, for a UI signal nothing reads. `SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md`'s own "Open questions" §1 already flagged the flat-timer design as unresolved (*"Haiku push cadence 20s ok? … Alternative: only on turn-phase transitions"*). Left untouched here — deliberately scoped out, not missed. Re-wiring it to fire on `turn_active` edges (mirroring §4) or removing it outright is a real, separate follow-up someone should pick up; flagging it here so it isn't lost.

## 6. Test coverage

`useControllerStatusEvents.test.ts` gained 6 new cases for `didTurnJustEnd` covering every transition (`true→false`, `false→true`, `true→true`, `false→false`, and both `undefined→*` first-reading cases). Full frontend suite (851 tests, 59 files) passes with no regressions.

## 7. References

- Internal: `frontend/app/view/agent/hooks/useAgentActivitySummary.ts`, `frontend/app/view/agent/hooks/useNextPromptSuggestion.ts`, `frontend/app/view/agent/hooks/useControllerStatusEvents.ts`, `frontend/app/view/agent/agent-view.tsx`, `frontend/app/store/agent-pane-state/reducer.ts:198-253,497-560,752-950`, `frontend/app/view/agent/providers/claude-translator.ts:201-235`, `agentmux-srv/src/backend/blockcontroller/persistent.rs:356-359,918-923`, `agentmux-srv/src/backend/reactive/activity_watcher.rs`, `agentmux-srv/src/server/app_api/session.rs`.
- `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md` — the gateway this feature is routed through; §0 already named "calls firing too often" as a pre-existing symptom, but its own fix (single-flight + generation fencing) addressed concurrency/staleness, not over-triggering from spurious `Done` transitions — §6 "Open questions" explicitly flagged a debounce/rate-limit primitive as designed but never implemented.
- `docs/specs/SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md` §4, "Open questions" §1 — the spec that shipped the 20s pushed sweep, with its own author flagging the flat-timer-vs-turn-phase-transition question as unresolved (§5 above).
- `docs/specs/SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md` §2.3 — explicitly scoped out changing "when these calls fire," confirming the over-trigger problem was known and deliberately left for a later fix (this one).
- `docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` — Finding 1 produced the `ReconcileTurnActive`/`turn_active` machinery this fix reuses; Finding 3 produced the `MAX_CONCURRENT_PULL_CALLS` semaphore (a related but separate concurrency fix, already shipped).
- `docs/retro/retro-haiku-activity-pane-header-2026-06-24.md` — an earlier, unrelated bug in this same feature (summary never fired at all, reading an empty WPS ring buffer instead of the FileStore) — already fixed; confirms this feature has a history of trigger-plumbing issues, of which this is the second.
- `docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md` — origin of the live `useControllerStatusEvents`/`onTurnActive` wiring this fix builds on.
