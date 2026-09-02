# Spec: `compaction_started` Arriving Before Turn-Phase Reconciliation Drops the Ping Permanently

**Date:** 2026-09-02
**Status:** implemented — shipped in PR #2928.
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

## 3. Two failed attempts before landing on the real fix

The first two versions of this fix (both reviewed and rejected on PR #2928) tried to
solve this entirely inside `useCompactionStream.ts`, the hook that owns the WPS
subscription: buffer a rejected ping in a local `missedPing` variable, and retry the
same dispatch once the pane's `turnPhaseAtom` accessor next reported a working phase.
A second revision added a bounded retry window (`MISSED_PING_RETRY_WINDOW_MS`) after
reagent (P1) caught the first version retrying forever on every subsequent stream
chunk when a retry itself failed.

Both were structurally unsound, and a third review round (reagent P1 + codex P2,
independently, on the *second* revision) pinned down exactly why: a hook observing
only the **resulting** `turnPhase` value cannot tell "this promotion is the SAME turn
the buffered ping was about" from "this is a later, unrelated turn." Concretely — a
genuinely stale ping (its real turn already ended before the ping arrived) gets
buffered; its confirming `ReconcileTurnActive(active: false)` is a same-ref no-op
when `turnPhase` is already `Idle`, so there is no reactive signal the hook can
observe to clear the buffer. The retry effect then fires on whatever working-phase
transition comes *next* — which can be a completely unrelated `TurnStart` from the
user's next message — and re-dispatch the stale ping against it. If no matching
`compact_boundary` was ever recorded for that stale compaction, the reducer's
`isStaleVsLastBoundary` guard has nothing to reject it against and falsely accepts
it, setting `compacting` on a turn that was never compacting. A time window only
narrows this (codex P2, second pass): too short and it doesn't cover a genuinely slow
`ReconcileTurnActive` RPC (regressing to the original bug); too long and it doesn't
prevent the false-positive replay. There is no time-based value that closes both
sides — the retry has to be resolved by the actual reconciliation *result*, which a
value-observing hook effect cannot access when that result doesn't change the value.

## 4. Fix: buffer in the reducer, where the discrete event is visible

The reducer, unlike the hook, processes every dispatched command as a discrete event
regardless of whether it changes the resulting state — so it can react to
`ReconcileTurnActive(active: false)` as a real signal ("this specific reconciliation
attempt just confirmed inactive") even when `turnPhase` stays `Idle` either way. The
fix moves entirely into `reducer.ts` and `types.ts`; `useCompactionStream.ts` reverts
to its original plain dispatch-and-check form.

**New state:** `state.pendingCompactionPing: { trigger, startedAt } | null`
(`types.ts`).

**`CompactionStarted`** — when `!workingFromPhase(state.turnPhase)` (and not stale vs.
the last known boundary, and the stream is subscribed — both unchanged from before):
instead of an unconditional no-op, checks whether the current phase is one a *later*
authoritative signal could still legitimately promote for this same turn —
`Idle` / `Disconnected` / `Done.completed` (exactly the set `StreamFlushObserved`'s own
promotion arm already treats as promotable). If so, buffer onto
`pendingCompactionPing` and emit a distinct `compaction-started-buffered` event (a
diagnostic/test signal, not consumed by the frontend — see the transcript-node design
below). Any other not-working phase (`Done.errored`/`stopped`/`interrupted`,
`Interrupting`) is still a true no-op exactly as before — round 5's orphan-state
guard: nothing can ever promote those back into working, so buffering there would
just strand the field.

**Promotion — two sites, both authoritative "this same turn is genuinely active"
signals, mirroring `StreamFlushObserved`'s own existing promotion standard:**
- `ReconcileTurnActive(active: true)`, on its `Idle`/`Done.completed` → `Streaming`
  promotion: if `pendingCompactionPing` is set, apply it into `compacting` and emit
  `compaction-started` (in addition to `turn-active-reconciled`).
- `StreamFlushObserved`'s existing `Idle`/`Disconnected`/`Submitting`/`Done.completed`
  → `Streaming` promotion arm: same treatment — resumed live content is proof the
  turn is genuinely active, exactly the standard that arm already uses for promoting
  the phase itself.

**Discard — the reducer's structural advantage over the hook:**
- `ReconcileTurnActive(active: false)` clears `pendingCompactionPing` *unconditionally*,
  independent of whether `turnPhase` itself changes (the common case: already `Idle`,
  a same-ref no-op for the phase, but still a distinct event this command carries and
  the reducer can still act on). This is the exact gap both hook-only attempts had no
  way to close.
- `TurnStart` clears it explicitly and does **not** promote it — a fresh
  user-initiated turn is definitionally not the turn a buffered ping was about. This
  matters concretely: without it, the very next `StreamFlushObserved`
  (`Submitting` → `Streaming`, which fires for practically every `TurnStart`) would
  otherwise inherit and falsely promote the stale ping onto the new turn — this is
  the precise mechanism codex's finding described.
- `CompactionBoundary` clears it once its own (parsed `frameTimestamp`) completion
  time is at or after the buffered ping's `startedAt` (that ping's compaction is
  confirmed already finished — nothing left to promote); preserves it, mirroring
  `preservesNewerCompaction`'s existing pattern, when the boundary is for an older
  compaction than the one currently buffered.
- Every "whatever compaction was in flight is now moot" terminal transition that
  already clears `compacting: null` (`TurnReset`, `TurnStartFailed`,
  `InterruptTimeoutElapsed`, `SubmitTimeoutElapsed`, and `FailureObserved`'s
  `turnWasEnded` branch, plus the main/working-phase paths of `StreamUnsubscribe` and
  `TurnEnd`) clears `pendingCompactionPing` the same way.
- `StreamUnsubscribe`'s and `TurnEnd`'s *early same-ref no-op branches* — reached when
  the phase is ALREADY non-working (`!wasWorking` / already `Disconnected`) — also
  clear `pendingCompactionPing` when it's set (reagent P1, PR #2928, a second review
  round after the main fix landed): these branches predate this field and previously
  assumed "no in-flight turn ⇒ nothing to clear," which no longer holds now that a
  ping can legitimately be buffered while `Idle`/`Disconnected`. Left unfixed, a pane
  backgrounded (or a late TurnEnd ack landing) while a ping was buffered could carry
  it forward to be wrongly promoted onto a later, unrelated turn on resubscribe —
  the same failure class as the `TurnStart` gap above, reached via a different path.

