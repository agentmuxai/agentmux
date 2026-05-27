# Agent pane replaceChild crash on send — root-cause investigation

**Date:** 2026-05-27
**Status:** Root cause hypothesized in §13 — **FALSIFIED** by §15. Investigation continues.
**Severity:** Hard blocker — agent pane crashes on every send
**Reporter:** User testing v0.38.16 (then v0.38.15 + v0.38.11)

---

## 1. Symptom

User opens the agent pane, types any message ("u there"), presses Enter. **Within ~200ms of the first stream event arriving from the backend**, the agent block renders an error fallback (`block-error-boundary`) and the conversation freezes. Reloading the pane brings it back; sending another message reproduces the crash. 100% reproducible across multiple sessions and across multiple builds.

## 2. Evidence (verbatim from host log)

Each crash logs an identical three-line signature:

```
[fe] [agentActivity] busyCount=1 panes=[<blockId>]
[fe] WaveObj updated block:<blockId>
[fe] Missing attribute name 'data-index={index}' on measured element.
[fe] [block-error-boundary] NotFoundError: Failed to execute 'replaceChild' on 'Node':
     The node to be replaced is not a child of this node. (block=<blockId>, view=agent)
[fe] [agentActivity] busyCount=0 panes=[]
[fe] [agent-document-store] CASCADE_DETECTED: slot disposed mid-dispatch
     (cmd=StreamFlush, blockId=<blockId>, source=system).
     A documentAtom subscriber unmounted the pane during this dispatch.
     Subsequent dispatches in the same callback will throw.
```

Timing (one observed crash):

| Time (ms) | Event |
|---|---|
| +0 | `busyCount=1` — turn began (TurnStart already processed) |
| +11 | `WaveObj updated block` — block-meta update propagates |
| +27 | TanStack Virtual warning about a measured element without `data-index` |
| +237 | `replaceChild` throws inside Solid's reconciler |
| +251 | `busyCount=0` — fallback rendered, pane considered done |
| +252 | Cascade detection fires (slot already disposed by the ErrorBoundary) |

## 3. Bisect — falsifying my initial hypothesis

I initially suspected my PR #1068's `TurnStart auto-collapses detailsOpen` write was the trigger and shipped PR #1083 as a hotfix dropping that write. **The crash still reproduced in the hotfix build** (v0.38.15 with the drop in place), proving the auto-collapse write was not the cause. The hotfix is a no-op for this bug.

