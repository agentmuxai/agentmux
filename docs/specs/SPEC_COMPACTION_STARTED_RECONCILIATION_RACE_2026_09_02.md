# Spec: `compaction_started` Arriving Before Turn-Phase Reconciliation Drops the Ping Permanently

**Date:** 2026-09-02
**Repo:** agentmuxai/agentmux
**Trigger:** User report — after loading/resuming an agent, the "Working…" status
row sometimes disappears just before a "Compact conversation" episode finishes,
leaving the user with no visibility that the agent is still busy.

**Relationship to prior art:** `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
is the feature this bug lives inside — `frontend/app/store/agent-pane-state/reducer.ts`'s
`CompactionStarted` case has already been hardened across 11 rounds of race-condition
fixes (see the inline `reagent`/`codex` round comments on that case), all of them
about the ping arriving **too late** relative to some other transition. This spec
covers the one direction those rounds don't: the ping arriving **too early**.

---

## 1. Problem

`compaction_started` (published by the `PreCompact` hook, `agentmux-bashwrap/src/precompact.rs`)
travels over a **separate WPS transport** (`persist: 0`, never replayed — see
`useCompactionStream.ts`'s module doc) from the primary NDJSON stream that carries
`TurnStart`/`TurnEnd`/`compact_boundary`. The reducer's `CompactionStarted` handler
(`reducer.ts` ~L1141-1145) only sets `state.compacting` — the field that suspends the
stuck-stream watchdog (`StreamWatchdogTick`, same file ~L287-300) and keeps the
"Working…" row visible (`AgentFooter.tsx`'s `AgentWorkingRow`, gated on
`props.loading || !!props.compacting`) — when `workingFromPhase(state.turnPhase)`
is already `true`:

```ts
// reducer.ts, CompactionStarted (current)
if (state.lastEventMs == null || !workingFromPhase(state.turnPhase) || isStaleVsLastBoundary) {
    return { state, events: [] };   // no-op: `compacting` is never set
}
```

Every pane mounts with `turnPhase: { kind: "Idle" }` (`initialState`,
`agent-pane-state/types.ts`). Whether a turn is actually already running gets
reconciled asynchronously — `ReconcileTurnActive` (dispatched from `agent-view.tsx`
on mount and on pane focus) is an authoritative RPC round-trip that promotes
`Idle`/`Done.completed` → `Streaming` once the backend confirms a turn is active.

If a `compaction_started` ping lands **before that RPC resolves**, `turnPhase` is
still `Idle`, `workingFromPhase` is `false`, and the ping is dropped. Because this
transport is `persist: 0`, there is **no replay** — the drop is permanent for that
compaction. `state.compacting` stays `null` for its entire duration.

With `compacting == null`, nothing suspends `StreamWatchdogTick`. A real compaction
produces **zero** NDJSON output for its whole duration (confirmed example:
231.6s — see the original compaction-detection spec §2) — comfortably past
`LIVENESS_RECOVERY_MS` (180s). The watchdog force-demotes the (genuinely healthy,
actively-compacting) `Streaming` phase to `Idle` mid-compaction, well before the
real `compact_boundary` arrives. The "Working…" row disappears; the user sees
nothing until the CLI's next output.

This matches both halves of the report:
- **"after loading an agent"** — the window only exists between mount/focus and
  `ReconcileTurnActive`'s RPC resolving.
- **"sometimes"** — it's a race between two independent async round-trips (the WPS
  ping vs. the RPC), not deterministic.

## 2. Why the existing reducer guard can't just be relaxed

`!workingFromPhase(state.turnPhase)` was added deliberately (round 5, PR #2378) to
stop a **late** ping from setting `compacting` on a pane that's already `Idle`/`Done`
because its real `TurnEnd` already fired — every one of the reducer's "clear
`compacting`" transitions is itself gated on *leaving* a working phase, so a
`compacting` value set while already idle would never get cleared and would strand
the pane on "Compacting…" forever (the exact bug class rounds 1-4 fixed from the
other direction). Simply dropping that gate reintroduces that orphan-state bug.

The two cases — "ping for a turn that already, genuinely ended" and "ping for a
turn that's real but hasn't been locally reconciled yet" — are indistinguishable
from `state.turnPhase` alone at the moment the ping arrives. They only become
distinguishable once reconciliation actually happens.

## 3. Fix: buffer the ping locally, retry once the pane is confirmed working

Rather than touching the reducer's race-hardened `CompactionStarted` gate (11 rounds
of prior, carefully-reasoned fixes — see §2), the fix stays entirely inside
`useCompactionStream.ts`, the hook that already owns this WPS subscription and the
one call site that dispatches `CompactionStarted`:

1. On a `compaction_started` event, dispatch as today. If the reducer accepts it
   (`wasCompactionStartedAccepted`), proceed as before (push the transcript node).
2. If the reducer rejects it (empty events — could be either "genuinely stale" or
   "too early"), remember it in a local `missedPing` slot (`{ trigger, startedAt }`,
   overwritten by any newer ping) instead of discarding it outright.
3. React to the pane's own `turnPhaseAtom` (already threaded into `useAgentStream.ts`
   and now passed down to this hook): the instant `workingFromPhase(turnPhase())`
   becomes `true`, if `missedPing` is still set, **re-dispatch the exact same
   `CompactionStarted` command**.

This re-dispatch reuses every existing reducer guard unchanged — no new reducer
logic, no new state shape:
- If the pane's `turnPhase` promotion to `Streaming` means a real turn genuinely is
  running and this compaction genuinely is (or still is) in flight, the retry now
  passes `workingFromPhase` and `compacting` is set correctly — closing the gap.
- If the compaction's own `compact_boundary` already arrived in the meantime (a
  `CompactionBoundary` command sets `state.lastCompactionBoundaryAt`), the retry is
  correctly rejected again by the existing `isStaleVsLastBoundary` check — no new
  staleness logic needed, the round-6 guard already covers a delayed retry exactly
  like it covers a delayed original delivery.
- The retry fires at most once per missed ping (cleared unconditionally after the
  retry attempt, success or reject) — no retry loop, no polling.

Net effect: the reducer's `CompactionStarted` case is untouched. The fix is
entirely "try again once we know more," using the hook's existing accept/reject
signal as the oracle, not a new inference.

## 4. Scope / non-goals

- Does not change `resolveCompactionStart`'s staleness window
  (`MAX_PLAUSIBLE_COMPACTION_MS`) — that guards the *original* WPS payload's
  `startedAt` against clock skew / implausible age, orthogonal to this retry.
- Does not add persistence or replay to the `compaction_started` WPS channel
  itself — the retry is a purely local, in-memory buffer scoped to one mounted
  pane's hook instance; it does not survive a pane unmount/remount (a fresh mount
  goes through the same mount-time `ReconcileTurnActive` path and would see a
  fresh live ping if compaction is still ongoing by then, same as today).
- Does not address a hypothetical *second* independent race after the retry fires
  (e.g. a brand-new, different compaction starting and completing entirely within
  the same reactive tick as the retry) — judged unreachable in practice since the
  retry fires synchronously off the same `turnPhase` transition a subsequent ping's
  acceptance would itself depend on; see the fix's implementation comment for the
  detailed reasoning.
