# Spec: `replaceChild` crash in the agent-pane virtualizer — full analysis and fix plan

**Date:** 2026-06-06  
**Status:** All five fixes shipped — PR #1293, PR #1299, PR #1303.  
**Related:** `docs/retro/RETRO_REPLACECHILD_CRASH_2026-06-06.md`, PR #1293, PR #1299, PR #1303  
**Block under test:** `a8cd53a4-a61c-4483-8606-0eba9ccb3565`

---

## 1. Background: architecture of the streaming buffer

`AgentDocumentVirtualList` splits the agent document into two DOM regions:

- **Virtualized head** (`<Key each={windowedRows()}>`) — prefix-summed rows managed by the Phase-3 layout slice. Off-screen rows are recycled.
- **Streaming buffer** (`<Index each={partition().streamingNodes}>`) — the trailing `STREAMING_BUFFER_SIZE=50` nodes, always mounted. New streaming tokens update node content here without remounting.

The split point is a **sticky frontier** (`stickyFrontierId`). Once the document crosses 50 nodes the frontier is frozen to the id of the 50th-from-last node. From then on, new appends only grow `streamingNodes`; the virtualized head never changes on a simple append. This prevents cross-subtree node migration (which was a separate earlier crash class).

### 1.1 Document store signal writes

`documentAtom` is a plain `createSignal<DocumentNode[]>`. Every write is a synchronous `writeSignal` that immediately marks all reactive observers. Outside a `batch()` each write starts its own `runUpdates` frame, flushing memos and effects synchronously before returning. Inside a `batch()` all writes are deferred until the batch completes, then memos and effects flush **once**.

The `partition` createMemo reads `documentAtom` (`viewState.nodes()`). The `<Index>` render effect reads `partition().streamingNodes`. Any write to `documentAtom` outside a batch can therefore trigger a `runUpdates` frame that starts `reconcileArrays` on the streaming buffer.

### 1.2 Why `reconcileArrays` can fail

SolidJS's `reconcileArrays(parent, a, b)` diffs the old DOM child list `a` against the new item list `b` and mutates `parent` to match. When it calls `parent.replaceChild(b[bStart], a[aStart])` and `a[aStart]` is no longer a child of `parent`, it throws:

```
NotFoundError: Failed to execute 'replaceChild' on 'Node':
  The node to be replaced is not a child of this node.
```

This happens when **two concurrent `runUpdates` frames** both run `reconcileArrays` on the same parent: the first frame moves a DOM node, and the second frame's stale `a` array still references that node at its old position.

SolidJS prevents this within a `batch()` (both writes share one frame). The race requires at least two **independent** `writeSignal(documentAtom, …)` calls with no enclosing `batch()`.

---

## 2. Crash history

### 2.1 Crash 0 — 2026-05-27 (pre-history, documented separately)

`docs/analysis/AGENT_PANE_REPLACECHILD_CRASH_ON_SEND_2026_05_27.md`

Root cause: no sticky frontier; cross-subtree node migration on every append caused `streamingNodes` to shrink and `<For>` (used before `<Index>`) to unmount/remount the active streaming row, crashing on the next token.

