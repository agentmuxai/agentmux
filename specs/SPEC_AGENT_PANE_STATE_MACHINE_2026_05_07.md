# Spec: Agent Pane State Machine Refinement

**Date:** 2026-05-07  
**Status:** Draft  
**Area:** `frontend/app/store/`, `frontend/app/view/agent/`

---

## Background

The agent pane is already reducer-based — two pure slices handle all state mutations:

- **`agent-document/`** — message list (nodes, sessionPhase, history)
- **`agent-pane-state/`** — lifecycle (streaming, turn, tokens, stopping, pending)

Each slice follows the `update(state, command) → {state, events}` contract: no I/O, immutable updates, typed audit events. This foundation is sound and should be preserved.

However, several gaps were found during analysis:

1. **Pending message durability** — `pending[]` is in-memory; pane unmount loses all queued entries.
2. **No acceptance timeout** — a pending entry can linger indefinitely if the backend never emits `agent-message-accepted`.
3. **Init phase not covered** — `InitState`/`InitQuestion` types exist in `types.ts` but neither reducer handles the init lifecycle.
4. **Stuck-stream recovery** — the only guard is a 1500 ms stop fallback timer; there is no watchdog for streams that stay active with no events for an extended period.
5. **`nodeIdSet` rebuilt on remount** — the dedup index is reconstructed from `nodes[]` on each mount; in-flight duplicates can slip through the gap.
6. **`DocumentState` lives outside reducers** — `collapsedNodes`, `pinnedNodes`, `filter`, and `scrollPosition` are managed via separate local signals; they are invisible to the audit trail and cannot be restored after navigation.
7. **Grace window hardcoded** — `TRUNCATE_GRACE_MS = 5000` is a compile-time constant with no way to extend it under load.

---

## Goals

1. **Close the init-phase gap** — model the init lifecycle as explicit reducer commands so the pane can render correctly from first paint.
2. **Bound pending messages** — add an acceptance timeout with a `PendingMessageExpired` command so stale entries are evicted.
3. **Add a stuck-stream watchdog** — `StreamWatchdogTick` command that transitions a stuck active stream to an error state.
4. **Persist DocumentState in the reducer** — migrate `collapsedNodes`, `pinnedNodes`, `filter`, `scrollPosition` into `agent-document` so they participate in the audit trail and survive navigation.
5. **Expose `nodeIdSet` from the reducer** — store the dedup index in `AgentDocumentState` so remounts skip the rebuild scan.
6. **Make grace window injectable** — accept `nowMs` already; extend to accept `gracePeriodMs` for tests and future tunability.

---

## Current Architecture (What Works — Keep It)

### Two-slice reducer contract

```
update(state, command) → { state: S, events: E[] }
```

- Pure functions: no side effects, no I/O, no async.
- Immutable updates via lazy clone (only clone when a field actually changes).
- Typed discriminated unions for `Command` and `Event`.
- Time injected as `nowMs?: number` for deterministic tests.

**Do not break this contract.** All additions below extend it, not replace it.

### Stream subscription and dispatch loop

`useAgentStream.ts` owns the imperative I/O:

```
subscribe → RAF batch → dispatch to both reducers → emit side-effect events
```

RAF batching coalesces high-frequency stream events before triggering reactive re-renders. Keep this; it is the primary performance guard.

### Stop fallback timer

After `RequestStop`, a 1500 ms `setTimeout` calls `finalizeTurn(null)` if the stream is still active. This guards against a `session_end` that never arrives.

### Pending → accepted visual transition

`agent-message-accepted` WPS event → `PendingMessageAccepted` + `TurnStart` + append `UserMessageNode`. The amber→blue color shift relies on `pending[]` containing the matching entry. This flow is correct; the gap is the missing timeout.

---

## Proposed Additions

### 1. Init phase commands (`agent-pane-state`)

Add to `AgentPaneCommand`:

```ts
| { type: "InitStart" }                             // pane mounted, loading history
| { type: "InitReady" }                             // history loaded, ready for input
| { type: "InitFailed"; reason: string }            // init error (session not found, etc.)
```

Add to `AgentPaneState`:

```ts
initPhase: "loading" | "ready" | "error";
initError?: string;
```

Initial value: `initPhase: "loading"`.

Reducer behavior:
- `InitStart` → reset `initPhase` to `"loading"`, clear `initError`.
- `InitReady` → set `initPhase: "ready"`.
- `InitFailed` → set `initPhase: "error"`, set `initError`.
- Guard: `TurnStart` is suppressed when `initPhase !== "ready"` (same guard as streaming check).

