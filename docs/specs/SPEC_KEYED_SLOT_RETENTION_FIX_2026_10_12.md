# Spec: `<Key>` slots must not keep old rows alive

**Date:** 2026-10-12
**Status:** Active, implemented together with this spec. Item 1 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Evidence:** `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` §4.2.

## 1. The problem

`@solid-primitives/keyed`'s `keyArray`, the function behind `<Key>`, creates each new slot in `addNewItem`:

```js
function addNewItem(list, item, i, key) {
    createRoot(dispose => {
        ...
        prev.set(key, save);
        list[i] = save.mapped;   // the root's closure captures `list`
    });
}
```

**The leak**
- `list` is the result array of the reconcile pass that created the slot. It holds the mapped output, meaning the DOM, of every item in the list at that moment.
- The arrow function passed to `createRoot` uses `list`, so V8 keeps `list` in that function's context.
- The slot's `dispose` (stored in the `prev` Map for the slot's whole life) and its root owner both reach that context.
- Result: every live slot keeps the full row output of the pass it was born in. Items that have since left the list are disposed, but their DOM stays reachable, detached, until the last slot born in the same pass is gone.

**Measured in AgentMux**
- The agent document re-partitions on almost every stream flush, so `<Key>` creates slots constantly (`AgentDocumentVirtualList.tsx`, the windowed head and the streaming buffer). Each slot pins a snapshot of the rows on screen when it was created, tool bodies included.
- One heap snapshot of 4 agent panes held **728k detached DOM objects**: 118k text nodes, 74k spans and 14.5k tool-preview lines.
- The retainer path ran from the context holding `addNewItem`'s parameters (`key`, `i`, `item`, `list`) to those rows.
- Patching only this function took the page's DOM node count from **497k to 249k** and the renderer from 850 to 760 MB, on the same workload.

**Who else is affected**
- `<Key>` is also used by the tile layout (`frontend/layout/lib/tilelayout-shared.tsx`, `TileLayout.core.tsx`) and the pane chrome (`frontend/app/tab/pane-leaf-chrome.tsx`).
- There the lists are small and change rarely, so the retained arrays are small too. The same fix removes them.

**Upstream:** the latest stable release (1.5.3, published 2026-08-12) has the same code. Only 3.0 pre-releases exist beyond it.

## 2. The fix

Ship `keyArray` and `Key` from `frontend/util/keyed.ts`, a copy of `@solid-primitives/keyed` 1.5.3's two functions, keeping its MIT notice. Change one thing: `addNewItem` hands the mapped value out through a local variable and writes `list[i]` after `createRoot` returns, so the root's closure never references `list`.

```ts
function addNewItem(list: U[], item: T, i: number, key: K) {
    let mapped!: U;
    createRoot((dispose) => {
        ...
        prev.set(key, save);
        mapped = save.mapped;
    });
    list[i] = mapped;
}
```

**Behaviour is otherwise identical:** same keys, same disposal, same fallback, same index signal when `mapFn` takes two arguments, same reconcile order.

**Not vendored:**
- The server-rendering branch (`isServer`). AgentMux renders only in the browser, and the copy doesn't import `solid-js/web`.
- The package's other exports (`Entries`, `MapEntries`, `SetValues`, `Rerun`). AgentMux uses none of them.

**The four call sites** import from `@/util/keyed` (`frontend/util/keyed`) instead of the package, and the dependency is removed from `package.json`.

**Why a copy, not a patch tool**
- The repo has no `patch-package` (or similar) setup, and npm's script allow-list would make adding one a policy change.
- Copying about 70 lines of MIT code, with its notice and a note on what differs, is smaller and plain to review.

## 3. Test

`frontend/util/keyed.test.tsx`:

**Leak**
- Map items to instances of a `Probe` class.
- Shrink the list so a surviving slot was born in a pass whose other items are gone.
- Then count live `Probe` objects with `v8.queryObjects(Probe, { format: "count" })`, which runs a full GC first.
- Pass: only the current items' probes are alive.
- On the unpatched library the same test counts the dropped items as alive.

**Behaviour**
- Keyed reuse: same key, same mapped object.
- Disposal of removed keys: their `onCleanup` runs.
- Index signal updates on reorder.
- The fallback when the list is empty.

The existing agent-pane and tile-layout tests run unchanged.

## 4. Measuring it

On the lab workload from the root-cause report (4 scripted agents, 30 rounds, then a forced GC), expect about the "B" column:
- DOM nodes roughly halved (497k → about 250k);
- detached DOM objects in a heap snapshot down from about 728k to a few thousand;
- the renderer about 90 MB lower.

The PR records its own measurement.

## 5. Follow-up

Report the bug upstream with the reproduction from §3, and offer the two-line patch. Once a fixed release ships, `frontend/util/keyed.ts` can go back to re-exporting the package.
