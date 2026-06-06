# Retro — `replaceChild` crash in the agent pane virtualizer

**Date:** 2026-06-06  
**Severity:** Hard blocker — agent pane blanks mid-conversation, repeatable  
**Status:** Root cause confirmed; fix required (see §6)  
**Related:** `docs/analysis/AGENT_PANE_REPLACECHILD_CRASH_ON_SEND_2026_05_27.md`, PR #1293

---

## 1. What the user experienced

After 1–3 turns in an agent pane conversation the pane goes blank. The "Reload pane" fallback appeared (sometimes), or the pane just stayed dead. The agent's backend subprocess kept running — the conversation was preserved — but the UI was gone.

The crash is **100% reproducible** once a conversation accumulates enough nodes (observed at `streamCount` 53–57, later at 85–86 after the partial fix).

---

## 2. Observed crash signatures

**Primary crash** — every time, same stack:
```
NotFoundError: Failed to execute 'replaceChild' on 'Node':
  The node to be replaced is not a child of this node.
  at reconcileArrays  (solid-js/web/dist/dev.js:185)
  at insertExpression (solid-js/web/dist/dev.js:601)     ← outer
  at Object.fn       (solid-js/web/dist/dev.js:576)
  at runComputation → updateComputation → createRenderEffect
  at insertExpression (solid-js/web/dist/dev.js:573)     ← inner
  at Object.fn       (solid-js/web/dist/dev.js:334)
  at runComputation → updateComputation
```