### 2. Pending message acceptance timeout (`agent-pane-state`)

Add to `AgentPaneCommand`:

```ts
| { type: "PendingMessageExpired"; id: string }
```

Add to `AgentPaneEvent`:

```ts
| { type: "pending-expired"; id: string; queuedAt: number; ageMs: number; wasPresent: boolean }
```

Reducer behavior for `PendingMessageExpired`:
- Find entry in `pending[]` by `id`; if not found, emit `pending-expired` with `wasPresent: false` (idempotent — caller may have already processed accept/reject).
- Remove entry; emit `pending-expired` event with `wasPresent: true`.

**Caller responsibility** (`useAgentCommands.ts`):
- After the `AgentInputCommand` RPC is kicked off, schedule a `setTimeout(PENDING_TIMEOUT_MS)` that dispatches `PendingMessageExpired(id)`. Scheduling AFTER the send (not at queue time) means a slow runtime-args metadata update can't pre-empt the timer.
- `PENDING_TIMEOUT_MS = 30_000` (30 s; configurable via `AgentRuntimeConfig`).

### 3. Stuck-stream watchdog (`agent-pane-state`)

Add to `AgentPaneCommand`:

```ts
| { type: "StreamWatchdogTick"; nowMs: number }
```

Add to `AgentPaneState`:

```ts
lastEventMs: number | null;   // updated on every stream event dispatch
```

Add to `AgentPaneEvent`:

```ts
| { type: "stream-stuck"; idleSinceMs: number; thresholdMs: number }
```

Reducer behavior for `StreamWatchdogTick`:
- If `!state.streaming.active` or `lastEventMs == null`, no-op.
- Compute `idleMs = nowMs - lastEventMs`.
- If `idleMs >= STUCK_THRESHOLD_MS` (default 45 000 ms), emit `stream-stuck` event with `idleSinceMs` and `thresholdMs`.

**Note (event-only design):** earlier drafts of this spec proposed a `streamStuckMs: number | null` field on `AgentPaneState`. The implementation chose to skip the persistent state and emit only the event — UI consumers subscribe to the event for the same information without redundant state mutation on every tick. The state listing below reflects this.

**Caller responsibility** (`useAgentStream.ts`):
- On `StreamSubscribe`, start a `setInterval(WATCHDOG_INTERVAL_MS = 5_000)` that dispatches `StreamWatchdogTick(Date.now())`.
- On `StreamUnsubscribe`, clear the interval.
- React to `stream-stuck` event: log a warning, optionally surface an error node.

Every stream event dispatch should also dispatch a lightweight `StreamEventReceived` (or extend existing commands) to update `lastEventMs`. The simplest approach: add `lastEventMs` update as a side effect in the existing `update()` calls that handle stream events (TurnStart, ToolStart, TokensIn, etc.) by adding it to the returned state unconditionally.

### 4. Persist DocumentState in reducer (`agent-document`)

Move the following from local signals into `AgentDocumentState`:

```ts
collapsedNodes: Set<string>;   // node IDs that are collapsed
pinnedNodes: Set<string>;      // node IDs that are pinned
filter: string;                // active search/filter string
scrollPosition: number;        // px from top, snapshotted on blur
```

Add to `AgentDocumentCommand`:

```ts
| { type: "NodeToggleCollapsed"; nodeId: string }
| { type: "NodeTogglePinned"; nodeId: string }
| { type: "FilterChanged"; filter: string }
| { type: "ScrollPositionSaved"; position: number }
```

No events needed for these (they are purely view state; no audit value).

Reducer behavior: straightforward toggling / assignment; lazy clone.

**Why this matters:** Navigating away and back to the agent pane currently resets all collapsed/pinned state. With this in the reducer, the store persists across pane mount/unmount (the store is owned by the parent, not the view).

### 5. Expose `nodeIdSet` from `AgentDocumentState`

Currently `useAgentStream.ts` rebuilds a local `nodeIdSet` on each remount by scanning `nodes[]`. If a duplicate arrives in the gap before the set is populated, it is inserted twice.

Add to `AgentDocumentState`:

```ts
nodeIdSet: Set<string>;   // maintained by reducer; always in sync with nodes[]
```

Reducer responsibility:
- On every `StreamFlush` that adds a node: `nodeIdSet.add(nodeId)`.
- On `UserClear`: `nodeIdSet = new Set()`.
- On `SessionStart`/`HistoryLoaded`: populate from the incoming nodes.

