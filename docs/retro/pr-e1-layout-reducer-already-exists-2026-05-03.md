# Retro: PR-E1 (layout focus/magnify reducer) — already exists

**Date:** 2026-05-03
**Status:** Stop-condition triggered (second time today). Writing this BEFORE any more code.
**Plan reference:** `docs/specs/frontend-reducer-implementation-plan-2026-05-03.md` PR-E
**Decision context:** User picked E1 (refactor-only) over E2 (refactor + srv sync) for smaller blast radius

## What I assumed for PR-E1

A clean greenfield reducer for local focused/magnified state, similar in shape to PR-A (agent-pane-state). One day of work.

## What's actually there

`frontend/layout/` has a **complete tree reducer architecture already**:

| Component | Where | What |
|---|---|---|
| Action types enum | `frontend/layout/lib/types.ts` | 15+ types: FocusNode, MagnifyNodeToggle, Move, Insert, Delete, Resize, Swap, ComputeMove, ResizeNode, etc. |
| Action dispatcher | `layoutModel.ts:486 (treeReducer)` | Switch-by-action-type, ~75 LOC |
| Pure-ish action handlers | `layoutTree.ts` | 544 LOC, 11 functions, e.g. `focusNode(state, action)` mutates passed-in state |
| Projection | `layoutModel.ts:571` | `localTreeStateAtom._set({ ...this.treeState })` after each reducer run |
| Validators (the multi-writer surface) | `layoutFocus.ts:15 (validateFocusedNode)`, `layoutMagnify.ts:166 (validateMagnifiedNode)` | Called from `layoutGeometry.ts:65–66` after geometry recompute; also mutate `treeState.focusedNodeId` directly |

The "mutations" in `layoutFocus.ts:21,22,34,38–45` are inside `validateFocusedNode` — a **post-geometry repair function**, not a parallel reducer pathway. It runs sequentially after the tree reducer (when `computeAdditionalProps` re-derives geometry). They don't race; they're stages of one pipeline.

## The two writers question

Strictly speaking, `treeState.focusedNodeId` IS written from two places:
1. Via `treeReducer` action `FocusNode`
2. Via `validateFocusedNode` direct assignment

But they execute **sequentially** in the same call stack:
```
user click
  → focusNode(model, nodeId)
    → model.treeReducer({type: FocusNode, ...})
      → layoutTree.focusNode(treeState, action)  // sets focusedNodeId
    → updateTree()
      → computeAdditionalProps()
        → validateFocusedNode(model, leafOrder)  // may rewrite focusedNodeId for stale-stack case
    → localTreeStateAtom._set({ ...treeState })  // single projection
```

No concurrent writers. No async race. No network event interleaving. The validator sees the post-reducer state and either confirms it or repairs it before projection.

This is **not the multi-writer pattern** the convention is designed to fix.

## Value assessment

A convergence PR (folding `validateFocusedNode` and `validateMagnifiedNode` into reducer arms, replacing in-place mutation with immutable updates, adding audit events + tests) would:

| Pro | Con |
|---|---|
| Validators become pure + testable | ~1 week of work for the focused/magnified slice alone (layoutTree + layoutFocus + layoutMagnify + tests) |
| Audit log captures geometry-driven repair events | Layout is the most-touched UI surface; regressions affect everything |
| Explicit "RepairFocus" / "RepairMagnify" commands surface invariants | No known bug or race motivates the work — pure consistency-with-conventions |
| Eventual srv sync (E2) needs a frontend reducer anyway | Could just write the reducer as part of E2 instead of a separate refactor |

The two real action items I see are:
- "We need to enforce focus repair invariants" — yes, but they're already encoded in the validator functions; the existing tests aren't bad; rewriting just to use a different pattern is gold-plating.
- "We need cross-window focus sync (E2)" — yes, but that's E2 territory, and doing E1 first as scaffolding doesn't reduce E2's risk meaningfully.

## Decision

**Cancel PR-E1 as planned. Do not implement.**

The layout system already has a reducer. It uses an older convention (in-place mutation, no audit events, no slot abstraction) but it works. Converging it to the new conventions is a multi-day investment with no known correctness benefit.

## Updated roadmap

| # | Slice | Status | Notes |
|---|---|---|---|
| #1 | agent-document | ✅ Shipped | |
| #2 | conventions | ✅ Shipped | |
| #3 | source-tagging | ✅ Shipped | |
| #4 | agent-pane-state | ✅ Shipped | |
| #5 / E1 | frontend-layout (refactor-only) | ❌ Cancelled — see this retro | Existing reducer is good enough |
| #5 / E2 | frontend-layout + srv sync | 🟨 **Real work, real value** — write spec when needed | Multi-window focus sync is the user-visible feature |
| #6 | launcher-event convergence | ✅ Shipped | |
| #7 | tab-state | ❌ Cancelled (PR-D retro) | No multi-writer; wstore-derived |
| #8 | pane-tree | ⬜ Deferred (waits srv E.4.B) | |

## Lessons (compounded with PR-D retro)

The implementation plan's per-slice descriptions were written without code inspection. Two of three remaining slices (D + E1) turned out to be premised on assumptions that don't hold. **The conventions work was right** — it set the bar. **The existing-code inventory was wrong** — it assumed reducer-shaped code was either absent (D, where wstore already mirrors) or unreducer-shaped (E, where there's already a reducer).

**Add to conventions §10 / planning checklist:**

> Before writing a "new reducer slice" spec, do a 30-minute code inspection:
> 1. Is the state already wstore-mirrored? (If yes, no slice needed.)
> 2. Is there already a reducer or reducer-shaped code? (If yes, the slice is convergence-only — assess vs. gold-plating.)
> 3. Is there a known bug or invariant violation? (If no, the slice is consistency-only — assess vs. gold-plating.)

## What's actually left to ship

Looking at the 8 slices honestly:

- **3 high-value PRs already shipped** (#1 fixed a live bug; #4 + #6 added real invariants + testability)
- **1 cross-cutting helper shipped** (#3 source-tagging)
- **2 slices cancelled** (#7 tab-state, #5 E1 layout-refactor — not applicable)
- **1 slice has real future value but needs a spec** (#5 E2 layout + srv sync — multi-window focus sync feature)
- **1 slice deferred indefinitely** (#8 pane-tree — waits on srv E.4.B)

**The frontend reducer migration is essentially done.** Further slices either don't fit (D, E1) or wait on upstream work (E2 needs a real spec; #8 needs srv E.4.B).

## Recommended next move

Three options:

1. **Stop the slice migration here.** Pivot to other work. Pick up E2 when multi-window focus sync becomes a user-visible priority (or when the user explicitly asks for it).
2. **Write the E2 spec now.** ~1 day of design work; doesn't commit to implementing. Captures the multi-window focus-sync design while context is fresh.
3. **Pivot to one of the other named follow-ups** from the implementation plan: diagnostics panel surface (PR-C follow-up, ~0.5d) or the README/architecture docs for `frontend/app/store/`.

My recommendation: **option 1 (stop)** + **option 3's diagnostics panel** as a small completion. Stops the migration cleanly with a tangible user-visible feature (the audit log surface) and avoids the trap of inventing more work to keep the queue moving.
