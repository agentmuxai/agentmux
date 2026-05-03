# Frontend Reducer Architecture — spec roadmap

**Date:** 2026-05-03
**Status:** Architecture overview + spec family inventory. Each named follow-up spec needs to be written before its implementation.
**Reads-this-first:**
- `docs/specs/agent-pane-document-reducer-2026-05-03.md` — first slice to land (the live agent-doc bug fix)
- `agentmux-srv/src/reducer.rs` + `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — the canonical reducer pattern, server-side
- `frontend/app/store/launcher-event-reducer.ts` — the only frontend reducer that exists today (window mirror)
- `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — granularity-decision precedent

## What we're trying to make true

Today the frontend has many state mutators with no coordinating layer. State is scattered across:

- Per-pane Solid signals (`createAgentAtoms`, `createTermAtoms`, etc.) with class view-models
- Global Solid atoms in `frontend/app/store/global.ts` (block atoms, focus, layout)
- A wstore mirror layer (`wos.ts`) that reflects srv objects
- `launcher-event-reducer.ts` — single store-level reducer mirroring launcher window state
- Direct RPC calls scattered across components (`pane-actions.ts`, `command-registry.ts`, slash commands, agent app-API endpoints)

Three concrete forces are pushing toward consolidation:

1. **The agent-doc bug**: `documentAtom` has 3 unsynchronized writers — proven to cause a mid-session wipe. Same shape exists in other atoms.
2. **Agent-driven UI mutations**: agents can call MCP tools / app APIs to create panes/tabs/windows. Today these go straight to srv RPCs, bypassing any frontend audit, policy, or coordination with concurrent user actions.
3. **Symmetry with srv**: srv reducer is the source of truth for tab/pane/block state and emits versioned events. The frontend should consume those events through a coherent reducer, not a per-feature mirror.

The end state is **a frontend command/event bus** that:
- Routes all outbound mutations through one chokepoint (audit, policy, optimistic UI)
- Routes all inbound events from srv/launcher/host through one dispatch
- Maintains an in-memory state projection in slices (per-pane and global)
- Preserves the per-slice reducer purity that makes the Rust reducers debuggable

We will not get there in one PR. This document names the staged specs.

## The spec family

Each row is a separate spec to be written. They ship in order; each builds on the previous. Estimates are spec-writing + implementation, not just code.

| # | Spec | Scope | Status | Effort | Depends on |
|---|---|---|---|---|---|
| **1** | `agent-pane-document-reducer-2026-05-03.md` | Single-slice reducer for agent `documentAtom` (Option B from spec). Lives at `frontend/app/store/agent-document-store.ts`. Per-blockId state cell. Fixes the live bug. | **Written, ready** | ~1 day | — |
| **2** | `frontend-reducer-conventions.md` | The shared conventions: command/event types, dispatch signature, slot lifecycle, atom projection, echo-loop guard, test layout. Becomes the template every later spec follows. | **Needed before #3+** | ~1 day spec | #1 sets de facto pattern |
| **3** | `frontend-command-bus.md` | The big one. Single outbound dispatch for user clicks + slash commands + agent app-API calls. Audits, applies policy ("agent X may not delete pane Y"), routes to srv/host/launcher RPCs. Optional optimistic-update queue for slow ops. Doesn't take ownership of state — that stays in the slice reducers. | **Needed**, biggest | 2–3 days spec, ~2 weeks impl | #2 |
| **4** | `agent-pane-state-reducer.md` | Option C from #1. Bundles the other agent-pane atoms — `streamingState`, `sessionStats`, `currentTool`, `turnTokens`, `turnActive`, `stopping`, `pendingMessages` — into one slice with cohesive invariants (e.g. `turnActive ↔ streamingState.active`). | Needed | ~1 day spec, ~2 days impl | #1, #2 |
| **5** | `frontend-layout-reducer.md` | Frontend mirror of srv's Phase E.4 layout reducer (focused node, magnified node, leaf order). Subscribes to srv events, projects into existing layout atoms. | Needed | ~1 day spec, ~3 days impl | #2; coordinates with srv-side `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` |
| **6** | `launcher-event-reducer-convergence.md` | Fold the existing one-off `launcher-event-reducer.ts` into the conventions established by #2. No behavior change — purely structural alignment. | Cleanup | ~0.5 day | #2 |
| **7** | `tab-state-reducer.md` | Frontend mirror of srv's tab state. Active tab, tab order, per-tab metadata. Subscribes to srv events. | Needed | ~1 day spec, ~3 days impl | #2 |
| **8** | `pane-tree-reducer.md` | Frontend mirror of srv's block/layout tree per-tab. The bigger version of #5 — owns the actual tree, not just focus. Has a `rootnode` field that needs the path-representation work flagged in srv Phase E.4. | Speculative — wait for srv E.4.B | ~2 days spec | #5, #7, srv E.4.B |

## Convention layer (#2) — what every slice needs

Before specs #3–#8 can be written usefully, we need the shared convention. Here's the proposed shape, informed by `launcher-event-reducer.ts` and the Rust reducers:

### Command / event types
```ts
type Command<TPayload> = { type: string; payload: TPayload };
type Event<TPayload>   = { type: string; payload: TPayload; version: number };
```
Every slice declares its own discriminated-union `Command` and `Event` types. Versioning enables saga buffer ordering (matching srv's pattern).

### Dispatch signature
```ts
type Dispatch<TCmd, TEvt> = (command: TCmd) => TEvt[];
```
Pure: synchronous, returns the events the command produced. No promises inside dispatch — async work happens before the dispatch (in the caller) or as a side effect after (in subscribers). Mirrors the host reducer's "snapshot-and-drop" rule.

### Slot lifecycle (per-instance slices)
For per-pane slices like #1 and #4:
```ts
interface Slice<TState, TCmd, TEvt> {
  registerSlot(key: string, projection: SignalPair<…>): void;
  unregisterSlot(key: string): void;
  dispatchFor(key: string, command: TCmd): TEvt[];
  snapshotFor(key: string): TState;  // diagnostics + tests
}
```
Pane components register on mount, release on cleanup. Failure to release leaks a state cell — convention: a `useSlot()` hook bundles this.

### Atom projection
Each slice's authoritative state lives in the slice's internal `Map<key, State>`. The frontend's existing per-pane Solid atoms become **projections** — the slice writes through them on each command application. Components keep using `someAtom()` as before; they don't talk to the slice directly.

### Echo-loop guard
Same pattern as `launcher-event-reducer.ts:54`: an `applyingRemote` flag set during inbound-event apply. Outbound commands check the flag and skip emitting if the change is already a remote echo. Prevents double-routing.

### Audit trail
Every dispatch returns events. The bus / slice can buffer the last N (~200) events in memory for diagnostics. Surface in the existing diagnostics panel — single place to see "what mutated and why."

### Tests
- Reducer is a pure function — table-driven tests assert post-state for each command.
- Slice (with slot lifecycle) gets integration tests for register/dispatch/unregister.
- The bus (#3) gets routing tests + policy tests.

## What this is NOT

- **Not** a wholesale rewrite of `wstore` / `wos`. The wstore subscription mechanism stays — it's the inbound channel for srv events. The reducer slices subscribe to it.
- **Not** Redux / redux-toolkit literally. The Rust reducers don't use middleware, action creators, etc. We borrow the pattern (pure update, command/event split) without the ceremony.
- **Not** state colocation reform — atoms stay where they are; the slice writes through them. Consumers don't care that there's a reducer behind the curtain.
- **Not** a saga framework — sagas live srv-side per `SPEC_PHASE_E_SAGAS_2026-04-30.md`. Frontend slices subscribe to terminal events; orchestration is upstream.

## Sequencing rationale

Why this order:

- **#1 first** because the bug is real and shipping it validates the per-slice pattern in production before we generalize.
- **#2 second** because writing more reducers without a convention guarantees inconsistency. Cheap to write; everything else depends on it.
- **#3 next** because the agent-driven UI mutation gap is the architectural thing that the user explicitly raised and the longest-lead-time piece. Writing the spec early lets implementation parallelize with #4–#5.
- **#4 alongside #5** because both are tightening per-pane invariants and don't compete for the same files.
- **#6** is a small cleanup that keeps the codebase consistent; do it whenever bandwidth allows after #2.
- **#7** before **#8** because tab state is simpler than the layout tree, and the path-representation question for `rootnode` (per srv Phase E.4 spec) needs the srv-side decision to land first.

## Open architectural questions

These need to be settled in #2 (conventions) before the rest can be specced cleanly:

1. **Single bus vs. per-slice dispatch**: do all dispatches flow through one entry point (`bus.dispatch(slice, command)`) or do slices expose their own dispatchers (`agentDocStore.dispatch(blockId, command)`)? Single-bus is cleaner for audit; per-slice is what `launcher-event-reducer.ts` does today.
2. **Inbound event routing**: does the bus subscribe to wstore + launcher + host once and fan out to slices, or does each slice subscribe independently? Subscribing once is the symmetric design; current code subscribes per consumer.
3. **Async commands**: how do commands that need an RPC round-trip (e.g. "create pane") fit into a synchronous dispatch model? Likely answer: dispatch returns immediately with a "pending" event; a follow-up event lands when the RPC completes (matches srv's saga pattern).
4. **Policy enforcement layer**: where does "this agent can/cannot do this" live? In #3 (the bus), in middleware, or in the slice? Probably the bus — single chokepoint.
5. **Optimistic updates**: are they per-slice (slice rolls back on rejection event) or bus-level (bus tracks pending commands and rolls back if the corresponding RPC fails)? Defer until a use case actually needs it.

## Next concrete action

Ship spec #1 (already written and approved) so the bug stops biting users. Once that PR is open, draft spec #2 in parallel. Specs #3–#8 wait until #2 is approved.

This document does not commit to implementing #2–#8 — it commits to writing the specs, then deciding which to implement based on priority and capacity.