**Caller change:** Remove the local `nodeIdSet` rebuild in `useAgentStream.ts`; read `state.nodeIdSet` instead.

### 6. Injectable grace period (`agent-document`)

Current signature:

```ts
update(state, command, nowMs?: number)
```

Extended signature:

```ts
update(state, command, nowMs?: number, opts?: { truncateGraceMs?: number })
```

`shouldSuppressTruncate` reads `opts?.truncateGraceMs ?? TRUNCATE_GRACE_MS`.

Callers in tests can pass `{ truncateGraceMs: 0 }` to disable suppression. The production caller passes nothing (defaults to 5000 ms). If future load testing shows 5 s is too short, the value can be passed in from `AgentRuntimeConfig` without a code change.

---

## Command / Event Type Additions Summary

### `agent-pane-state/types.ts`

**State additions:**
```ts
initPhase: "loading" | "ready" | "error";
initError?: string;
lastEventMs: number | null;
// `streamStuckMs` was in an earlier draft but the implementation
// emits the stuck-stream signal as an event only (see §3 above).
```

**New commands:**
```ts
| { type: "InitStart" }
| { type: "InitReady" }
| { type: "InitFailed"; reason: string }
| { type: "PendingMessageExpired"; id: string }
| { type: "StreamWatchdogTick"; nowMs: number }
```

**New events:**
```ts
| { type: "pending-expired"; id: string; queuedAt: number; ageMs: number; wasPresent: boolean }
| { type: "stream-stuck"; idleSinceMs: number; thresholdMs: number }
```

### `agent-document/types.ts`

**State additions:**
```ts
nodeIdSet: Set<string>;
collapsedNodes: Set<string>;
pinnedNodes: Set<string>;
filter: string;
scrollPosition: number;
```

**New commands:**
```ts
| { type: "NodeToggleCollapsed"; nodeId: string }
| { type: "NodeTogglePinned"; nodeId: string }
| { type: "FilterChanged"; filter: string }
| { type: "ScrollPositionSaved"; position: number }
```

**Signature change:**
```ts
update(state, command, nowMs?: number, opts?: { truncateGraceMs?: number })
```

---

## Non-Goals

- **Sorting / filtering the document list** — `FilterChanged` stores the string; actual filtering is a `createMemo` in the view, not a reducer concern.
- **Undo / redo** — not requested; no command history stack.
- **Cross-pane state sharing** — each pane owns its own store instance; sharing is out of scope.
- **Persistence to disk** — pending queue and document state live in memory. Disk persistence would require a separate serialization layer; defer.
- **Changing the RAF batching strategy** — the current approach works; leave it alone.

---

## Implementation Order

1. **Add `nodeIdSet` to `AgentDocumentState`** — pure reducer change, no caller changes needed except removing the local rebuild in `useAgentStream.ts`. Lowest risk.

2. **Injectable grace period** — one-line signature change + update `shouldSuppressTruncate`. Update existing tests.

3. **Init phase commands** — add `initPhase` to state, add three commands, add guard in `TurnStart`. Wire `InitStart`/`InitReady`/`InitFailed` dispatch in `useAgentStream.ts` around history load.

4. **Pending message timeout** — add `PendingMessageExpired` command + event. Wire `setTimeout` in `useAgentCommands.ts` on `PendingMessageQueued`.

5. **DocumentState migration** — add four commands, migrate callers from local signals. Most impactful for UX (collapse/pin persistence); medium risk.

6. **Stuck-stream watchdog** — add `StreamWatchdogTick`, `lastEventMs`, watchdog interval in `useAgentStream.ts`. Add `lastEventMs` update to all stream-event commands.

---

## Testing Guidance

All reducer additions are pure functions and should have unit tests:

```ts
// agent-pane-state reducer
it("suppresses TurnStart when initPhase is loading")
it("PendingMessageExpired removes entry and emits event")
it("PendingMessageExpired emits wasPresent:false when id not found")
it("StreamWatchdogTick emits stream-stuck when idle >= threshold")
it("StreamWatchdogTick is a no-op when stream not active")

// agent-document reducer
it("StreamFlush updates nodeIdSet")
it("UserClear resets nodeIdSet")
it("NodeToggleCollapsed toggles membership in collapsedNodes")
it("StreamTruncate respects injected gracePeriodMs")
```

Integration tests (`app.e2e.test.ts`) should cover:
- Send message → pending entry appears → accepted → disappears within 30 s
- Stop agent → finalizeTurn fires within 1500 ms fallback
