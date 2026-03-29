# Deep Analysis: ViewModel createMemo Disposal Bug

**Date:** 2026-03-29
**Status:** Fix applied, pending verification
**Severity:** High — affects all ViewModels that use `createMemo` in their constructors

---

## Summary

When a sysinfo pane's metric is changed via the context menu (`SetMetaCommand`),
the block meta updates, the backend confirms the write (`WaveObj updated`), but
the chart **does not switch**. The data keeps animating (telemetry still flows),
but the metric selection is frozen at whatever it was on first render.

**Root cause:** Solid.js `createEffect` in `block.tsx` disposes all reactive
computations created during its previous run when it re-runs. ViewModel
`createMemo`s created inside the effect body are owned by the effect and get
destroyed on the first block meta update.

---

## Affected Code Path

### 1. ViewModel Creation — `block.tsx:258-269` (BEFORE fix)

```typescript
createEffect(() => {
    const bd = blockData();          // ← TRACKS this signal
    const view = bd?.meta?.view;
    if (!bd || !view) return;
    const bcm = getBlockComponentModel(props.nodeModel.blockId);
    let vm = bcm?.viewModel;
    if (vm == null || vm.viewType !== view) {
        vm = makeViewModel(blockId, view, nodeModel);   // ← runs ONCE
        registerBlockComponentModel(blockId, { viewModel: vm });
    }
    setViewModel(vm);
});
```

**Problem:** `makeViewModel` → `new SysinfoViewModel()` → constructor calls
`createMemo` multiple times. These memos are **owned by the `createEffect`**.

### 2. What triggers the effect to re-run

Any block meta change causes `blockData()` to return a new object:

- `SetMetaCommand({ "sysinfo:type": "Mem" })` — metric switch
- `SetMetaCommand({ "term:zoom": 1.2 })` — zoom change
- `SetMetaCommand({ "pane-title": "..." })` — title edit
- Backend-initiated meta updates (agent color, connection status, etc.)

All of these cause `blockData()` → effect re-runs.

### 3. What happens when the effect re-runs

Solid.js `createEffect` semantics (v1.9.x):