I then asked the user to test **v0.38.11** — built BEFORE my agent-pane redesign (#1069 / #1075 / #1083). **Same crash, same signature.** This rules out the entire 2026-05-26 agent-pane PR sequence as the culprit. The bug existed before this session's work.

Not yet bisected: 0.38.10, 0.38.8, 0.38.6 — pre-modal-compact builds. Should the user confirm those crash too, the bug is even older.

## 4. What the cascade-detector message actually means

The detector lives in `agent-document-store.ts` after the `setter(state.nodes)` call:

```ts
slot.setter(slot.state.nodes);     // writes documentAtom signal
if (!slots.has(blockId)) {          // slot vanished mid-dispatch
    console.warn("CASCADE_DETECTED: ... A documentAtom subscriber unmounted the pane ...");
}
```

The detector observes a **consequence**, not the cause. The "documentAtom subscriber unmounted the pane" is a literal description: a subscriber, while reacting to the documentAtom write, synchronously called `unregisterPane`. That happens via `onCleanup` in `agent-view.tsx` only when the agent-view component itself disposes. The agent-view component disposes when:

1. The pane closes (user action — not happening here).
2. The block-error-boundary catches a throw and renders the fallback (which doesn't include `<AgentPresentationView>`).

It's path **2**. The `replaceChild` error throws inside a subscriber's render. The error bubbles to `<BlockErrorBoundary>`, which unmounts the agent-view subtree, which fires `onCleanup`, which calls `unregisterPane` → slot deleted. The cascade-detector fires because the deletion happens INSIDE the still-running dispatch callstack.

So the cascade-detector log line is downstream of the real bug. The real bug is the `replaceChild` throw itself.

## 5. What `replaceChild` not-found means in Solid

Solid's reconciler calls `parent.replaceChild(newNode, oldNode)` when it needs to swap a rendered node out (e.g., a `<For>` re-keys, a `<Show>` flips, an HTML attribute changes type). The DOM throws `NotFoundError` when `oldNode.parentNode !== parent` at the moment of the call. Common Solid-specific causes:

| Cause | How to recognize |
|---|---|
| **External DOM mutation** — code outside Solid called `innerHTML`, `removeChild`, `replaceChild`, etc., on a node Solid tracks | Look for direct DOM API usage in components (xterm.js, monaco, browser webview, manual measurers) |
| **Stale element ref** — `ref={...}` callback captured an element that Solid later moved/replaced | Look for `let el; ref={(e) => el = e}` patterns; the captured ref doesn't track Solid's tree |
| **`<For>` with unstable keys** — same logical item shows up under different keys in successive renders | Look for `<For each={...}>` where the input array's identity isn't stable (e.g. recomputed without memo) |
| **Conditional `<Show>` reusing an element** — a child node used in BOTH the `fallback` and the `<Show>` body | Look for shared element references |
| **Manually-managed virtual list — measurement targets** | TanStack Virtual's `measureElement` reads `data-index` and calls some operations on the measured row; if the row was just unmounted by Solid the attribute is gone, the warning fires, and downstream operations work on a stale DOM ref |

The `Missing attribute name 'data-index={index}'` warning at +27ms is from **TanStack Virtual's `measureElement` runtime**. It's the same family — TanStack measured an element that no longer has `data-index` (probably because Solid just unmounted it). The TanStack warning and the Solid throw are **siblings**, both downstream of the same upstream event: **the virtualizer's view of the rendered rows is out of sync with Solid's view at the moment of the first `StreamFlush`**.

## 6. Why `StreamFlush` specifically triggers it

`StreamFlush` is the first command after the agent begins producing output. It commits any buffered nodes onto the documentAtom. Concretely:

- Before flush: documentAtom holds N nodes (history).
- After flush: documentAtom holds N+K nodes (K new agent-message / tool nodes appended).

The virtualizer reacts: new `virtualizedNodes` partition → `getVirtualItems()` returns more items → some new rows mount, possibly some existing rows shift position. TanStack Virtual asynchronously schedules `measureElement` calls on all visible rows.

**Hypothesis**: there's a race where TanStack measures an element AFTER Solid has dismounted it (no `data-index` warning fires), and Solid's next reconcile tries to replace a child that another reactive pass has already removed.

## 7. Why I can't pin it down without instrumentation

The stack trace is minified to `bpe → wp → Object.fn → Jge → dm → j → wp → Object.fn → Jge → dm`. Solid's render path with no source maps. Doesn't tell me **which component** is reconciling at the failure point.

What I'd need to bisect:
- **Source maps in production bundle** — currently the Vite prod build strips them. Re-enable for a debug build and the stack becomes readable.
- **`getOwner()`-instrumented panic handler** — wrap the reconciler in a try/catch that logs `getOwner().componentName` before re-throwing.
- **Disable virtualization temporarily** — render the document as a plain `<For>` instead of `AgentDocumentVirtualList` and see if the crash still fires. If yes, the virtualizer is innocent; the bug is in a node component. If no, the bug is in the virtualizer / measurement coordination.

## 8. Open hypotheses (ranked by suspicion)

1. **TanStack Virtual + Solid `<For>` row-keying race.** `AgentDocumentVirtualList` renders rows from `getVirtualItems()` inside a Solid `<For>`. If TanStack returns items with a key that Solid mis-maps to a different DOM node on re-render, Solid's reconciler can call `replaceChild` against the wrong parent. This is consistent with the `data-index` warning + replaceChild throw co-occurrence.

2. **A streaming row component throws on partial data.** When the first agent-message node arrives, it has e.g. empty `content` or a transitional shape that the renderer doesn't expect. The throw is internal to the component (not in framework code), but reaches the boundary the same way. The replaceChild error might be a side-effect of trying to render an unmount-mid-render component.

3. **External DOM mutation on a streaming surface.** `<MarkdownRenderer>` or `<AnsiText>` or a code-highlight component might be doing manual DOM work. If it `innerHTML`-mutates a node Solid tracks, the next reconcile fails. Less likely but possible.

4. **`agent-document-store` reducer producing reference-unstable arrays.** If `state.nodes` is being rebuilt on every flush with new object identities for unchanged nodes, the virtualizer over-measures and Solid over-reconciles. Audit the reducer's `StreamFlush` arm.

5. **Backwards-incompatible TanStack Virtual upgrade.** A recent dependency bump may have changed `measureElement`'s contract.

## 9. Proposed investigation path

In priority order:

### 9.1 Build a debug-bundle portable

`task package` with `--mode=development` so the bundle keeps source maps. Reproduce the crash. The stack trace will name the offending component. ~5-10 minute task, totally deterministic.

### 9.2 Read the cascade audit ring

`recordDispatch()` is supposed to capture every dispatch with source / commit / events. The agent-document-store has its own audit slice. Compare an ordinary StreamFlush (when the pane doesn't crash) with the crashing one — what's different about the latter's command payload? The `addedNodes` shape may reveal the bad node.

### 9.3 Disable virtualization (temporary)

Replace `AgentDocumentVirtualList` with a plain `<For each={documentAtom()}>...</For>` in agent-view.tsx for a debug build. If crash stops → virtualizer at fault. If crash persists → a node component throws on first paint of new nodes.

### 9.4 Add a `getOwner()`-aware reconcile wrapper

Monkey-patch Solid's `insertNode` / `replaceChild` calls in the bundle to log the owning component before invoking. Heavy-handed but yields a precise answer.

## 10. Why I should NOT keep shipping hotfixes

Three small reverts so far this session targeted symptoms (TurnStart auto-collapse, padding micro-adjustments, etc.). None addressed the actual `replaceChild`. The pattern matches `feedback_3strikes_term_jumble.md` from memory: **when 3+ fixes fail in the same area, stop and restructure.** That's where we are.

## 11. Recommended next step

The user should choose one:

- **A. Build a debug bundle now**, reproduce, read the source-mapped stack. Highest signal, ~15 minutes. **Recommended.**
- **B. Temporarily route around the virtualizer** — render the document as a plain list (no virtualization). If the crash stops we've localized to the virtualizer interaction; if it persists we've localized to a node component. ~30 minutes.
- **C. Live in this state** — agent pane crashes on every send; user can't use the app meaningfully. Not viable.

## 12. What to do about #1083 (the wrong hotfix)

PR #1083 dropped the `TurnStart auto-collapses detailsOpen` write. The drop is **wrong** — that write is the spec-prescribed UX behavior and dropping it does not fix the crash. **Close #1083 without merging** and let the spec write stand. We can re-investigate the original concern (whether that write contributes to ANY race) after the actual replaceChild root cause is fixed.

## 13. ROOT CAUSE IDENTIFIED — 2026-05-27 follow-up

Built a debug-bundle portable with NODE_ENV=development to get source maps and re-read the minified positions. The replaceChild call resolves to:

```js
}else t.replaceChild(n[a++],e[o++])
```

This is Solid's **`reconcileArrays`** — the child-array diff loop that `<For>` uses to reconcile its rendered items against the new array. `e` is the prior children, `n` is the new children. `t.replaceChild(n[a], e[o])` fails because **`e[o]` is no longer a child of `t`**.

The For in question is in `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`:

```tsx
<For each={virtualizer.getVirtualItems()}>
    {(virtualItem) => {
        const nodeAccessor = () => partition().virtualizedNodes[virtualItem.index];
        return <DocumentRow node={nodeAccessor} ... />;
    }}
</For>
```

**Why this breaks:**

Solid's `<For>` keys items by **referential identity** (`===` comparison). TanStack Virtual's `getVirtualItems()` returns a freshly-allocated array of `VirtualItem` objects on every call:

```ts
interface VirtualItem { key: Key; index: number; start: number; end: number; size: number; lane: number; }
```

When `StreamFlush` lands → `partition()` recomputes → `virtualizer.getVirtualItems()` returns ALL-NEW objects (even for unchanged rows). Solid's `<For>` sees every item as fresh → re-keys all rows → reconcile from scratch → `replaceChild` called for every row.

Meanwhile, TanStack Virtual has scheduled async `measureElement` ResizeObserver callbacks against the rows. Between Solid's reconcile and the next measurement tick, a row's DOM gets moved (or removed) by Solid, but TanStack's stored reference is now stale. When TanStack measures, the row's `data-index` attribute is gone → the **`Missing attribute name 'data-index={index}' on measured element`** warning fires. When Solid's loop reaches that row in its diff, the parent-child relationship has been mutated under it → `replaceChild` throws.

### The fix

Solid's `<For>` doesn't accept a custom key function. Standard remedies in priority order:

1. **`<Index each={...}>`** — Solid's positional keyer. Each slot in the array gets a stable identity tied to its **position**. Re-rendering the same array shape doesn't churn keys. Trade-off: when items reorder, `<Index>` does NOT move DOM nodes; it re-renders content in place. For a virtual list where order is stable (indexed by virtualizer), this is correct.

2. **`<Key by={(v) => v.key} each={...}>`** from `solid-primitives/keyed` — keys on TanStack's stable `item.key` (which IS stable across renders — it's derived from `keyExtractor(index)`, default `(i) => i`). Adds a small dep.

3. **Manually memoize the items array** — wrap `getVirtualItems()` such that unchanged items return the same reference. Brittle; not recommended.

**Recommended: option 1** (`<Index>`). It's a one-keyword swap in the JSX (`<For>` → `<Index>`) plus a getter wrapper since `<Index>` passes accessors to children instead of values. The semantics match what the virtualizer actually does (positional rows).

### Why this is a long-standing bug

`AgentDocumentVirtualList` was introduced as PR #784 (per the comment about "reagent P1"). The `<For>` keying issue was almost certainly always present — but it only manifests as a crash when the document grows during a streaming render that races with TanStack's measurement. Before this session's testing scenarios, the user may have hit it occasionally; the cascade-detector logs from #878 caught the symptom but the prevention pattern (`dispatchIfRegistered` for async dispatches) didn't apply because this isn't a dispatch race — it's a render race.

### Action

Switch the For to `<Index>` (or `<Key by={item.key}>`). Single-file change with the keying swap. Then rebuild + verify the crash stops.

## 15. Hypothesis falsified — `<For>` keying alone is NOT the cause

Built v0.38.16 with the `<For>` → `<Index>` swap on `AgentDocumentVirtualList`'s virtualizer loop (PR #1086, closed). Reproducer ran identically — same `reconcileArrays` + `replaceChild` stack, same `CASCADE_DETECTED` on `StreamFlush`. The swap is a valid Solid hygiene improvement but it's NOT the trigger.

This means EITHER:

(a) **Another `<For>` exists in the agent document's render tree that mis-keys.** Audit didn't find an obvious candidate in `AgentMessageBlock` / `ToolBlock` / `DocumentRow`. Possible: the markdown/AST renderer trees, a hidden conditional `<For>`, or `<For>` inside an effect-driven component that's NOT in the agent view directory.

(b) **The root cause is not `<For>` keying.** Solid's `reconcileArrays` is the call site, but the underlying cause could be:

  - DocumentRow's `ref={virtualizer.measureElement}` calling TanStack's measure on a node whose Solid-owned children get mutated during measurement. TanStack reads `data-index`, then schedules layout work; if the children of the measured node mutate between the read and the layout, Solid's next reconcile sees DOM out of sync with its model.
  - An `innerHTML` mutation inside one of the message renderers (markdown/syntax-highlight component might use raw HTML).
  - A Solid-incompatible ref pattern where a `ref={...}` callback caches a node Solid later moves.

## 16. Concrete next-step proposals

Stop guessing. Use one of these to localize the bug deterministically:

### 16.1 Surgical disable — render document without virtualization

Bypass `AgentDocumentVirtualList` entirely. In `AgentDocumentView.tsx`, replace the call with a plain:

```tsx
<For each={viewState.nodes()}>
    {(node) => <DocumentRow node={() => node} ... />}
</For>
```

(or `<Index each={viewState.nodes()}>` for symmetry). No virtualizer. No `measureElement`. No `data-index`. If the crash stops → confirms the virtualizer-measurement interaction is the cause. If it persists → the bug is in a node-renderer component.

Cost: O(N) DOM rows; bad for long sessions but FINE for the crash-repro scenario (a fresh agent with 5-10 nodes).

### 16.2 Disable all dynamic node-content renderers — render bare summaries

Replace `DocumentRow`'s body with a stub:

```tsx
return <div data-index={dataIndex()}>node-{node().id}</div>;
```

Reload, send. If the crash stops → the bug is in one of the message/tool/markdown renderers. Then bisect by re-enabling them one at a time.

Cost: ugly UI for the diagnosis run.

### 16.3 Instrument `<ErrorBoundary>` to capture component owner

Patch `<BlockErrorBoundary>` to call `getOwner()` and walk up logging component names BEFORE re-throwing. Will name the component whose render failed. 10-line change to `BlockErrorBoundary.tsx`.

### 16.4 Read source-mapped stack via DevTools

Open DevTools (Ctrl+Shift+I) before sending the message, let the crash fire — Chromium's DevTools console resolves source maps and shows the original file:line for each frame. Doesn't require any code change.

### 16.5 Roll back to before PR #784

Per #2 of the file: AgentDocumentVirtualList was introduced as PR #784 (per the comment about reagent P1). Hypothesis: that whole redesign brought in the bug. Test by checking out a tagged build from BEFORE #784 (April 2026 or earlier) and reproducing.

## 17. Recommended next step

**16.4 (DevTools)** — zero code change, deterministic answer, and the user can do it themselves in 30 seconds. Open DevTools (Ctrl+Shift+I or right-click → Inspect Element), focus the Console tab, send "u there" and let it crash. The error in the console will show source-mapped frames pointing at the offending component.

If for some reason DevTools doesn't resolve the maps (CEF can be quirky about that), fall back to 16.1 (surgical-disable virtualization). About 15 minutes to wire.

## 18. References

- `docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md` — the original cascade-disposal class of bug. PR #878 added detection; the prevention contract was supposed to be `dispatchIfRegistered` for async paths. The current crash sneaks through because it's NOT async — the cascade is on a synchronous render throw.
- Solid `replaceChild` family of crashes documented at https://github.com/solidjs/solid/issues — multiple matches.
- TanStack Virtual `measureElement` contract: https://tanstack.com/virtual/latest/docs/api/virtualizer
