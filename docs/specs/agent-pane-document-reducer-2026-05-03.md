# Agent Pane Document Reducer — root-cause + architecture spec

**Date:** 2026-05-03
**Status:** Draft — awaiting review
**Scope:** `frontend/app/view/agent/`
**Triggering bug:** Mid-Claude-session, the agent pane chat history disappears and only a "skeleton" (empty pane shell) remains.

## TL;DR

The agent pane has **three independent writers** to the message-list signal (`documentAtom`) with **no serialization, invariants, or audit trail**:

1. **`useAgentStream`** — appends nodes from live stdout/stderr stream
2. **`useHistoryPagination`** — prepends nodes loaded from blockfile on mount
3. **`/clear` command** — wipes the document on user request

A fourth, unintentional writer is the **truncate fileop branch** of `useAgentStream` (`useAgentStream.ts:366–381`) which calls `setDocument([])` unconditionally. The most likely root cause of the skeleton-wipe bug is a **socket-reconnect race**: when the WebSocket reconnects, `wpsReconnectHandler()` re-issues subscriptions; if the backend has reinitialized the output blockfile in the meantime, a stale `truncate` event arrives after the resubscribe and clears the live document.

This spec proposes:

1. **Immediate-relief patch** (1–2h) — gate the truncate handler behind a session-active guard so a transient reconnect can't wipe a live conversation.
2. **Architectural fix** (medium effort) — introduce a small frontend reducer (`agentDocumentReducer`) that serializes all `documentAtom` mutations and enforces invariants ("the document only goes from non-empty to empty via an explicit, user-initiated event"). Pattern follows the existing host/launcher reducers (task #182, `agentmux-cef/src/reducer/`, `agentmux-launcher/src/reducer/`).

## Bug — what we know

### Symptom

Mid-Claude-session, the agent pane's rendered message list goes from "rich conversation history" to "empty pane shell." User has not pressed any clear/reset action. Re-opening the pane (or a hot-reload) restores the messages from the blockfile — i.e. the on-disk history is intact; the in-memory document atom was wiped.

### Architecture (current state)

- `documentAtom` is a Solid signal pair created per-pane by `createAgentAtoms(blockId)` (`state.ts:70–99`)
- It holds `DocumentNode[]`, the rendered message-tree
- Three writers (above), no coordination
- `useAgentStream.ts:366–381` handles `fileop=truncate` events from the live file subject by calling `setDocument([])` unconditionally
- `wpsReconnectHandler()` (`wps.ts:30–34`) re-subscribes to all `fileop` events on socket reconnect — this is where the race window opens

### Ranked root-cause hypotheses

| Likelihood | Cause | Evidence |
|---|---|---|
| **HIGH** | Socket-reconnect race delivers a stale `truncate` to a still-active session, wiping the document | `useAgentStream.ts:366–381` clears unconditionally; `wps.ts:30` resubscribes on reconnect; backend reinit between subs is the race window |
| HIGH | `/clear` accidentally invoked (user, hotkey, MCP tool) | `commands/global/clear.ts:17–20` calls `setDocument([])`; symptomatic match but requires user action |
| MEDIUM | `documentVersion` bump from history-load races against live stream's dedup-map rebuild | `useAgentStream.ts:329–330` resets `nodeIndexMap`/`nodeIdSet` on version bump; `useHistoryPagination.ts:155–156` calls `bumpDocumentVersion()` after prepend; if a stream batch is mid-flush during the bump, indices go stale |
| MEDIUM | `useAgentStream` and `useHistoryPagination` race on `setDocument` — Solid batches, last-write-wins | Both hooks own the same setter with no sequencing primitive |
| LOW | Block-meta change triggers atom factory rerun → fresh `[]` document | `agent-view.tsx:91` uses `createMemo(() => createAgentAtoms(model.blockId))`; `blockId` is stable post-create, but a future bug here would manifest exactly as the reported symptom |

The HIGH/MEDIUM causes share a common shape: **multiple writers, no invariants, one of them clears state**. That's the architectural bug — the truncate-race is just the manifestation that's biting now.

## Why a reducer (and which reducer)

We already use the reducer pattern in three production places + one frontend instance:

- `agentmux-srv/src/reducer.rs` — srv reducer (Phase E): workspaces, tabs, blocks, layouts, identity. Largest scope; serves as the canonical state.
- `agentmux-cef/src/reducer/` — host reducer (task #182 PR-F-2): browsers, panes, drag, pool, quit, top-level
- `agentmux-launcher/src/reducer/` — launcher reducer (task #182 PR-A through PR-E): window lifecycle
- `frontend/app/store/launcher-event-reducer.ts` — **frontend reducer**: subscribes to launcher events, holds an in-memory `knownEntries: Map`, mirrors into store atoms, has echo-loop scaffolding (`applyingRemote`)

The closest analog to this proposal is **`launcher-event-reducer.ts`** — that's the prior art for a frontend store-level reducer.

The Rust reducers all follow the same shape: pure `update(&mut State, Command) → Vec<Event>`, no I/O, snapshot-and-drop the lock, sub-millisecond mutex hold. Tests assert post-conditions per command.

This pattern transfers directly to the frontend with minor adjustments:

- **State**: `AgentDocumentState { nodes: DocumentNode[]; sessionPhase: "loading-history" | "active" | "ended"; lastTruncateAt: number | null; ... }`
- **Commands** (the operations that today mutate `documentAtom`):
  - `HistoryLoaded { nodes }` — from `useHistoryPagination`
  - `StreamAppend { nodes }` — from `useAgentStream` flush
  - `StreamTruncate { reason }` — from `fileop=truncate` (now an explicit, contextual command)
  - `UserClear` — from `/clear`
  - `SessionStart`, `SessionEnd` — phase transitions
- **Invariants enforced inside the reducer**:
  - `StreamTruncate` is a no-op when `sessionPhase === "active"` AND the truncate isn't the first one within a small window after `SessionStart` (catches the reconnect race)
  - `UserClear` is the only path that can take the document from non-empty to empty during an active session
  - `HistoryLoaded` only prepends — never overwrites
  - Stream appends are dedup-checked against existing node IDs by the reducer, not by ad-hoc maps in the hook
- **Output**: the new state replaces `documentAtom` via a single, serialized setter

Reducer ownership of dedup is also where the medium-likelihood `nodeIndexMap`-rebuild bug goes away: there's no separate dedup map to fall out of sync — the reducer is the source of truth for which IDs are present.

What stays out of the reducer (per the host-reducer pattern's "snapshot-and-drop" rule):

- Async I/O — `useHistoryPagination` keeps owning the fetch; it just emits the result as a command instead of calling `setDocument` directly
- xterm scroll position, search highlight state, collapsed/pinned node sets — UI chrome, lives in `documentStateAtom` as today
- Stream parsing — `useAgentStream` keeps the parser; it emits `StreamAppend { nodes }` after parsing

## Where the reducer lives — three options

The reducer logic itself is the same regardless of where it sits. The question is **scope of state owned** and **where the dispatch lives**.

### Option A — View-local, agent-document only (~1 day) ← original spec

`frontend/app/view/agent/document-reducer/` — per-pane state cell created alongside `createAgentAtoms`. Out-of-scope: every other agent-view atom.

Pros: smallest blast radius; doesn't presume a generalization that doesn't yet have a second consumer.
Cons: doesn't match the established frontend pattern (`launcher-event-reducer.ts` lives in `app/store/`); each future per-pane reducer would re-implement the slot/lifecycle plumbing.

### Option B — Store-level, agent-document keyed by blockId (~1 day, recommended)

`frontend/app/store/agent-document-store.ts` — single module owns `Map<blockId, AgentDocumentState>` and exports `dispatchFor(blockId, command)`. Pane registers a slot on mount, releases on cleanup. The per-pane `documentAtom` becomes a projection over the store.

Pros: matches `launcher-event-reducer.ts` exactly (single module, in-memory mirror, atom projection); leaves the door open for cross-pane consumers (e.g. a future "agent activity feed"); centralized event log easier to surface in diagnostics.
Cons: needs slot lifecycle (cleanup on pane close to avoid leak); marginally more wiring than Option A.

### Option C — Store-level + other agent atoms (~2 days)

Same as B but pulls in the other per-pane agent atoms that today have a single owner: `streamingState`, `sessionStats`, `currentTool`, `turnTokens`, `turnActive`, `stopping`. One state cell per pane, all mutations dispatch.

Pros: stronger invariant surface (e.g. `turnActive` ↔ `streamingState.active` cohesion); single audit log for the whole agent-stream lifecycle.
Cons: substantially expands the original "documentAtom only" scope; the other atoms aren't suffering the race that motivated this work.

### Option D — Frontend tab/pane state mirror (~1 week, multi-PR)

Build a frontend reducer mirroring srv reducer's tab/pane/block state shape (per `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`), with agent atoms as slices alongside tab/pane/block. Retrofits the existing wstore-driven atoms into a reducer pattern.

Pros: symmetric with srv; full audit; positions the frontend for receiving srv reducer events directly (Phase E.4 layout reducer convergence).
Cons: big project; needs its own spec; not motivated by today's bug.

### Recommendation

**Option B.** It addresses the architectural concern (matches the established frontend reducer pattern), keeps scope tight (still just `documentAtom`), and the relocation is mechanical from Option A. Option C and D are valuable follow-ups but should not block fixing the live bug.

## Migration plan

### Phase 0 — Immediate-relief patch (ship today, defends users while Phase 1 is in flight)

Single edit in `useAgentStream.ts:366–381`:

```ts
// Before — unconditional wipe
if (fileop === "truncate") {
    setDocument([]);
    return;
}

// After — guarded
if (fileop === "truncate") {
    const phase = sessionPhase();              // new accessor (see Phase 1)
    const elapsedSinceSessionStart = ...;
    if (phase === "active" && elapsedSinceSessionStart > GRACE_MS) {
        log.warn("[agent-stream] suppressing late truncate during active session", {...});
        return;
    }
    setDocument([]);
}
```

Until Phase 1 lands, `sessionPhase()` can be approximated as "is `documentAtom` currently non-empty AND is the WebSocket reachable" — a coarse but effective heuristic. The exact predicate is best refined as part of Phase 1 once the reducer owns phase transitions.

### Phase 1 — Introduce the reducer (single PR)

1. Create `frontend/app/view/agent/document-reducer/` directory:
   - `types.ts` — `AgentDocumentState`, `AgentDocumentCommand`, `AgentDocumentEvent`
   - `reducer.ts` — pure `update(state, command): { state, events }` function
   - `reducer.test.ts` — table-driven tests covering each command + invariant
2. Wire `documentAtom` setter to a single `dispatch(command)` function exported from the reducer module. The setter wraps `update()` and applies the new `state.nodes` to the atom.
3. Refactor `useAgentStream.ts`:
   - On `append` fileop → `dispatch({ type: "StreamAppend", nodes })`
   - On `truncate` fileop → `dispatch({ type: "StreamTruncate", reason: "fileop" })` (reducer decides whether it's honored)
   - Drop the dedup `nodeIndexMap` / `nodeIdSet` — reducer owns dedup
4. Refactor `useHistoryPagination.ts`:
   - On fetch complete → `dispatch({ type: "HistoryLoaded", nodes })`
   - Drop the direct `setDoc([...newNodes, ...prev])` call
5. Refactor `commands/global/clear.ts`:
   - Replace `setDocument([])` with `dispatch({ type: "UserClear" })`
6. Add `SessionStart` / `SessionEnd` dispatch calls at agent-stream lifecycle boundaries.
7. Add a debug accessor `agentDocumentEventsAtom: SignalAtom<AgentDocumentEvent[]>` (ring buffer, last N events) — surfaceable in the diagnostics panel for future bug triage.

### Phase 2 — Audit + remove (follow-up PR)

- grep for any remaining `setDocument(` callsite → reducer or delete
- Add a TS exhaustiveness check: `documentAtom` setter is private to the reducer module; only `dispatch()` is exported
- Remove the Phase 0 inline guard once the reducer's invariant covers it

## Reducer scope assessment (what's NOT in scope)

A common failure mode of reducer migrations is over-reach. **This spec scopes the reducer to the message document only.** Specifically NOT in scope:

- The agent view's other atoms (`streamingStateAtom`, `sessionStatsAtom`, `currentToolAtom`, `stoppingAtom`, `pendingMessagesAtom`) — they have a single owner each and aren't suffering the same race
- xterm/scroll/search/UI-chrome atoms (`documentStateAtom`)
- AgentViewModel class itself
- Any other view (term, swarm, etc.)

If the document reducer proves out, future PRs can selectively pull additional atoms in. Don't pre-design a god-reducer.

## Tests

The reducer being pure is the point — it lets us assert invariants directly:

| Test | Setup | Assertion |
|---|---|---|
| Stream append after history load preserves both | Dispatch `HistoryLoaded([h1,h2])`, then `StreamAppend([s1])` | State has `[h1, h2, s1]` |
| Truncate during active session is suppressed | Dispatch `SessionStart`, `StreamAppend([s1])`, `StreamTruncate` | State still has `[s1]`, event log shows suppression |
| Truncate before session start is honored | Dispatch `StreamTruncate` (no SessionStart yet) | State `nodes === []` |
| Concurrent stream appends with duplicate IDs dedup | Dispatch `StreamAppend([s1, s1])` | State has one `s1` |
| User clear always wipes | Dispatch any sequence ending in `UserClear` | State `nodes === []` |
| History load after stream append prepends correctly | Stream first, then history | History nodes precede stream nodes |
| Reducer is pure | Same input twice → same output | Property test |

## Open questions

| # | Question | Default |
|---|---|---|
| Q1 | Should `SessionStart` be auto-derived from "first stream-append" or an explicit signal from `useAgentStream`? | Explicit signal — the hook knows when subprocess spawn completed |
| Q2 | Truncate suppression window: time-based (GRACE_MS) or event-based (next non-truncate event lifts the suppression)? | Time-based, ~5s — survives socket reconnect (~1–3s typical) without permanently blocking legitimate truncates |
| Q3 | Persist event log across pane sessions, or in-memory ring buffer only? | In-memory ring buffer (200 events); persistence is a future feature if telemetry needs it |
| Q4 | Reducer in `frontend/app/view/agent/document-reducer/` or in a more generic location like `frontend/app/store/`? | View-local — single consumer, follows the per-domain split established by task #182 PR-F-2 |
| Q5 | Are there other agent-view atoms suffering similar race issues that should join this PR? (e.g. `pendingMessagesAtom` is also written by multiple hooks) | Out of scope; surface in a follow-up retro after Phase 1 ships |
| Q6 | Should the diagnostics panel surface the event-log immediately, or in a follow-up? | Follow-up — keep Phase 1 focused |

## Best-practice references

The reducer/event-log pattern for chat UIs is well-trodden:

- **Redux + redux-toolkit** (web standard since ~2015) — pure reducer, action log, time-travel debugging. AgentMux's existing host/launcher reducers are stylistically closer to this than to other patterns.
- **Elm Architecture** — model/msg/update triple, the direct ancestor of Redux. Strongest invariant guarantees because messages are exhaustive sum types (which TypeScript's tagged unions approximate).
- **Solid stores (`createStore`)** — the Solid-native option. Simpler than a reducer but has the same multi-writer race vulnerability `documentAtom` has today; doesn't solve the bug. The reducer is worth the extra layer specifically for the invariant enforcement.
- **CRDT-based chat clients** (Slack, Linear) — overkill for our local-only document but their convergence guarantees are conceptually what we want from the reducer (mutations are commutative within a phase, late-arriving messages can't undo state).

The choice here is **redux-toolkit-style reducer** over Solid store, because the value being added is the invariant ("can't accidentally wipe a live session"), not the state-shape ergonomics. A pure function with a tagged-union command type makes the invariant trivially auditable.

## Estimated effort

- Phase 0 (relief patch): 1–2 hours, single PR, no API change.
- Phase 1 (reducer + refactor): ~1 day. ~200 LOC reducer + ~100 LOC tests + ~3 callsites refactored. Breaking change is internal to `frontend/app/view/agent/`.
- Phase 2 (cleanup): 1–2 hours follow-up.

## Risks

- **Risk:** the reducer adds latency to high-frequency stream appends. **Mitigation:** the reducer is a pure function operating on in-memory arrays, well under 1ms per call (compare to host reducer's sub-ms target). Stream-append rate is bounded by RAF batching in `useAgentStream` (already 60Hz max).
- **Risk:** the truncate-suppression heuristic is wrong — a legitimate user-initiated truncate gets suppressed. **Mitigation:** `UserClear` is a distinct command and is never suppressed. Backend-initiated truncates during active session are arguably always wrong (the existing code's behavior is incidental, not designed); if a legitimate use case surfaces post-Phase-1, add an explicit `BackendTruncate { authoritative: true }` command.
- **Risk:** scope creep into a "redux for the whole agent view" rewrite. **Mitigation:** spec is explicit that scope = `documentAtom` only. Other atoms join only via follow-up PRs with their own justification.
