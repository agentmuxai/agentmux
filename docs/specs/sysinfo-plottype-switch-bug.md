# Bug: Sysinfo Plot Type Switch Does Nothing

**Date:** 2026-03-18
**Status:** Root cause confirmed — fix ready

---

## Symptom

Right-clicking a sysinfo pane and selecting CPU/Memory/Disk from the context menu does not change the displayed chart. The chart always shows what was active at first render (typically CPU).

---

## Root Cause: `createEffect` Disposes ViewModel Memos on Block Meta Update

### The Chain

1. `Block` component (block.tsx line 256) has a `createEffect` that reads `blockData()`:
   ```typescript
   createEffect(() => {
       const bd = blockData();   // reads the block signal
       const view = bd?.meta?.view;
       if (!bd || !view) return;
       const bcm = getBlockComponentModel(props.nodeModel.blockId);
       let vm = bcm?.viewModel;
       if (vm == null || vm.viewType !== view) {
           vm = makeViewModel(...);   // calls SysinfoViewModel constructor
           registerBlockComponentModel(...);
       }
       setViewModel(vm);
   });
   ```

2. `SysinfoViewModel` constructor calls `createMemo` for all reactive properties:
   ```typescript
   this.plotTypeSelectedAtom = createMemo(() => this.blockAtom()?.meta?.["sysinfo:type"] ?? "CPU");
   this.metrics = createMemo(() => PlotTypes[this.plotTypeSelectedAtom()](...));
   this.viewName = createMemo(() => ...);
   this.connection = createMemo(() => ...);
   // etc.
   ```

3. **These memos are owned by the `createEffect`'s first execution** (SolidJS ownership propagates through the synchronous call stack).

4. When `SetMetaCommand` is called (e.g., switch to Memory):
   - Backend sends `waveobj:update` → `wov.setData(newBlock)` → `blockData` signal updates
   - `createEffect` is scheduled to re-run
   - **On re-run, SolidJS disposes all reactive computations owned by the previous execution** — including all SysinfoViewModel's memos
   - The effect finds `vm != null && vm.viewType == "sysinfo"` → reuses the existing VM
   - `setViewModel(vm)` is called with the same vm (no re-render triggered)

5. **Result:** The SysinfoViewModel instance is reused but all its memos are dead:
   - `plotTypeSelectedAtom()` returns the stale value "CPU" forever
   - `metrics()` returns stale `["cpu"]` forever
   - `viewName()` returns stale "CPU" forever
   - `connection()`, `connStatus()`, `intervalSecsAtom()`, `numPoints()` — all stale

6. `SysinfoViewInner` reads `model.metrics()` → gets stale `["cpu"]` → `<For>` never re-renders.

---

## Why Other Things Appear to Work

- The **context menu** rebuilds every right-click (calls `getSettingsMenuItems()` fresh)
- The `plotTypeSelectedAtom()` call inside `getSettingsMenuItems` returns the stale "CPU" value — this is why the radio button might still show CPU as checked even after clicking Memory
- Things that read from OTHER signals (not `blockAtom`-derived memos) still work

---

## Fix

Wrap `makeViewModel` in `createRoot` so the ViewModel's memos are owned by a stable root, not by the `createEffect`:

```typescript
// block.tsx — inside the createEffect:
import { createRoot } from "solid-js";

if (vm == null || vm.viewType !== view) {
    let rootDispose: () => void;
    vm = createRoot((dispose) => {
        rootDispose = dispose;
        return makeViewModel(props.nodeModel.blockId, view, props.nodeModel);
    });
    (vm as any)._solidRootDispose = rootDispose;
    registerBlockComponentModel(props.nodeModel.blockId, { viewModel: vm });
}
```

And in `onCleanup`, call the root dispose:
```typescript
onCleanup(() => {
    const vm = viewModel();
    (vm as any)?._solidRootDispose?.();
    vm?.dispose?.();
    unregisterBlockComponentModel(props.nodeModel.blockId);
});
```

### Why This Works

- `createRoot` creates an isolated reactive scope
- Memos created inside `createRoot` are owned by **that root**, not by the enclosing `createEffect`
- When `createEffect` re-runs (on block meta change), it only disposes its OWN owned computations — NOT the `createRoot`'s computations
- ViewModel memos survive effect re-runs and remain reactive
- `plotTypeSelectedAtom()` correctly re-reads `blockAtom()` and returns the new type

### Files to Modify

| File | Change |
|------|--------|
| `frontend/app/block/block.tsx` | Add `createRoot` around `makeViewModel` call (both `Block` and `SubBlock` components) |
