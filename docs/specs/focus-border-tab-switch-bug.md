# Spec: Focus Border Breaks After Tab Switch

**Bug:** Green focus border stops following the selected pane after switching between tabs. Clicking panes still fires focus state updates (confirmed via log pipe), but the CSS class `block-focused` is never toggled — the border stays grey.

**Status:** Root cause identified via frontend log pipe instrumentation.

---

## Root Cause

### The Reactive Owner Problem

SolidJS `createMemo()` and `createEffect()` are tied to a **reactive owner** — the component or `createRoot` scope in which they were created. When that owner is disposed (e.g., component unmounts), all memos and effects under it are cleaned up and stop tracking signal changes.

### How It Manifests

1. **Tab A is active.** `useTileLayout()` calls `getLayoutModelForTab()` which creates a `LayoutModel` and caches it in `layoutModelMap`.

2. **Panes render.** Each pane calls `getNodeModel()` → `layoutNodeModels.ts:39` creates:
   ```typescript
   isFocused: createMemo(() => {
       const treeState = model.localTreeStateAtom();
       return treeState.focusedNodeId === nodeid;
   }),
   ```
   This memo is created under the **current component's reactive owner** (the `TileLayout` component for Tab A).

3. **User switches to Tab B.** Tab A's `TileLayout` component unmounts. SolidJS **disposes its reactive owner**, killing all memos and effects created under it — including the `isFocused` memos in the cached `nodeModels`.

4. **User switches back to Tab A.** The `LayoutModel` is retrieved from cache (line 20 of `layoutModelHooks.ts`). The `nodeModels` Map still has the old entries with **dead memos**. The `isFocused` memos no longer track `localTreeStateAtom` — they return their last cached value forever.

5. **Result:** `focusNode()` correctly updates `treeState.focusedNodeId` and `localTreeStateAtom._set()` fires, but no memo re-evaluates → `block-focused` CSS class never changes → green border is stuck.

### Evidence From Logs

The log pipe (`~/.agentmux/logs/agentmux-host-v*.log | grep '[fe]'`) shows:

- `[focus:focusNode] changing focus from X to Y` — fires correctly on every click
- `[focus:validate] final focusedNodeId= Y` — state updates correctly
- `[service] object.UpdateObject layout` — backend receives the correct focusedNodeId
- **But the CSS never changes** — the memos are dead, so `isFocused()` returns stale values

The SolidJS warning in the log confirms orphaned reactive scopes:
```
[WARN] cleanups created outside a `createRoot` or `render` will never be run
```

### Affected Code

| File | Line | Issue |
|------|------|-------|
| `frontend/layout/lib/layoutNodeModels.ts` | 39-42 | `isFocused: createMemo(...)` created under component owner, dies on unmount |
| `frontend/layout/lib/layoutNodeModels.ts` | 45-48 | `isMagnified: createMemo(...)` same issue |
| `frontend/layout/lib/layoutNodeModels.ts` | 23-35 | `innerRect: createMemo(...)` same issue |
| `frontend/layout/lib/layoutNodeModels.ts` | 38 | `blockNum: createMemo(...)` same issue |
| `frontend/layout/lib/layoutNodeModels.ts` | 49-52 | `isEphemeral: createMemo(...)` same issue |
| `frontend/layout/lib/layoutModelHooks.ts` | 13 | `layoutModelMap` caches models across tab switches |
| `frontend/layout/lib/layoutModelHooks.ts` | 19-24 | Cache hit path returns model with dead memos |

---

## Fix

### Strategy: Give LayoutModel Its Own Reactive Root

Create a `createRoot` scope owned by the `LayoutModel` itself. All memos created for cached node models run inside this root, so they survive component mount/unmount cycles.

### Implementation

#### 1. `LayoutModel` constructor — create a root (`layoutModel.ts`)

```typescript
import { createRoot, Owner } from "solid-js";

// In the LayoutModel class:
private disposeFn: () => void;
runInModelRoot: <T>(fn: () => T) => T;

constructor(tabAtom: () => Tab) {
    // Create a long-lived reactive root for this model's memos.
    // This root survives component mount/unmount cycles.
    createRoot((dispose) => {
        this.disposeFn = dispose;
        // Capture runWithOwner for later use
        const owner = getOwner();
        this.runInModelRoot = <T>(fn: () => T): T => {
            return runWithOwner(owner, fn);
        };
    });
    // ... rest of constructor
}
```

#### 2. `getNodeModel()` — create memos inside model root (`layoutNodeModels.ts`)

Wrap all `createMemo()` calls in `model.runInModelRoot()`:

```typescript
export function getNodeModel(model: LayoutModel, node: LayoutNode): NodeModel {
    const nodeid = node.id;
    const blockId = node.data.blockId;
    const addlPropsAtom = getNodeAdditionalPropertiesAtom(model, nodeid);
    if (!model.nodeModels.has(nodeid)) {
        model.runInModelRoot(() => {
            model.nodeModels.set(nodeid, {
                // All createMemo() calls now run under the model's root
                isFocused: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    return treeState.focusedNodeId === nodeid;
                }),
                // ... other memos
            });
        });
    }
    return model.nodeModels.get(nodeid);
}
```

#### 3. Cleanup on model disposal

When a tab is deleted (`deleteLayoutModelForTab`), call `model.dispose()` to clean up the root:

```typescript
export function deleteLayoutModelForTab(tabId: string) {
    const model = layoutModelMap.get(tabId);
    if (model) {
        model.dispose();
        layoutModelMap.delete(tabId);
    }
}
```

### Why This Works

- `createRoot` creates a reactive scope that is **not** tied to any component lifecycle
- Memos created inside this root track signals normally and survive indefinitely
- The root is disposed only when the `LayoutModel` itself is deleted (tab closed)
- Tab switches merely unmount the rendering component; the model's reactive root stays alive

### What NOT To Do

- **Don't clear `nodeModels` on tab switch** — this would cause unnecessary re-creation and loss of focus history
- **Don't recreate `LayoutModel` on every tab switch** — the cache is intentional for preserving state
- **Don't use `getOwner()/runWithOwner()`** from the component scope — that's the same problem (component owner dies on unmount)

---

## File Changes

| File | Change |
|------|--------|
| `frontend/layout/lib/layoutModel.ts` | Add `createRoot` in constructor, `dispose()` method, `runInModelRoot()` helper |
| `frontend/layout/lib/layoutNodeModels.ts` | Wrap all `createMemo()` in `model.runInModelRoot()` |
| `frontend/layout/lib/layoutModelHooks.ts` | Call `model.dispose()` in `deleteLayoutModelForTab` |

---

## Testing

1. Open Tab A with 2+ panes — green border follows clicks (baseline)
2. Switch to Tab B — interact with panes
3. Switch back to Tab A — green border must still follow clicks
4. Repeat 5+ times rapidly — no degradation
5. Close Tab A — no memory leak (root is disposed)
6. Split panes on a tab, switch away, switch back, click each pane — border follows every time

---

## Related

- **Log pipe spec:** `docs/specs/frontend-log-pipe.md` — the instrumentation that revealed this bug
- **Previous fix (PR #125):** Fixed `numBlocksInTab` destructuring and `<For>` → `<Key>` for resize handles — same class of SolidJS reactivity bugs, but those were about static capture at mount time, not reactive owner disposal
