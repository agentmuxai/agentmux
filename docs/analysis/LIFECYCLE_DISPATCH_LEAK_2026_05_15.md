# Analysis: Lifecycle / Dispatch Leak Class

**Date:** 2026-05-15
**Author:** AgentA
**Incident:** v0.33.899 portable, first launch post-#877 merge — red-text `replaceChild` NotFoundError surfaced; root cause was `[agent-pane-state] dispatch for unregistered pane c0ae6c7 (cmd=StreamFlushObserved)`.

---

## 1. Incident & root signal

From `~/.agentmux/versions/0.33.899/logs/agentmux-host-v0.33.899.log.2026-05-15`:

```
23:46:19.953Z WARN  [fe] [agent-document-store] dropped 1 stream updates for unknown ids in c0ae6c7
23:46:20.073Z ERROR [fe] [UNCAUGHT-ERROR] Uncaught Error:
    [agent-pane-state] dispatch for unregistered pane c0ae6c7 (cmd=StreamFlushObserved).
    registerPane must be called synchronously in the component body.
```

The "replaceChild NotFoundError" the user saw in the UI is a *downstream symptom* — a SolidJS render tree torn by an uncaught error inside a reactive owner. The actual fault is the dispatch into a slot that has already been unregistered.

The 120 ms warning right before the throw is the critical clue: it's emitted by `agent-document-store.ts`'s `StreamFlush` handler when the reducer drops stale `updatedNodes`. So at line 161 of `useAgentStream.ts`, `dispatchDoc(blockId, { type: "StreamFlush", ... })` **succeeded** — the agent-document slot was still registered. Seven lines later at line 168, `dispatchPane(blockId, { type: "StreamFlushObserved", ... })` **threw** — the agent-pane-state slot was gone.

Two slots, two different stores, two different `onCleanup` callbacks. Whatever unregistered the pane-state slot did so **between** lines 161 and 168 of a single synchronous function call.

## 2. The leak class

> **A reactive owner can be disposed *during* a synchronous dispatch sequence if a signal write inside the first dispatch cascades through reactive subscribers and causes a parent to re-render the owner away. Subsequent dispatches in the same callback then hit a no-longer-registered slot and throw.**

### 2.1 Why slot unregistration *can* happen mid-callback

`agent-pane-state-store.ts` `dispatch()` runs the reducer, then calls projection setters:

```ts
if (slot.state.streaming !== prev.streaming) slot.proj.streaming(slot.state.streaming);
if (slot.state.turnActive !== prev.turnActive) slot.proj.turnActive(slot.state.turnActive);
// ... seven more projections
```

Each `slot.proj.*` is a SolidJS signal setter. Setters notify subscribers **synchronously** within the same JavaScript turn. Subscribers include:
- `createMemo` and `createEffect` registered against the atom.
- JSX expressions reading the atom (their owning `Show` / `For` / `Switch` will re-evaluate).
- The layout tree, which composes the entire pane hierarchy from reactive state.

If any of those subscribers, on this write, decides "the pane should no longer exist" (e.g. a layout transformation triggered by a state change, a tab close cascading from a focus shift, a model swap), SolidJS synchronously runs the disposed pane's `onCleanup` chain — including `unregisterAgentPaneStatePane(blockId)`.

The original `dispatch()` returns. Control returns to `flushPendingNodes`. The next line (`dispatchPane(blockId, { type: "StreamFlushObserved" })`) sees a deleted slot and throws.

### 2.2 Why `cancelAnimationFrame` doesn't save us

The cleanup at `useAgentStream.ts:518` calls `cancelAnimationFrame(flushRafId)`. That **does** prevent a *pending* RAF callback from running. It does **not** unwind a callback that is *currently executing* — and the cascade-during-dispatch scenario above is exactly "currently executing."

### 2.3 Why the per-store throw rule is right (and the leak is on us)

Both `agent-pane-state-store.ts` and `agent-document-store.ts` throw on dispatch to an unregistered slot, on purpose: silent drops would mask reducer-command loss and were explicitly removed in conventions §4–§5 (frontend-reducer-conventions-2026-05-03.md). The throw is the contract.

It is the *callers'* job not to dispatch to a slot that may already be gone.

## 3. The defensive patterns in use today

Four distinct guard patterns exist in the codebase. They are *not* equally effective against the cascade scenario:

| Pattern | Example | Defends against |
|---|---|---|
| **Mounted-flag** (`let mounted = true; onCleanup(() => mounted = false; if (!mounted) return)`) | `useHistoryPagination.ts:126-129` and 6 dispatch guards | Async (await) continuations after the owner disposed. **Does not** defend against mid-callback cascade unless checked between *every* dispatch. |
| **Subscription lifecycle** (subscribe in `onMount`, unsubscribe in `onCleanup`) | `useAgentStream.ts:519-521` `fileSubject.subscribe`, `blockChunkUnsub`, `acceptedUnsub` | Future emissions after `onCleanup` — but emissions *already in flight on the JS stack* still fire. |
| **`if (this.closed) return`** model flag, checked at every dispatch | `browser-model.ts:261, 282, 311, 361` (uniform pattern across IPC handlers) | Both async continuations *and* mid-handler cascades, **provided the check is at every dispatch site, not just the function entry.** |
| **`dispatchIfAlive(...)` model helper** | `workflows-model.ts:509-515`, used at all 10 dispatch sites | The strongest variant — encapsulates the closed-check so callers can't forget. |

### 3.1 The `workflows-model.ts` pattern is the right answer

```ts
private dispatchIfAlive(command, source) {
    if (this.disposed) return;
    dispatchWorkflowRun(this.blockId, command, source);
}
```

Every async path (`await RpcApi.RunWorkflowCommand()`, `.then(async () => …)` continuations, the WPS subscription handler) routes through this one method. There is no way for a caller to forget the guard. New dispatches default-safe.

Compared to the mounted-flag pattern (which requires the caller to remember to `if (!mounted) return` before each dispatch), the helper-method pattern is strictly stronger — it is impossible to forget unless someone bypasses the helper entirely.

## 4. Site-by-site audit

Catalog produced by reading every dispatch call site in `frontend/app/`. Risk re-evaluated under the cascade lens (an async-context with even one un-guarded dispatch is a risk; a synchronous body that does multiple dispatches in sequence after a signal write is also a risk because the first write can cascade).

### 4.1 HIGH RISK — directly implicated in tonight's crash

#### `frontend/app/view/agent/useAgentStream.ts:150-173` — `flushPendingNodes()` (RAF callback)
```ts
function flushPendingNodes() {
    flushRafId = null;
    if (pendingNew.length === 0 && pendingUpdates.length === 0) return;
    const batchNew = pendingNew; const batchUpdates = pendingUpdates;
    pendingNew = []; pendingUpdates = [];

    dispatchDoc(blockId, { type: "StreamFlush", newNodes: batchNew, updatedNodes: batchUpdates });
    // ↑ THIS WRITE can cascade through reactive subscribers and dispose the pane.
    dispatchPane(blockId, { type: "StreamFlushObserved", addedCount: batchNew.length, at: Date.now() });
    // ↑ THIS THROWS if the cascade unregistered the pane-state slot.
}
```
**Status:** This is the exact site that crashed. Two consecutive dispatches across two stores; the first can cascade-dispose the second's slot.

### 4.2 HIGH RISK — same pattern, undiagnosed but vulnerable

#### `frontend/app/view/agent/useAgentStream.ts:359-374` — `StreamTruncate` handler
```ts
const events = dispatchDoc(blockId, { type: "StreamTruncate", reason: "fileop" });
const honored = events.some((e) => e.type === "truncate-applied");
if (!honored) return;
// ...reset locals...
dispatchPane(blockId, { type: "TurnReset" });
```
Same shape: `dispatchDoc` → cascade window → `dispatchPane`.

#### `frontend/app/view/agent/useAgentStream.ts:290-301` — accepted-message handler
```ts
dispatchPane(blockId, { type: "PendingMessageAccepted", id: messageId });
// ...sync logic that reads getPending()...
dispatchPane(blockId, { type: "TurnStart", at: Date.now() });
```
Two `dispatchPane` calls; the first can cascade. The second can hit a gone slot.