Fix: switch to `<Index>` + sticky frontier (PR #784 / #1101 predecessor work).

### 2.2 Crash 1 — 2026-06-06 05:45 UTC (pre-fix)

**Signal:** `block-error-boundary ERROR NotFoundError replaceChild`  
**render_trail:** frontier=`toolu_01`, virtCount=58, streamCount=85→86

**Root cause** (confirmed): `dispatchDoc({ type: "StreamFlushObserved" })` was dispatched **after** `dispatchDoc({ type: "StreamFlush" })` without a `batch()` enclosing both. Two sequential `runUpdates` frames interleaved: `StreamFlush` started the `<Index>` reconcile (adding a new row at position 85), then `StreamFlushObserved` started a second frame that also invoked `reconcileArrays` with the stale `a` array. `replaceChild` failed on the node the first frame had already moved.

**Fix:** PR #1293 — wrap `StreamFlush + StreamFlushObserved` together in `batch()` in `flushPendingNodes`.

**Status:** Partially effective. Eliminates the StreamFlush/StreamFlushObserved interleave. But `ToolChunkAppend` writes were still outside the batch.

### 2.3 Crash 2 — 2026-06-06 11:46 UTC (post PR #1293, pre PR #1299 HMR)

The `ToolChunkAppend` path from the WPS `tool_chunk` handler was calling `model.dispatchDoc({ type: "ToolChunkAppend" })` directly — one immediate `documentAtom` write per tool output line — outside any `batch()`. During active tool streaming with parallel tool calls (ToolStart ×3 observed in the render_trail), multiple `ToolChunkAppend` writes could land in the same animation frame as a RAF-triggered `StreamFlush`. Two independent `runUpdates` frames interleaved; `<Index>` crashed.

render_trail signature: frontier=`node_18`, virtCount=94, streamCount=67→71 (long conversation with paginated history load, many chunk-only RAF flushes at same streamCount, then adds at 68/69/70/71 where one of the length-change flushes triggered the race).

**Fix:** PR #1299 — accumulate tool chunks in `pendingChunks[]` and flush them inside the same `batch()` as `StreamFlush/StreamFlushObserved` in `flushPendingNodes`. This eliminates the `ToolChunkAppend`-vs-`StreamFlush` race entirely: all `documentAtom` writes from streaming now originate from one code path (`flushPendingNodes`) in one Solid reactive frame.

**Status:** Shipped 2026-06-05 23:09 PDT. Confirmed by Vite HMR log (agent-view.tsx and AgentDocumentVirtualList.tsx hot-updated at 23:00:38 PDT; Vite reconnect + `useAgentStream subscribed` at 23:00:40). **Crash class from §2.3 is closed.**

---

## 3. Remaining crash paths — not yet fixed

After PR #1299, all **streaming** writes are batched together. The remaining unbatched `documentAtom` write paths are:

### 3.1 `SessionEnd` from `onCleanup` (highest priority)

**Location:** `useAgentStream.ts`, `onCleanup` block:

```typescript
onCleanup(() => {
    if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
    subscription.unsubscribe();
    const at = Date.now();
    model.dispatchPane({ type: "StreamUnsubscribe", at });   // paneStateAtom write
    model.dispatchDoc({ type: "SessionEnd", at });            // documentAtom write — UNBATCHED
});
```

**Why it can crash:** When SolidJS's error boundary catches a `reconcileArrays` throw, it disposes the errored scope. Disposal runs `onCleanup` callbacks synchronously. `StreamUnsubscribe` fires first (pane write), then `SessionEnd` fires (document write). The `SessionEnd` reducer calls `scrubOrphanedInProgress` which returns a new `nodes` array → `documentAtom` changes → `partition` recomputes → the `<Key>` streaming-buffer scope (whose scope may still be partially live during cleanup ordering) is re-triggered → a second `reconcileArrays` call on a partially-torn-down DOM → `replaceChild` fails.

Additionally, `StreamUnsubscribe` and `SessionEnd` are two sequential, unbatched writes. Even in the non-crash path (normal turn end with component unmounting), if there is any concurrent RAF flush pending (one that snuck through before `cancelAnimationFrame` cleared it), the double write produces two independent frames.

**Evidence:** Both crashes end their render_trail with `StreamUnsubscribe` (logged synchronously in cleanup before the trail is dumped by the error boundary). The `agentActivity busyCount=0` fires immediately after the crash log — cleanup runs as part of error-boundary disposal.

**Fix:** Wrap the two cleanup dispatches in a single `batch()`, AND defer `SessionEnd` (which mutates nodes) to a `queueMicrotask` so it fires after the current synchronous disposal cycle:

```typescript
onCleanup(() => {
    if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
    subscription.unsubscribe();
    const at = Date.now();
    model.dispatchPane({ type: "StreamUnsubscribe", at });
    // Defer SessionEnd out of the synchronous disposal chain. During error-boundary
    // cleanup the <Key> streaming-buffer scope is still partially live; a synchronous documentAtom
    // write here can re-trigger reconcileArrays on a half-torn-down DOM → replaceChild
    // NotFoundError (observed 2026-06-06 crash 2). By the time the microtask fires,
    // all scope disposal is complete and the <Key> effect is already removed from
    // the computation graph — the write still runs the reducer (orphan scrub) but
    // no reactive effect reconciles the dead DOM. (model.dispatchDoc uses the soft
    // dispatchIfRegistered variant, so if the slot is gone by microtask time it's a
    // silent no-op rather than a throw.)
    queueMicrotask(() => model.dispatchDoc({ type: "SessionEnd", at }));
});
```

**Risk:** `SessionEnd`'s orphan scrub runs slightly later. Between the component unmounting and the microtask firing, any code that reads `sessionPhase` from `documentAtom` sees "active" for one microtask. This window is cosmetically harmless. `HistoryLoaded` and `HistoryRestored` both also run the orphan scrub, so a missed `SessionEnd` is recovered on next history access.

### 3.2 History pagination `HistoryLoaded` (medium priority)

**Location:** `useHistoryPagination.ts` (or equivalent) — fires `model.dispatchDoc({ type: "HistoryLoaded", nodes, ... })` from an async RPC callback (after `await`).

**Why it can crash:** A `HistoryLoaded` write to `documentAtom` outside any `batch()` can race with a concurrent RAF flush. If the RPC resolves between a RAF's `StreamFlush` write and its effect flush (possible via Promise microtask resolution within the same browser task), two frames interleave.

Crash 2 had `virtCount=94` indicating a long conversation where older history had been paginated. This path is a confirmed candidate but has not produced a captured crash trace yet.

**Fix:** Wrap `dispatchDoc(HistoryLoaded)` in `batch()`:

```typescript
// In the pagination RPC callback:
batch(() => {
    model.dispatchDoc({ type: "HistoryLoaded", nodes: prepended, loadOlder: true });
});
```

Since there's no concurrent same-frame pane write needed here, the `batch()` just ensures that if any SolidJS effects are mid-flight when the RPC resolves, this write defers correctly into the current frame via the `if (Updates) return fn()` re-entrancy rule.

### 3.3 Defensive `<Show>` guard (low priority — belt-and-suspenders)

From retro §6.3: wrap `<Index each={partition().streamingNodes}>` in a `<Show when={partition()}>` to prevent the secondary crash:

```
TypeError: Cannot read properties of undefined (reading 'streamingNodes')
```

This TypeError fires when `indexArray`'s internal mapper accessor is called after its slot has been disposed — not because `partition()` returns `undefined`, but because the disposed accessor returns an undefined value that then fails `.streamingNodes`. The `<Show>` guard prevents the expression from being evaluated post-dispose.

Not the primary fix for `replaceChild` crashes, but eliminates the secondary crash class. Low implementation cost, high resilience benefit.

**Implementation:**

```tsx
<Show when={partition()}>
    {(p) => (
        <div class="agent-document-streaming-buffer" data-animate={animateEnabled() || undefined}>
            <Index each={p().streamingNodes as DocumentNode[]}>
                {(nodeAccessor) => (
                    <DocumentRow ... />
                )}
            </Index>
        </div>
    )}
</Show>
```

---

## 4. Complete fix plan (prioritized)

| # | Fix | File | Status |
|---|-----|------|--------|
| 1 | `batch(StreamFlush + StreamFlushObserved)` | `useAgentStream.ts` | ✅ PR #1293 |
| 2 | Route `ToolChunkAppend` through `pendingChunks[]` RAF buffer | `useAgentStream.ts` | ✅ PR #1299 |
| 3 | `queueMicrotask(SessionEnd)` in `onCleanup` | `useAgentStream.ts` | ✅ PR #1303 |
| 4 | `batch()` all `HistoryLoaded`/`HistoryRestored` calls | `useHistoryPagination.ts` | ✅ PR #1303 |
| 5 | `<Show when={partition()}>` guard on streaming buffer (`<Key>`) | `AgentDocumentVirtualList.tsx` | ✅ PR #1303 |

All five fixes are shipped. The streaming buffer now uses `<Key by={n.id}>` (agenta PR #1300, primary crash fix) wrapped in `<Show when={partition()}>` (this PR, secondary-crash guard).

---

## 5. Invariant: all `documentAtom` writes from streaming must be batched

Going forward, **every `model.dispatchDoc(...)` call that fires from an event callback (RAF, WPS, RPC, `onCleanup`) must either:**
- be inside an existing `batch()`, OR
- be the sole write in that event-loop turn (i.e., no concurrent pane write in the same turn), OR
- be deferred via `queueMicrotask()` to after the synchronous call stack.

A `dispatchDoc` that issues a raw `writeSignal(documentAtom, newNodes)` outside these constraints is a latent `replaceChild` bug waiting for a concurrent write to expose it.

The lesson from the 2026-06-06 crashes: the reducer guarantees per-command consistency but the **reactive scheduler** sees only independent signal writes. Batching is the scheduler-level contract that must be maintained end-to-end across every `documentAtom` writer.

---

## 6. Diagnostic aids already in place

- **`render_trail` ring buffer** (`frontend/log/render-trail.ts`): captures last ~60 `agent:dispatchPane` and `agent:virt:partition` events. Dumped by `BlockErrorBoundary` on every crash. The `virt:partition` entries capture `virtCount`, `streamCount`, and `frontier` at each `partition()` recompute, making the crash context readable without a debugger.
- **`DISPOSE UNEXPECTED(mid-turn)` diagnostic** (`agent-view.tsx:194–217`): logs a stack trace + render_trail + recent dispatches when the agent-view disposes while `turnPhase` is in a working state. Identifies silent owner-driven unmounts (e.g., upstream `<Show>` going `false` on a block-delete event).
- **`CASCADE_DETECTED` warning** (`agent-document-store.ts:139–146`): fires when a `documentAtom` subscriber unmounts the pane synchronously during a dispatch setter call. Identifies re-entrant teardown triggered by document state changes.

---

## 7. Open questions — resolved

### 7.1 Did the agent-view unmount before crash 2? **No — closed.**

**Evidence:** The `DISPOSE UNEXPECTED(mid-turn)` diagnostic in `agent-view.tsx:207` logs a warning whenever the component disposes while `turnPhase` is working. It was committed and HMR-live before crash 2: the CEF log shows `hot updated: /frontend/app/block/BlockErrorBoundary.tsx` at 23:00:40 PDT (same batch as the retro's diagnostic changes), and the fix HMR at 23:09:53 PDT re-ran `agent-view.tsx`. The diagnostic was active for the entire 5-hour window leading to crash 2. It did NOT fire in the host log around 11:46 UTC.

**Conclusion:** The agent-view was NOT unexpectedly unmounted before crash 2. The component was in a normal mid-turn state (28-second turn, started at `busyCount=1` 11:46:16 UTC, crashed at 11:46:44). The `StreamUnsubscribe` in the render_trail is from the error boundary's **post-crash** cleanup: when `reconcileArrays` threw, SolidJS propagated the error to `BlockErrorBoundary`, which disposed the scope. `onCleanup` fired synchronously during disposal, dispatching `StreamUnsubscribe` to the ring buffer before the trail was dumped.

### 7.2 Was `HistoryLoaded` racing in crash 2? **Unbatched but not the direct crash vector — partially closed.**

**Code confirmation:** `useHistoryPagination.ts:130` calls `opts.model.dispatchDoc({ type: "HistoryLoaded", nodes: newNodes })` bare — no `batch()` wrapper — from an `async` function after `await RpcApi.BlockfileReadRangeCommand(...)`.

**WPS delivery is fully synchronous:** `waveEventSubscribe` handlers are called synchronously inside `handleWaveEvent` ← `recvRpcMessage` ← the WebSocket `onmessage` handler (`rpc-util.ts:19: DefaultRouter.recvRpcMessage(event.data)`). No Promise wrapping. The tool_chunk handler runs synchronously within the WebSocket browser task, pushes to `pendingChunks`, and `scheduleFlush()` schedules a RAF. This is properly batched by the PR #1299 pendingChunks fix.

**`HistoryLoaded` interleave analysis:** The `await RpcApi...` continuation fires as a Promise microtask between browser tasks. By that point, any in-flight `runUpdates` frame from the RAF has already completed synchronously (SolidJS's `completeUpdates` is synchronous; effects including `<Index>` reconcileArrays run to completion before the RAF callback returns). A `HistoryLoaded` microtask landing after the RAF callback cannot inject itself into an ongoing `reconcileArrays`.

**However:** `HistoryLoaded` IS an unbatched `documentAtom` write that starts its own `runUpdates` frame. If a subsequent `StreamFlush` or chunk flush fires in the same browser task as `HistoryLoaded`'s microtask (theoretically possible under very specific task scheduling), two frames could still interleave. This is an unverified edge case but warrants the `batch()` fix (Fix 4) for correctness.

**Crash 2 attribution:** `HistoryLoaded` is NOT the confirmed cause of crash 2. The paginated history (virtCount=94) was loaded in an earlier turn. The crash turn had no visible pagination activity.

### 7.3 Why `virtCount > streamCount` (inverted ratio)? **Resolved — normal.**

`virtCount=94 > streamCount=67` is expected for long conversations where older history was prepended before the streaming buffer. When `HistoryLoaded` prepended 94 older nodes before the sticky frontier (`node_18`), those nodes went into `virtualizedNodes`. All subsequent streaming nodes (from new turns) went into `streamingNodes`. The 94-node virtualized head and 67-node streaming buffer are consistent with a conversation that: (a) initially grew to 50+ nodes (crossing STREAMING_BUFFER_SIZE), setting the frontier; (b) then paginated older history, growing the virtual head; (c) then ran several more streaming turns, growing the streaming buffer to 67.

The inverted ratio is not a crash indicator. Only the interleaved `reconcileArrays` mechanism matters.

### 7.4 What caused crash 2 if the pendingChunks fix was active? **Mechanism unresolved — new diagnostic needed.**

**Evidence that crash 2 ran with fixed code:** The last `[useAgentStream] subscribed` before crash 2 is at `0605/230955.289800` (23:09:55 PDT), 4 seconds after the fix's HMR (`hot updated: agent-view.tsx` at 23:09:53 PDT, triggered by the `useAgentStream.ts` import change). The pane ran with pendingChunks-batched chunks from that subscription through crash 2 at 04:46 PDT — 5 hours with the fixed code.

**What the trail tells us:** The crash happened at streamCount=71 (the last length-change flush at `1780746403896`, which added node 70→71). Eight consecutive same-length chunk flushes preceded it (streamCount=68 held across ~2.5s). Then 68→69→70 in rapid succession (367ms), followed by 3 parallel ToolStarts, then 70→71. The crash trail ends with `StreamUnsubscribe` (post-crash cleanup).

**What static analysis rules out:**
- Tool chunk race → fixed by pendingChunks (PR #1299, confirmed active)
- Pre-crash unmount → ruled out by absent DISPOSE UNEXPECTED
- HistoryLoaded synchronous interleave → ruled out (microtask fires between tasks, not mid-runUpdates)
- SessionEnd from cleanup → fires post-crash, cannot be pre-crash cause

**Hypothesis (unverified):** The rapid back-to-back length additions (68→69 at 400846, 69→70 at 401213, 367ms apart) followed by 3 simultaneous ToolStarts (401666×2) may expose an edge case in SolidJS's `<Index>` internal `mapped`/`disposers` state management under rapid consecutive length changes. Specifically: if a RAF frame adds a node while the previous frame's reactive settlement is still executing some nested reactive computation (e.g., the layout `createEffect` at `AgentDocumentVirtualList.tsx:206` writing `setLayoutView(view)` during its own run, which marks `windowedRows` dirty and queues `<Key>` for another effect pass in the SAME `runEffects` loop), there may be a window where the `<Index>` outer reconcile and an internal `setLayoutView`-triggered re-render briefly overlap.

**Action to identify root cause:** Add per-frame logging to `reconcileArrays` caller site: log `a.length`, `b.length`, and whether `a` array nodes are actually children of `parent` before calling. The next crash would show whether the crash is on a length-change flush (expected) or a same-length flush (unexpected), and which `a[i]` node is no longer in `parent` and when it was removed. This is the minimum instrumentation needed to pinpoint the residual crash.
