# SPEC: Agent Pane Session Cost/Token Totals

**Date:** 2026-07-02
**Status:** implemented — shipped in #1920. Verified 2026-08-23: `reducer.ts`'s `accumulateStats()` function cites this spec by filename in its own doc comment; a distinct `sessionTotals` field (separate from the per-turn `sessionStats` this spec's bug report describes) is wired into `AgentComposerStrip.tsx` and `agent-view.tsx:2225`.
**Area:** Agent pane (frontend)
**Severity:** P2 — misleading UI, no data loss

---

## Problem

The agent pane's composer strip (`AgentComposerStrip`, the slim status row directly
above the textarea) is documented as showing "final session totals" next to the token
count and elapsed time. In practice it shows the exact same number as the per-turn
"Worked" delimiter row (`AgentWorkingRow`, rendered once below the transcript) —
i.e. the cost/tokens for the single most-recently-completed query, not a running
total across every query sent in the pane's lifetime.

User-visible symptom: after sending several queries in one pane, the number shown
"near the input" never grows past what a single query cost — it should be the sum
across all queries in the session, resetting only when the pane/session resets.

---

## Root Cause

There is exactly one piece of state involved, `AgentPaneState.sessionStats`
(`frontend/app/store/agent-pane-state/types.ts:167`), and it is *correctly*
per-turn:

- `TurnStart` nulls it (`reducer.ts:382`).
- `TurnEnd` overwrites it with `mergeStats(command.stats, state.turnTokens)`
  (`reducer.ts:409,451`) — the merge combines the just-finished turn's CLI
  `result` stats with that turn's live token signal. It never reads or adds to
  the *previous* `sessionStats` value.

Both consumers bind to the identical projected atom:

- `AgentWorkingRow` (`frontend/app/view/agent/components/AgentFooter.tsx:66-141`),
  wired at `frontend/app/view/agent/agent-view.tsx:822-828` via
  `sessionStats={agentAtoms().sessionStatsAtom[0]()}`.
- `AgentComposerStrip` (`frontend/app/view/agent/components/AgentComposerStrip.tsx:97-160`),
  wired at `agent-view.tsx:984-985` via the same
  `sessionStats={agentAtoms().sessionStatsAtom[0]()}`.

Since only one such value exists in the whole pane, and both components read it by
reference, they trivially always match. No cumulative accumulator was ever built —
the composer strip's doc comment ("Final session totals") was aspirational, not
implemented. (A genuine cross-turn accumulator does exist —
`frontend/app/store/token-usage.ts` — but it feeds the global status-bar token
popover, not this pane-local composer strip, and it is keyed by provider/service,
not by pane.)

---

## Fix

Add a second, genuinely-cumulative field to `AgentPaneState` — `sessionTotals` —
that sums each completed turn's stats into a running total for the pane's
lifetime. Leave `sessionStats` (and `AgentWorkingRow`, which correctly shows
per-turn data) untouched. Point `AgentComposerStrip` at the new field instead.

### 1. State shape

`frontend/app/store/agent-pane-state/types.ts`

- Add `sessionTotals: SessionStats | null` to `AgentPaneState`, reusing the
  existing `SessionStats` shape (`cost_usd`, `duration_ms`, `num_turns`,
  `input_tokens`, `output_tokens`) since the composer strip only ever reads
  those fields.
- Initialize to `null` in `initialState()`.

### 2. Reducer accumulation

`frontend/app/store/agent-pane-state/reducer.ts`

- Add an `accumulateStats(totals, merged)` helper next to `mergeStats`: returns
  a new `SessionStats` summing `cost_usd`, `duration_ms`, `input_tokens`,
  `output_tokens` (treat missing fields as `0`) and `num_turns` (fallback `1`
  per completed turn when the CLI didn't report one), seeded from `totals ??
  { }` when this is the pane's first completed turn.
- In the `TurnEnd` arm, after computing `merged` (the existing per-turn value),
  also compute `sessionTotals: accumulateStats(state.sessionTotals, merged)`
  and include it in the returned state alongside the existing `sessionStats:
  merged`.
- Reset `sessionTotals: null` on `TurnReset` (session wipe) alongside the
  existing `sessionStats: null` reset (`reducer.ts:476`) — a wiped session
  has no history to total.
- `TurnStart` does **not** touch `sessionTotals` (unlike `sessionStats`,
  which intentionally nulls per turn) — totals persist across turns by
  design.

### 3. Atom + projection wiring

`frontend/app/view/agent/state.ts`

- Add `sessionTotalsAtom: SignalPair<SessionStats | null>`, initialized to
  `createSignal<SessionStats | null>(null)`, mirroring `sessionStatsAtom`.

`frontend/app/store/agent-pane-state-store.ts`

- Add `sessionTotals: (next: SessionStats | null) => void` to
  `AgentPaneProjections`.
- Add the corresponding `proj("sessionTotals", prev.sessionTotals,
  slot.state.sessionTotals, slot.proj.sessionTotals);` line beside the
  existing `sessionStats` line (~line 190) in `dispatch()`.

`frontend/app/view/agent/agent-view.tsx`

- Register the new projection: `sessionTotals: a.sessionTotalsAtom[1]` in the
  `projections` object passed to `registerAgentPane` (~line 214, beside the
  existing `sessionStats` entry).
- Rewire `AgentComposerStrip`'s `sessionStats` prop (~line 984-985) to read
  `agentAtoms().sessionTotalsAtom[0]()` instead of `sessionStatsAtom[0]()`.
  Leave the `AgentWorkingRow` usage (~line 827) unchanged — it is correctly
  per-turn today.

### 4. Component prop rename (clarity)

`frontend/app/view/agent/components/AgentComposerStrip.tsx`

- Rename the `sessionStats` prop to `sessionTotals: SessionStats | null` and
  update its doc comment to accurately describe cumulative totals (no
  behavior change beyond the name — the prop type is unchanged since both
  reuse `SessionStats`). Update the one call site in `agent-view.tsx`
  accordingly.

---

## Non-goals

- No change to `AgentWorkingRow` / per-turn `sessionStats` — that display is
  already correct (resets to the just-finished turn's own numbers, not a
  running total).
- No change to `frontend/app/store/token-usage.ts` (the global status-bar
  accumulator) — it's a separate, already-working mechanism at a different
  scope (cross-pane, per-provider) and out of scope here.
- No backend changes — `AgentEvent::Cost` already carries correct per-run
  data; this is purely a frontend aggregation gap.

---

## Verification

1. Open an agent pane, send three queries in sequence.
2. After each query completes, confirm the "Worked" row (below the
   transcript) shows only that query's own cost/tokens/duration (should NOT
   equal the running sum).
3. Confirm the composer strip (above the input) shows the cumulative sum of
   all completed queries so far, growing after each one.
4. Trigger a session reset (`TurnReset`) and confirm both values clear to
   null/empty.
