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

### 1. Derive `focusedBlockId` from the existing `focusedNode` memo

`LayoutModel` already exposes `focusedNode: () => LayoutNode` as a reactive `createMemo` (reading from `localTreeStateAtom`). No changes to `layoutModel.ts` are needed.

### 2. Compute `focusedBlockId` reactively in `SwarmView`

```ts
// swarm-view.tsx — inside SwarmView()

const focusedBlockId = createMemo<string | null>(() => {
    const tabId = atoms.activeTabId();
    if (!tabId) return null;
    const layoutModel = getLayoutModelForTabById(tabId);
    if (!layoutModel) return null;
    return layoutModel.focusedNode()?.data?.blockId ?? null;
});
```

`atoms.activeTabId()` and `layoutModel.focusedNode()` are both reactive, so `focusedBlockId` re-computes automatically whenever the active tab changes or the focused tile changes within the active tab.

### 3. Pass `focusedBlockId` accessor into `AgentRow`

The accessor is passed as a prop (not the evaluated value) so it can be read reactively inside the row via `classList`:

```tsx
<For each={tree()}>
    {(node) => <AgentRow node={node} focusedBlockId={focusedBlockId} />}
</For>
```

### 4. Visual treatment in `AgentRow`

```tsx
function AgentRow({ node, focusedBlockId }: { node: AgentTreeNode; focusedBlockId: () => string | null }): JSX.Element {
    return (
        <div class="swarm-agent-group">
            <div
                classList={{
                    "swarm-agent-row": true,
                    [`swarm-agent-row--${node.agentStatus}`]: true,
                    "swarm-agent-row--active": focusedBlockId() === node.blockId,
                }}
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
    outline: 1px solid var(--accent-color, #5b8dd9);
    border-radius: 0;
}
```

`outline` renders outside the box model, avoiding any layout shift. `border-radius: 0` overrides the row's default `4px` to give hard corners that frame the entire row.

---

## What Changes

| File | Change |
|------|--------|
| `frontend/app/view/swarm/swarm-view.tsx` | Add `focusedBlockId` memo using existing `layoutModel.focusedNode()`; pass accessor to `AgentRow`; apply `--active` class via `classList` |
| `frontend/app/view/swarm/swarm-view.scss` | Add `.swarm-agent-row--active` style |

No changes to `layoutModel.ts`, `swarm-model.ts`, `termagent.ts`, or the backend.

---

## Edge Cases

| Case | Behaviour |
|------|-----------|
| Focused pane is not a tracked agent (terminal, browser) | `focusedBlockId()` returns null → no row highlighted |
| Swarm pane itself is focused | Swarm blockId is not in the agent tree → no row highlighted |
| Agent pane in a non-active tab | `activeTabId` points to a different tab; `focusedNode()` for the active tab won't match → no row highlighted. This is intentional: the "active" indicator reflects the currently visible/interactive pane, not a historical selection. |
| Tab switch | `atoms.activeTabId()` changes → `focusedBlockId` recomputes for the new tab immediately |
| Agent pane unmounted mid-session | Its block ID leaves `tree()`; the `--active` class naturally disappears with the row |
| Two agent panes tiled in the same tab | Only the one with the focused tile gets `--active`; the other shows no highlight |

---

## Non-goals

- Persisting the "selected" row across restarts.
- Selecting multiple rows.
- Syncing to the Swarm from a *different window* (cross-window focus is not tracked).