#### `frontend/app/view/agent/useAgentStream.ts:520-526` — onCleanup itself
```ts
dispatchPane(blockId, { type: "StreamUnsubscribe", at });
dispatchDoc(blockId, { type: "SessionEnd", at });
```
This runs in the cleanup chain. It is *before* `unregisterAgentPaneStatePane` in cleanup-execution order (the stream hook's cleanup is registered later in onMount, runs first in reverse). Currently safe — **but fragile**: if anyone adds an `onCleanup` after this hook's that calls `unregisterAgentPaneStatePane` first, both these dispatches start throwing. The same cascade risk applies *inside* the dispatches themselves.

### 4.3 HIGH RISK — same class, different file

#### `frontend/app/view/browser/browser-model.ts:442-444` — reload() RAF
```ts
reload(): void {
    if (this.closed) return;
    const url = this.urlAtom();
    if (url) {
        this._dispatch({ type: "UrlCleared" }, "reload-clear");
        requestAnimationFrame(() => {
            this._dispatch({ type: "Navigate", url }, "reload-restore");
            // ↑ no this.closed check inside the RAF callback
        });
    }
}
```
The closed check at function entry doesn't cover the RAF callback firing after a dispose. This site was already flagged in the dispatch-site audit and is the lowest-friction fix.

### 4.4 MEDIUM RISK — synchronous-only today, fragile to refactor

#### `frontend/app/view/agent/agent-view.tsx:268-272, 300-304` — `handleDecide` and `onLoginSuccess`
Synchronous click and callback dispatches. Today the click flow runs to completion before any reactive cascade can dispose the pane. *But*: if either dispatch is later wrapped in a microtask, `await`, or callback chain, the synchronous-safety disappears. There is no compile-time enforcement.

#### `frontend/app/view/agent/hooks/useAgentCommands.ts:314, 355` — RPC `.catch()` handlers
```ts
RpcApi.AgentInputCommand(...).catch((err) => {
    dispatchPane(blockId, { type: "SendFailed", ... });
});
```
Today the latency of the failing RPC effectively covers the race. *But*: a synchronous RPC failure path (e.g. precondition check) would expose this. No guard.

### 4.5 LOW RISK — guarded via mounted-flag

`frontend/app/view/agent/hooks/useHistoryPagination.ts` — six dispatch sites, all preceded by `if (!mounted) return;` after every `await`. **Safe today** *for the dispatch-after-await scenario*. **Not safe** against the mid-callback cascade — if a `dispatchDoc(HistoryRestored)` cascade-disposes the pane, the immediately-following `dispatchPane(InitReady)` would still throw. (No evidence this happens in practice, but the pattern is fragile.)

### 4.6 SAFE — `workflows-model.ts`

All 10 dispatch sites route through `dispatchIfAlive()`. Even if the helper runs inside a cascade-disposed state, the `this.disposed` check fires first and the dispatch is silently dropped.

## 5. Why tonight's bug surfaced now (a guess)

PR #877 added two new code paths:
1. `useHistoryPagination` dispatches `HistoryRestored` from a microtask after `BlockfileReadStateCommand` resolves.
2. `agent-view.tsx` adds a `createEffect` that watches `getDocument()` and toggles a `dirty` flag.

The `createEffect` is a *new reactive subscriber* on the document atom. Every `StreamFlush` now triggers one additional reactive tick. That tick is what tipped a previously-quiet timing into the cascade window: the dirty-effect runs, *something* it touches (or that the projection setters touch in sequence) cascades into a re-evaluation that disposes the pane. The race was always there; the new reactive subscriber made it deterministic enough to hit on the first smoke test.

This is unfalsified speculation — proving it would require instrumenting the reactive scheduler to log dispose ordering, which we don't have. But the timing of the regression (PR #877 added the only new reactive subscriber on `documentAtom` in months) is consistent.

## 6. Proposed fix

### 6.1 Tactical (one-PR scope, immediate)

Adopt the `dispatchIfAlive` pattern at the store layer rather than at every caller. Two options:

**Option A — caller-side guard at every site (cheaper, fragile):** add `let mounted = true; onCleanup(() => mounted = false)` in `useAgentStream`, then `if (!mounted) return;` before each of the 11 dispatch calls. Replicate in `useAgentCommands`, fix `browser-model.reload()`. Easy to forget on next feature.

**Option B — store-level "soft dispatch" variant (correct, stronger):** add a sibling export to each pane-store:
```ts
// agent-pane-state-store.ts
export function dispatchIfRegistered(
    blockId: string,
    command: AgentPaneCommand,
    source: CommandSource = "system",
): AgentPaneEvent[] {
    if (!slots.has(blockId)) return [];  // silent no-op
    return dispatch(blockId, command, source);
}
```
Migrate all *async-context* dispatch sites (anything reached via RAF/setTimeout/setInterval/await/subscription) to the soft variant. Keep the throwing `dispatch()` for *synchronous body* dispatches where a missing slot really is a bug (registration order violation).

The pair preserves the original safety net for the "you forgot to register" bug while giving async sites a way to dispatch without caring whether the owner is still alive.

**Recommendation: B.** A grep policy ("any dispatch inside RAF/setTimeout/setInterval/await/subscribe must use `dispatchIfRegistered`") becomes mechanically auditable.

### 6.2 Architectural (follow-up)

The two-stores-two-cleanups asymmetry is the deeper issue. A single dispatch from `useAgentStream` writes to both stores. A future refactor might:
- Co-locate per-pane state into one slot keyed by blockId with multiple sub-fields, so registration is atomic.
- Or: have agent-view.tsx own a single `paneOwnerToken` registered synchronously and required by every store dispatch; on unregister of the token, *all* per-pane stores cull their slot in one step.

This is beyond the scope of a hotfix. File it as a tracking issue against discussion #707.

### 6.3 What NOT to do

- **Do not** silently catch the throw in `useAgentStream`. The current throw is the contract; catching it would mask future registration-order bugs introduced by other hooks.
- **Do not** delete the throw entirely. Same reason. Silent drops were intentionally removed in May 2026.
- **Do not** reorder cleanups by re-registering `unregisterAgentPaneStatePane` inside `onMount` to push it last in execution order. Mismatched register/unregister scopes are worse than the original problem.

## 7. Action items

| # | Action | Owner | Scope |
|---|---|---|---|
| 1 | Add `dispatchIfRegistered` to `agent-pane-state-store.ts` and `agent-document-store.ts` | AgentA | small PR |
| 2 | Migrate the 11 async dispatch sites in `useAgentStream.ts` to use the soft variant | AgentA | same PR |
| 3 | Migrate the 2 RPC `.catch()` sites in `useAgentCommands.ts` to the soft variant | AgentA | same PR |
| 4 | Fix `browser-model.reload()` RAF — add `if (this.closed) return;` inside the RAF callback (or migrate the model's `_dispatch` to the soft variant) | AgentA | same PR |
| 5 | Migrate `useHistoryPagination` post-await dispatches to the soft variant (defense in depth; mounted-flag stays as belt-and-suspenders) | AgentA | same PR |
| 6 | Add a Vitest that exercises the cascade: dispatchDoc → reactive subscriber disposes pane → dispatchPane should not throw | AgentA | same PR |
| 7 | Follow-up tracking issue: unify per-pane store registration into one atomic step | AgentA → #707 | doc-only |
| 8 | (Optional) lint rule or grep CI gate: forbid bare `dispatch(` inside `requestAnimationFrame`/`setTimeout`/`setInterval`/`subscribe(`/`.then(`/`.catch(` | AgentA | follow-up PR |

## 8. Instrumentation (installed 2026-05-15)

The two open questions from the original draft — *how do we confirm the cascade theory* and *which subscriber triggers it* — are now answered by instrumentation rather than speculation. The next time the crash reproduces, the log will pinpoint the cause without further guesswork.

### 8.1 What was installed

**`frontend/app/store/agent-pane-state-store.ts:103-132`** — the dispatch() function now wraps each of the eight projection setters in a small `proj()` helper. After each setter call, it checks whether `slots.has(blockId)` is still true. The first setter that loses the slot is recorded as `cascadeSetter`. After the projection block, if `cascadeSetter` is set, a single warning is logged:

```
[agent-pane-state] CASCADE_DETECTED: '<setterName>' setter disposed pane mid-dispatch
(cmd=<commandType>, blockId=<short>, source=<source>).
A reactive subscriber on the '<setterName>' atom unmounted the pane during dispatch.
Subsequent dispatches in the same callback will throw.
```

**`frontend/app/store/agent-document-store.ts:115-128`** — same check around `slot.setter(slot.state.nodes)`. There is only one setter (the document atom) so the message is simpler:

```
[agent-document-store] CASCADE_DETECTED: slot disposed mid-dispatch
(cmd=<commandType>, blockId=<short>, source=<source>).
A documentAtom subscriber unmounted the pane during this dispatch.
Subsequent dispatches in the same callback will throw.
```

### 8.2 Properties

- **Pure logging, no behavior change.** The throw on subsequent dispatches still fires — the throw is the contract, the log just makes its cause visible. The instrumentation runs after the projection setter so it can't itself prevent the cascade. It is safe to leave on in production until the §7 soft-dispatch migration lands.
- **No new RPC, no new IPC.** `console.warn` goes through the existing `log-pipe` and lands in `~/.agentmux/logs/agentmux-host-vX.Y.Z.log.YYYY-MM-DD` tagged `[fe]` like every other frontend warning.
- **Per-setter granularity in the pane-state store.** If the cascade comes from a subscriber on `streaming` vs `initPhase` vs `pending`, we will see exactly which one. That tells us which atom the offending subscriber is reading.
- **301 reducer tests still pass** after the refactor — the `proj()` helper is a behavior-preserving rewrite of the eight per-field setter calls.

### 8.3 What to expect from the next reproduction

When the user repros tonight's crash on a build with this instrumentation, the log will show one of two shapes:

**Shape A — pane-state setter is the cascade source:**
```
... CASCADE_DETECTED: '<X>' setter disposed pane mid-dispatch (cmd=StreamFlushObserved, ...)
... Uncaught Error: [agent-pane-state] dispatch for unregistered pane ... (cmd=<next>)
```
That confirms a subscriber on atom `<X>` is the trigger — almost certainly the new dirty-flag `createEffect` if `<X>` is one of the atoms it transitively reads. Verifies the cascade theory **and** identifies the exact subscriber.

**Shape B — document setter is the cascade source:**
```
... [agent-document-store] CASCADE_DETECTED: slot disposed mid-dispatch (cmd=StreamFlush, ...)
... Uncaught Error: [agent-pane-state] dispatch for unregistered pane ... (cmd=StreamFlushObserved)
```
Confirms the cascade originates from documentAtom subscribers — matches tonight's crash signature exactly. This is the predicted shape.

**Shape C — no CASCADE_DETECTED log, just the throw:**
Would falsify the cascade theory; the slot was unregistered by something other than a reactive subscriber chain (a remount race, or two panes sharing a blockId). At that point, instrument `unregisterPane` itself with a stack trace and reproduce again.

### 8.4 Why the createEffect-removal experiment is no longer needed

The original §8 suggested removing the `createEffect` in `agent-view.tsx:213-220` to test whether it's the trigger. With per-setter cascade detection, that experiment is now mechanically resolved — the `cascadeSetter` name tells us which atom the offending subscriber reads. We can then grep for that atom's subscribers and confirm the createEffect (or some other subscriber) is the path.

If the next reproduction shows `cascadeSetter = "initPhase"` or `"streaming"` or one of the others **that the dirty-flag effect doesn't read**, then the dirty-flag isn't the trigger and we look elsewhere (likely the layout tree reacting to a derived signal).

The createEffect reads `getDocument()` (the documentAtom). So the cascade hypothesis predicts **Shape B** above, not Shape A. The instrumentation will distinguish.

## 9. Action items (updated)

| # | Action | Status | Owner |
|---|---|---|---|
| 1 | Add `dispatchIfRegistered` soft-variant to both pane stores | PENDING | AgentA |
| 2 | Migrate the 11 async dispatch sites in `useAgentStream.ts` to soft variant | PENDING | AgentA |
| 3 | Migrate the 2 RPC `.catch()` sites in `useAgentCommands.ts` to soft variant | PENDING | AgentA |
| 4 | Fix `browser-model.reload()` RAF — add `if (this.closed) return;` inside the RAF callback | PENDING | AgentA |
| 5 | Migrate `useHistoryPagination` post-await dispatches to soft variant (belt-and-suspenders) | PENDING | AgentA |
| 6 | Add a Vitest that exercises the cascade: setter calls `unregisterPane`, verify CASCADE_DETECTED log fires + subsequent dispatch throws | PENDING | AgentA |
| 7 | Cascade-detection instrumentation in both pane stores | **DONE** 2026-05-15 | AgentA |
| 8 | Reproduce the crash on a build with the instrumentation; capture the `cascadeSetter` value from the log | PENDING | user |
| 9 | Follow-up tracking issue: unify per-pane store registration into one atomic step | PENDING | AgentA → #707 |
| 10 | (Optional) lint rule or grep CI gate: forbid bare `dispatch(` inside `requestAnimationFrame`/`setTimeout`/`setInterval`/`subscribe(`/`.then(`/`.catch(` | PENDING | AgentA |

### 9.1 Recommended PR sequencing

- **PR-1 (this work)**: instrumentation only. Lands on main; next portable build captures cascade data the next time the crash repros.
- **PR-2 (informed by PR-1's data)**: `dispatchIfRegistered` soft-variant + migrations + vitest. Scope of action items 1–6.
- **PR-3 (architectural)**: unified per-pane registration. Action item 9.

This sequencing lets PR-2's design choices be informed by the actual cascade source rather than designed-around the predicted-but-unconfirmed one.

---

🤖 Authored by AgentA, 2026-05-15. Filed against discussion #707 once PR-2 ships.