Two nested `insertExpression` levels: an outer array reconcile (`<Index>`) and an inner component child update (`DocumentRow`'s `props.node()` update).

**Secondary crash** (same session, different manifestation):
```
TypeError: Cannot read properties of undefined (reading 'streamingNodes')
```
This is NOT `partition()` returning `undefined` — see §4.3.

**render_trail at crash time:** `virtCount` never changes (no cross-subtree migration). `streamCount` monotonically grows to 85–86 with the same `frontier=toolu_01` frozen. No `splitIndex=-1` re-anchor fires.

---

## 3. How the reducer architecture connects

### 3.1 `documentAtom` is a plain Solid signal

`documentAtom` is created as `createSignal<DocumentNode[]>([])` (`state.ts:118`). It is **not** a `createStore`. Writing to it via the signal setter (`documentAtom[1](newNodes)`) is a **synchronous `writeSignal`** call that immediately queues all reactive observers.

### 3.2 The reducer always returns a new array

`StreamFlush` in the agent-document reducer (`reducer.ts:305–396`) uses a lazy-clone pattern: `ensureClone()` → `state.nodes.slice()` + appends. Every flush that adds at least one node produces a **new `DocumentNode[]` reference**, causing `documentAtom`'s equality check to see a change and notify all observers.

### 3.3 Multiple independent write paths — not serialized

`documentAtom` is written from at least **three independent code paths**:

| Path | Trigger | Batched? |
|---|---|---|
| `flushPendingNodes` → `dispatchDoc(StreamFlush)` | `requestAnimationFrame` | After PR #1293: yes |
| `dispatchDoc({ type: "ToolChunkAppend" })` | `tool_chunk` WPS event handler | **No** |
| History pagination restore | RPC callback | No |

`ToolChunkAppend` is dispatched directly from the WPS event handler (`useAgentStream.ts:136–158`) outside of `flushPendingNodes`. **Each `ToolChunkAppend` is a raw `model.dispatchDoc()` call that immediately writes `documentAtom`**, triggering its own reactive flush. During active streaming, a tool chunk event and a RAF flush can both fire in the same browser task, or the tool_chunk handler can fire between a `StreamFlush` write and a `StreamFlushObserved` write.

This is the architectural gap: the reducer ensures each individual document transition is consistent, but it does not coordinate how the resulting signal writes interact with Solid's reactive scheduler.

---

## 4. The crash mechanism — step by step

### 4.1 Solid's reactive scheduler semantics (relevant internals)

- `batch(fn)` = `runUpdates(fn, false)`. When `Updates === null` (normal synchronous context), this creates a fresh update queue, runs `fn`, then flushes memos then effects.
- **Critical re-entrancy rule** (`solid.js:793`): `if (Updates) return fn()`. If a signal write happens while `Updates` is already non-null (i.e. inside an ongoing `runUpdates`), the write's observers are enqueued into the existing queue rather than starting a new flush — they run when the *outer* flush completes, not immediately.
- `createRenderEffect` and `createEffect` **both** go into the `Effects` queue (both are `pure=false`). Both **are** deferred by `batch()`. The common belief that `createRenderEffect` is synchronous and not deferrable by `batch()` is **wrong** — reagent confirmed the `batch()` fix is sound.
- However: if `batch()` is entered when `Updates` is already non-null (nested batch), the inner contents run immediately via the `if (Updates) return fn()` short-circuit.

### 4.2 The pre-fix crash path

Without `batch()` in `flushPendingNodes`:

```
[RAF callback — Updates === null]
  dispatchDoc(StreamFlush)
    → writeSignal(documentAtom, newNodes)
    → runUpdates(fn, false)           // Updates = [] created
      → partition memo queued into Updates
      → <Index> createRenderEffect queued into Effects
    → completeUpdates()
      → runQueue(Updates): partition memo runs synchronously
      → runEffects(Effects): <Index> createRenderEffect starts —
          reconcileArrays begins, sets `current = newArray`
          starts inserting new DOM rows…

  [INTERLEAVE: <Index> reconcile is mid-flight]

  dispatchPane(StreamFlushObserved)
    → writeSignal(streamingStateAtom, ...)
    → runUpdates(fn, false)           // Updates = [] created (new frame)
      → dependents of streamingStateAtom queued
    → completeUpdates()
      → some effect runs that may invalidate `current` in the first
        insertExpression closure, OR
      → a concurrent ToolChunkAppend from a WPS event fires here,
        writing documentAtom AGAIN and resetting `streamingNodes`

  [back in the first reconcile pass]
  reconcileArrays tries: parent.replaceChild(newNode, oldNode)
  → oldNode was just removed by the interleaved update → NotFoundError
```

### 4.3 The secondary crash (`streamingNodes` undefined)

This is NOT `partition()` returning `undefined`. It is Solid's `indexArray` mapper (the internal engine behind `<Index>`) returning a signal accessor over an already-disposed slot. When `indexArray` shrinks (or sees a different length), it calls `disposers[i]()` for removed positions. If this disposal happens while the outer `insertExpression` still holds a reference to those DOM nodes in its `current` array, a subsequent `replaceChild` call fails — and the error, as it propagates through the reactive computation chain to the `ErrorBoundary`, materialises as a `TypeError` reading a property of a now-disposed signal value.

### 4.4 Why `batch()` only partially fixed it

The `batch()` added in PR #1293 correctly serializes `StreamFlush` + `StreamFlushObserved` — those two writes now see a single `runUpdates` frame and their effects flush together. The `replaceChild` crash from **those two writes interleaving** is fixed.

But `ToolChunkAppend` dispatches (tool streaming output — one per tool output line) fire from the WPS `tool_chunk` event handler **outside** `flushPendingNodes` and outside any `batch()`. During active streaming with tool calls, `ToolChunkAppend` and `StreamFlush` can race: a WPS event fires between a `StreamFlush` write and a `StreamFlushObserved` write (they're in the same `batch()` now so that specific race is gone), or a `ToolChunkAppend` fires in the same animation frame as a later RAF-triggered `StreamFlush`.

The crashes at `streamCount=85–86` (after the `batch()` fix) were driven by `ToolChunkAppend` races, not the `StreamFlush`/`StreamFlushObserved` interleave. The `render_trail` shows `virtCount=58` frozen and `streamCount` climbing from 50 to 86 — many `ToolChunkAppend` dispatches each writing `documentAtom` independently, each triggering its own `runUpdates` frame, each with the same `<Index>` reconcile interleave risk.

---

## 5. What the sticky frontier does and doesn't protect

The sticky frontier (`stickyFrontierId`) was introduced to prevent a specific class of crash: a node appearing in **both** the `<Key>` virtualized head and the `<Index>` streaming buffer simultaneously during a reactive tick. It achieves that — `virtCount` never changes during streaming, confirming no cross-subtree migration occurs.

What it does **not** protect against: the interleaved-reconcile DOM race described in §4. The sticky frontier ensures the *content* of `streamingNodes` only grows (never shrinks mid-turn, never reorders). But `<Index>` reconciling a growing array is still subject to the concurrent write issue if another signal write fires mid-reconcile.

The `streamCount > STREAMING_BUFFER_SIZE` growth was correctly identified as a necessary condition for observing the crash (more streaming buffer nodes = more reconcile work = longer window for a race), but the growth itself is not the **cause**. Adding a buffer cap would reduce the probability of the crash but not eliminate it — a `ToolChunkAppend` and a `StreamFlush` can still interleave even with a small streaming buffer.

---

## 6. The complete fix

**Scope:** Every `dispatchDoc(...)` call that writes `documentAtom` must be batched with any concurrent writes that could trigger a competing reactive flush during the same event-loop turn.

### 6.1 Already fixed (PR #1293)
`StreamFlush` + `StreamFlushObserved` wrapped in `batch()` in `flushPendingNodes` (`useAgentStream.ts:167–203`).

### 6.2 Still needed
**Wrap `ToolChunkAppend` dispatches in `batch()`** (`useAgentStream.ts:136–158`). Each `ToolChunkAppend` call is currently a standalone `model.dispatchDoc()` write. These should either:
- Be wrapped individually in `batch()` so their Effects flush doesn't interleave with an in-progress `flushPendingNodes` flush, OR
- Be deferred into `pendingUpdates` and flushed via the existing RAF path (i.e. convert `ToolChunkAppend` to the update-accumulation pattern already used by other streaming events).

The second option (route `ToolChunkAppend` through the RAF `pendingUpdates` buffer) is architecturally cleaner: it means all `documentAtom` writes from streaming originate from a single code path (`flushPendingNodes`) with a single `batch()` wrapper, rather than two independent writers that need independent batching.

### 6.3 Defensive addition
Wrap `<Index each={partition().streamingNodes}>` in a `<Show when={partition()}>` guard (`AgentDocumentVirtualList.tsx:700`) so a racing `partition()` read after a dispose doesn't propagate as a crash. This is belt-and-suspenders, not the primary fix.

---

## 7. Lessons for future document-store writes

1. **Every `dispatchDoc` that fires from an event callback (RAF, WPS, RPC) should be `batch()`-wrapped** unless it is the *only* write in that event turn. Solid's signal writes are synchronous and immediate; two writes in sequence without `batch()` = two separate reactive flushes = possible interleave.

2. **A reducer guarantee is not a reactive scheduler guarantee.** The document reducer ensures each `StreamFlush` transition is internally consistent (new array, correct content). But the *scheduler* doesn't know that `StreamFlush` and `ToolChunkAppend` are logically separate phases — it just sees two signal writes. The fix must live at the scheduler boundary, not the reducer.

3. **`createRenderEffect` IS deferrable by `batch()`.** A persistent misconception caused several review cycles. Solid defers both `createEffect` and `createRenderEffect` until `completeUpdates` — the only difference is `createRenderEffect` is not marked `.user = true` and runs in the first effects pass rather than the user-effects pass. Both are blocked by `batch()` when entered from a non-reactive context.

4. **`ToolChunkAppend` should be accumulated, not immediately dispatched.** Tool streaming produces many fine-grained document writes (one per output line). Routing them through the existing RAF accumulation buffer (like `pendingNew`) would naturally batch them with the next `StreamFlush`, eliminating the two-writer race entirely and reducing per-line reconcile overhead.