1. Effect re-run begins
2. **All computations created during the previous run are disposed** (Solid.js
   calls `cleanNode` on the effect's owned children)
3. Effect body executes again
4. `vm == null || vm.viewType !== view` → **FALSE** (view is still "sysinfo")
5. ViewModel is reused — `makeViewModel` is NOT called
6. `setViewModel(vm)` re-fires with the same ViewModel instance

**After step 2**, the SysinfoViewModel's `createMemo`s are disposed:
- `plotTypeSelectedAtom` — frozen, no longer tracks `blockAtom()`
- `metrics` — frozen, no longer tracks `plotTypeSelectedAtom()`
- `numPoints`, `connection`, `connStatus`, `intervalSecsAtom` — all frozen

**After step 6**, the chart component still references `model.metrics()` but
calling a disposed memo returns its **last computed value** — it never updates.

### 4. Why data keeps animating but metrics don't switch

Telemetry data arrives via `waveEventSubscribe("sysinfo")` and calls
`model.addContinuousData(dataItem)` which sets `this.dataAtom._set(newData)`.

`dataAtom` is a `createSignalAtom` (a manual signal), not a `createMemo`. It is
**not owned by the createEffect** — it's a standalone signal on the ViewModel
instance. So it survives the disposal and continues to work.

The rendering chain:
```
dataAtom._set(...)  → SysinfoViewInner plotData memo → <For> re-renders  ✓ WORKS
blockAtom() change  → plotTypeSelectedAtom → metrics → yvals → <For>     ✗ BROKEN
```

---

## Reactive Ownership Diagram

```
Block component
  └── createEffect (tracks blockData)
        ├── [run 1] makeViewModel() → new SysinfoViewModel()
        │     ├── createMemo: plotTypeSelectedAtom    ← OWNED BY EFFECT
        │     ├── createMemo: metrics                 ← OWNED BY EFFECT
        │     ├── createMemo: numPoints               ← OWNED BY EFFECT
        │     ├── createMemo: connection               ← OWNED BY EFFECT
        │     ├── createMemo: connStatus               ← OWNED BY EFFECT
        │     └── createMemo: intervalSecsAtom         ← OWNED BY EFFECT
        │
        └── [run 2] (triggered by ANY meta update)
              → cleanNode() disposes ALL owned memos from run 1
              → vm already exists, NOT recreated
              → ViewModel instance alive but memos are dead
```

---

## Why TermViewModel Is Not (Obviously) Affected

`TermViewModel` also creates `createMemo`s in its constructor, but its
**critical memos** use the `useBlockAtom` caching pattern:

```typescript
// TermViewModel (termViewModel.ts)
this.termThemeNameAtom = useBlockAtom(blockId, "termthemeatom", () =>
    createMemo<string>(() => {
        return getOverrideConfigAtom(this.blockId, "term:theme")() ?? DefaultTermTheme;
    })
);
```

`useBlockAtom` (global.ts) wraps the memo factory in `createRoot`:

```typescript
function useBlockAtom<T>(blockId, name, makeFn) {
    const bc = getSingleBlockAtomCache(blockId);
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot(makeFn);   // ← ISOLATED ROOT
        bc.set(name, memo);          // ← CACHED
    }
    return memo;
}
```

The memo is created in its own reactive root (not owned by the effect) and
cached globally by blockId + name. It persists across effect re-runs.

**SysinfoViewModel does NOT use `useBlockAtom`** — its memos are plain
`createMemo` calls in the constructor, directly owned by whatever reactive
context the constructor runs in.

---

## The Fix

### Applied: `createRoot` wrapper in `block.tsx`

Wrap `makeViewModel()` in `createRoot` so the ViewModel's reactive computations
are owned by an independent root, not by the `createEffect`:

```typescript
// BEFORE (broken)
if (vm == null || vm.viewType !== view) {
    vm = makeViewModel(blockId, view, nodeModel);
    registerBlockComponentModel(blockId, { viewModel: vm });
}

// AFTER (fixed)
if (vm == null || vm.viewType !== view) {
    vmRootDispose?.();
    createRoot((dispose) => {
        vmRootDispose = dispose;
        vm = makeViewModel(blockId, view, nodeModel);
    });
    registerBlockComponentModel(blockId, { viewModel: vm });
}
```

`vmRootDispose` is called in:
- The `if` branch (when ViewModel is replaced due to view type change)
- `onCleanup` (when the Block component unmounts)

This gives every ViewModel its own stable reactive root. The memos survive
`createEffect` re-runs because they are no longer children of the effect.

### Scope of Fix

Applied to both `Block` (line 258) and `SubBlock` (line 294) components in
`block.tsx`. Affects ALL ViewModels, not just SysinfoViewModel.

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/block/block.tsx` | `createRoot` wrapper in Block + SubBlock |
| `frontend/app/block/blockframe.tsx` | `getBodyContextMenuItems` injected into body menu |
| `frontend/app/view/sysinfo/sysinfo-model.ts` | Added `getBodyContextMenuItems()` method |
| `frontend/types/custom.d.ts` | Added `getBodyContextMenuItems?()` to ViewModel interface |

---

## Verification

### Confirm the bug existed (before fix)

1. Open a sysinfo pane → chart shows CPU
2. Right-click header → Plot Type → select "Mem"
3. Chart should switch to Memory — **but stays on CPU**
4. Dev log shows `WaveObj updated block:XXX` (meta DID update)
5. Any subsequent meta changes also have no effect

### Confirm the fix works (after fix)

1. Open a sysinfo pane → chart shows CPU
2. Right-click body → select "Mem" → chart switches to Memory
3. Right-click body → select "Net" → chart switches to Network
4. Right-click body → select "CPU" → chart switches back to CPU
5. Repeat several times — each switch should be immediate

### Edge cases to test

- Open multiple sysinfo panes → each should independently switch metrics
- Switch metric on one pane → other panes should not be affected
- Close and reopen a sysinfo pane → should remember last selected metric
- Switch to "All CPU" → should show per-core charts (dynamic count)
- Right-click header → "Plot Type" submenu should also work (pre-existing)

---

## Broader Impact

This disposal bug potentially affects **any ViewModel that creates `createMemo`
in its constructor** without the `useBlockAtom` caching pattern. Currently:

| ViewModel | Has createMemo in ctor? | Uses useBlockAtom? | Affected? |
|-----------|------------------------|--------------------|-----------|
| SysinfoViewModel | Yes (6 memos) | No | **Yes** |
| TermViewModel | Yes (many) | Yes (for critical ones) | Partially |
| AgentViewModel | No | N/A | No |
| ForgeViewModel | No | N/A | No |
| HelpViewModel | No | N/A | No |
| LauncherViewModel | No | N/A | No |

TermViewModel's non-cached memos (`viewIcon`, `viewName`, `viewText`,
`termMode`, `shellProcStatus`, etc.) are also affected — they would freeze after
the first meta update. The `createRoot` fix in `block.tsx` protects all of them.

---

## Alternative Approaches (not taken)

1. **Migrate SysinfoViewModel to `useBlockAtom` pattern** — More surgical but
   only fixes sysinfo. Other ViewModels still exposed.

2. **Move `createMemo`s from constructor to component** — Requires refactoring
   the ViewModel pattern. The view component would need to create the memos and
   pass them back. Breaks encapsulation.

3. **Use `untrack` around `blockData()` in the effect** — Would prevent the
   effect from re-running on meta changes. But the effect NEEDS to track
   `blockData()` to detect view type changes (e.g., "Replace With..." menu).

The `createRoot` wrapper is the correct fix: minimal change, protects all
ViewModels, no behavior change for the effect's view-type-change detection.
