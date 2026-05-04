# Retro: PR-D (tab-state-reducer) — premise doesn't fit the actual frontend

**Date:** 2026-05-03
**Status:** Plan stop-condition triggered. Writing this BEFORE proceeding so the architecture decision is captured.
**Plan reference:** `docs/specs/frontend-reducer-implementation-plan-2026-05-03.md` PR-D (slice #7)

## What the plan said

> **PR-D — Slice #7: tab-state-reducer**
> Goal: frontend mirror of srv tab state (active tab, tab order, per-tab metadata). First pure mirror slice; validates §6 echo-loop guard.
> Effort: ~3 days.

The plan assumed there was tab-related state on the frontend that needed to be mirrored from srv via a reducer with an echo-loop guard.

## What I actually found

After ~10 minutes of code inspection (`frontend/app/store/global.ts:40–110`, `command-registry.ts`, `keymodel.ts`, `layoutModel.ts`):

1. **There are no local tab atoms.** All tab/workspace/window state is derived via `createMemo` over `WOS.getObjectValue`. Wstore already does the mirror.
   ```ts
   export const tabAtom = createMemo<Tab>(() => {
       return WOS.getObjectValue(WOS.makeORef("tab", staticTabId()));
   });
   export const activeTabId = createMemo<string>(() => {
       const ws = workspace();
       return ws?.activetabid || ...;
   });
   ```
2. **`setActiveTab` is a one-line RPC wrapper** (`global.ts:866`): `WorkspaceService.SetActiveTab(ws.oid, tabId)`. The result lands back in wstore, which reactively updates `activeTabId` automatically. Single chokepoint already exists.
3. **`pendingbackendactions` is part of the LAYOUT slice**, not tab state — it lives in `frontend/layout/lib/layoutModel.ts` + `layoutPersistence.ts`. The plan flagged this as a decision point (lean: tab-state) but the answer is now obvious: it's already in the layout module.

## Why a tab-state reducer would be wrong

If I built it as planned, the resulting module would:

- **Duplicate state**: hold a `Map<tabId, TabSnapshot>` that's already in wstore
- **Duplicate subscriptions**: subscribe to wstore `tab:*` updates that consumers already subscribe to via `tabAtom`
- **Add a pass-through dispatch**: `dispatchTab(SetActive, ...)` that just calls `setActiveTab` which already exists
- **Add an echo-loop guard** for an echo that doesn't exist (no local→srv→back loop because the local "set" path IS the srv RPC)

Net effect: ~600 LOC of new code that adds zero invariants and zero auditing value the existing event log (PR-C `command-source.ts`) doesn't already provide for the existing dispatch path.

## What changed about the plan's reasoning

The plan was written without inspecting the existing tab-state surface. It assumed a launcher-event-style scenario (local mirror, multiple writers, async upstream events). The reality is that tab state was always wstore-mirrored — there's no architectural problem to solve.

## Comparison to slices that DID fit

| Slice | Local writers? | Multi-writer race? | Reducer added value? |
|---|---|---|---|
| #1 agent-document | Yes (3) | Yes (truncate-wipe bug) | Big — fixed live bug |
| #4 agent-pane-state | Yes (~14) | Latent (turnActive ↔ streamingState cohesion) | Real — invariants now enforced |
| #6 launcher-event | Yes (1, but with seed-vs-close race) | Yes (codex P1/P2 fixes) | Real — invariants now in tests |
| #3 source-tagging | N/A (cross-cutting) | N/A | Real — audit trail |
| **#7 tab-state (this)** | **No** | **No** | **None** |

The pattern: reducers add value when frontend state has multiple writers OR when invariants need formal enforcement. Tab state has neither.

## Decision

**Cancel PR-D as planned. Do not implement.**

Three options for what to do instead:

### Option 1 — Skip to PR-E (frontend-layout) ← recommended

Layout state DOES have local multi-writer patterns (drag-resize ticks, focus changes, magnify toggles, leaf-order updates). It's the original target spec'd by srv-side `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md`. The frontend mirror is genuinely useful here.

**Caveat per the plan**: PR-E "blocks on srv E.4.A landing first." Need to check srv status. If srv E.4.A is in flight, this is the next move.

### Option 2 — Find a different real target

Search for any frontend state that DOES have a multi-writer pattern. Candidates worth a 30-min investigation:
- The activity log (per-pane diagnostic entries) — multiple hooks append
- Token usage store (`@/store/token-usage`) — turn-aggregation
- Focus manager (per-block focus tracking)

If any of these have race-prone patterns, they'd be a better fit than tab-state.

### Option 3 — Stop the slice migration here

We've shipped 4 slices (agent-doc, agent-pane-state, launcher-event convergence, source-tagging). The pattern is established. Layout (PR-E) is the only remaining one with a clear value prop. If srv E.4.A isn't ready, we could pause the migration and pivot to other work until it is.

## Updated roadmap

| # | Slice | Status |
|---|---|---|
| #1 | agent-document | ✅ Shipped |
| #2 | conventions | ✅ Shipped |
| #3 | source-tagging | ✅ Shipped (PR-C) |
| #4 | agent-pane-state | ✅ Shipped (PR-A) |
| #5 | frontend-layout | ⬜ PR-E — blocks on srv E.4.A |
| #6 | launcher-event convergence | ✅ Shipped (PR-B) |
| #7 | **tab-state** | **❌ Cancelled — see this retro** |
| #8 | pane-tree | ⬜ Deferred (waits for srv E.4.B) |

## Lessons for future planning

The plan was written in one pass without code inspection per slice. Doing the inspection during implementation caught the mismatch — but ideally the spec phase for each slice should include "verify the premise by reading the current code" as a checklist item before estimating.

Should add to the conventions doc §10: **"Before specifying a new mirror slice, read the existing wstore-derived atoms for the domain. If everything's already a `createMemo` over `WOS.getObjectValue` with no local writers, the mirror reducer adds nothing — skip it."**
