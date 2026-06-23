# Swarm ↔ Pane Two-Way Active-Row Sync

**Status:** Implemented  
**Date:** 2026-06-23

---

## Goal

Keep the Swarm pane's highlighted row in sync with whatever agent pane is currently focused, and vice versa.

- **Swarm → Pane** (already done): clicking a Swarm row focuses the corresponding pane, switching tabs if needed.
- **Pane → Swarm** (this spec): whenever the user directly clicks an agent pane, the Swarm updates to highlight that row. The highlight clears automatically when no tracked agent pane is focused (e.g. the user focuses a terminal or browser tile).

---

## Desired Behaviour

1. User focuses agent pane "Naki" by clicking its tile → the Swarm row for Naki gains a subtle highlighted background.
2. User clicks the Swarm row for "Korp" → Korp's pane is focused (existing behaviour) AND Korp's Swarm row is highlighted; Naki's highlight clears.
3. User clicks a non-agent tile (terminal, browser) → no Swarm row is highlighted.
4. User switches tabs → Swarm re-evaluates focus from the newly active tab. If the focused tile in the new tab is a tracked agent pane, its row is highlighted; otherwise none.

The highlight is **subtle**: a slightly lighter background or a left-border accent — it must not compete visually with the "running" status chip or the hover state.

---

## Architecture

### 1. Expose a reactive `focusedNodeIdAtom` on `LayoutModel`

`LayoutModel.focusedNodeId` is currently a plain getter over `focusedNodeIdStack[0]`, which is not a SolidJS signal and cannot be tracked reactively.

**Change:** add a `createSignal` pair inside `LayoutModel` and update it whenever `focusedNodeIdStack` changes in `updateTree`:

```ts
// layoutModel.ts — inside the LayoutModel class

private _focusedNodeId = createSignal<string | undefined>(undefined);
focusedNodeIdAtom: Accessor<string | undefined> = this._focusedNodeId[0];
private setFocusedNodeId: Setter<string | undefined> = this._focusedNodeId[1];
```

In `updateTree`, after the line that sets `focusedNodeIdStack`:
```ts
this.focusedNodeIdStack = [newId, ...this.focusedNodeIdStack.filter(id => id !== newId)];
this.setFocusedNodeId(this.focusedNodeIdStack[0]);
```

Also update `setFocusedNodeId` in the existing place where `focusedNodeIdStack` is cleared/reset (e.g. on tab teardown or magnify changes that reset focus). The getter `focusedNodeId` stays unchanged — it's still the authoritative read path for non-reactive callers.

### 2. Compute `focusedBlockId` reactively in `SwarmView`

```ts
// swarm-view.tsx — inside SwarmView()

const focusedBlockId = createMemo<string | null>(() => {
    const tabId = atoms.activeTabId();
    if (!tabId) return null;
    const layoutModel = getLayoutModelForTabById(tabId);
    if (!layoutModel) return null;
    const nodeId = layoutModel.focusedNodeIdAtom();
    if (!nodeId) return null;
    // Map nodeId → blockId via leafOrder (reactive signal, geometry-ordered)
    const entry = layoutModel.leafOrder().find(e => e.nodeid === nodeId);
    return entry?.blockid ?? null;
});
```

`atoms.activeTabId()` and `layoutModel.focusedNodeIdAtom()` and `layoutModel.leafOrder()` are all reactive, so `focusedBlockId` re-computes automatically whenever the active tab changes or the focused tile changes within the active tab.

### 3. Pass `focusedBlockId` into `AgentRow`

```tsx
<For each={tree()}>
    {(node) => (
        <AgentRow
            node={node}
            active={focusedBlockId() === node.blockId}
        />
    )}
</For>
```

### 4. Visual treatment in `AgentRow`

```tsx
function AgentRow({ node, active }: { node: AgentTreeNode; active: boolean }): JSX.Element {
    return (
        <div class="swarm-agent-group">
            <div
                class={`swarm-agent-row swarm-agent-row--${node.agentStatus}${active ? " swarm-agent-row--active" : ""}`}
                onClick={() => void focusBlock(node.blockId)}
                title={node.agentName}
            >
                ...
            </div>
            ...
        </div>
    );
}
```

**SCSS** (in `swarm-view.scss`):

```scss
.swarm-agent-row--active {
    background: var(--highlight-bg);
    border-left: 2px solid var(--accent-color, #5b8dd9);
    padding-left: 8px; // compensate for the 2px border so text doesn't shift
}
```

The hover state (`&:hover { background: var(--highlight-bg) }`) already uses `--highlight-bg`, so the active state differentiates via the left-border accent. The active + hover combination is naturally identical to hover (background already shown), which is fine.

---

## What Changes

| File | Change |
|------|--------|
| `frontend/layout/lib/layoutModel.ts` | Add `focusedNodeIdAtom` signal; update it in `updateTree` and wherever `focusedNodeIdStack` is reset |
| `frontend/app/view/swarm/swarm-view.tsx` | Add `focusedBlockId` memo; pass `active` prop to `AgentRow`; apply `--active` class |
| `frontend/app/view/swarm/swarm-view.scss` | Add `.swarm-agent-row--active` style |

No changes to `swarm-model.ts`, `termagent.ts`, or the backend.

---

## Edge Cases

| Case | Behaviour |
|------|-----------|
| Focused pane is not a tracked agent (terminal, browser) | `focusedBlockId()` returns null → no row highlighted |
| Swarm pane itself is focused | Swarm blockId is not in the agent tree → no row highlighted |
| Agent pane in a non-active tab | `activeTabId` points to a different tab; `focusedNodeIdAtom` for the active tab won't match → no row highlighted. This is intentional: the "active" indicator reflects the currently visible/interactive pane, not a historical selection. |
| Tab switch | `atoms.activeTabId()` changes → `focusedBlockId` recomputes for the new tab immediately |
| Agent pane unmounted mid-session | Its block ID leaves `tree()`; the `--active` class naturally disappears with the row |
| Two agent panes tiled in the same tab | Only the one with the focused tile gets `--active`; the other shows no highlight |

---

## Non-goals

- Persisting the "selected" row across restarts.
- Selecting multiple rows.
- Syncing to the Swarm from a *different window* (cross-window focus is not tracked).