**Transcript node** — the "Compacting conversation…" node is pushed by
`useCompactionStream.ts` watching the reactive `compacting` signal itself (kept in
sync with `state.compacting` by `registerAgentPane`'s generic per-field projection,
regardless of which dispatch changed it) and firing whenever it transitions from
`null` to set — not by inspecting any one dispatch's returned events. This detail
changed after review (reagent P1, PR #2928, a third round): the promotion sites live
in `agent-view.tsx` (`ReconcileTurnActive`) and `stream-flush-queue.ts`
(`StreamFlushObserved`), neither of which has access to this hook's document queue or
dedup cache, and neither actually captured its dispatch's return value — so the
transcript node was silently never pushed for a promoted (as opposed to live-accepted)
ping, even though `state.compacting` was set correctly. Watching the signal instead of
each call site closes this for every current AND future promotion path uniformly, with
no per-call-site wiring needed. `wasCompactionStartedAccepted` (the old accept-check
helper) is removed as part of this — the signal-driven push makes it structurally
unreachable to push a node for a rejected-or-merely-buffered ping, so there's nothing
left for it to guard against.

No change to `isStaleVsLastBoundary`, `resolveCompactionStart`'s staleness window, or
any of the 11 prior `CompactionStarted`/`CompactionBoundary` race-hardening rounds —
this is a new, orthogonal field with its own narrow set of setters/clearers, not a
relaxation of any existing guard.

## 5. Scope / non-goals

- Does not add persistence or replay to the `compaction_started` WPS channel itself —
  `pendingCompactionPing` is ordinary reducer state, scoped to one pane's lifetime the
  same as `compacting` already is (wiped on `TurnReset`, not carried across a fresh
  `initialState()`).
- Does not change what counts as "stale" for an already-*promoted* `compacting` value
  (`CompactionBoundary`'s existing `preservesNewerCompaction` logic) — only adds the
  analogous, independent gate for the *buffered-but-not-yet-promoted* case.
