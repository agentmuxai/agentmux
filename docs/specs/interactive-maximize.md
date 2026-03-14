# Spec: Interactive Maximize (Pane Magnify Overhaul)

**Goal:** Replace the current non-interactive magnify zoom with a fully interactive maximize that lets users work inside the maximized pane.

**Status:** Analysis complete, ready for implementation.

---

## Current Behavior

When a user clicks the magnify button:
1. The pane expands to 80% of the container (centered, configurable via `window:magnifiedblocksize`)
2. A backdrop blur overlay appears behind it (z-index 6)
3. The magnified pane gets z-index 7
4. Clicking anywhere on the backdrop unmagnifies the pane
5. The pane has CSS `backdrop-filter: blur(10px)` and reduced opacity (60%)
6. Dragging is disabled (`draggable={!isMagnified()}`)

### What's Actually Broken

1. **Z-index doesn't work:** Despite the magnified tile-node getting `z-index: 7` via inline style (confirmed by log instrumentation), other panes (especially agent view) still render on top. The CSS `translate3d()` stacking contexts and `width: 100%; height: 100%` on `.tile-node` interfere with the inline z-index.

2. **Content appears non-interactive:** The backdrop (z-index 6, full-screen, `pointer-events: auto`) sits just below the magnified pane. Clicking in the 10% margin area hits the backdrop and unmagnifies. The reduced opacity (60%) and blur make the pane look like a preview rather than a working area.

3. **Terminal/editor can't be used:** Even though `inert` is NOT set on magnified panes, the visual treatment (blur + low opacity) signals "non-interactive" to the user.

---

## Root Cause: Why Z-Index Fails

Every `.tile-node` gets `transform: translate3d()` from `setTransform()`, which creates a CSS stacking context. The magnified node gets `z-index: 7` and normal nodes get `z-index: undefined` (auto = 0).

**Theory:** z-index 7 > 0 should work. But empirical testing shows agent panes always render on top regardless of z-index value (tested up to 100). This suggests either:
- SolidJS doesn't apply the z-index inline style the way we expect
- The CSS `width: 100%; height: 100%` on `.tile-node` combined with inline dimensions causes layout conflicts
- There's a browser-specific stacking context interaction we're not accounting for

**Conclusion:** Trying to fix z-index within the current architecture is a dead end. A different approach is needed.

---

## Proposed Design: True Interactive Maximize

### Strategy: Render Maximized Pane in a Separate Layer

Instead of trying to make the magnified tile-node float above siblings via z-index, **render the maximized pane in a dedicated overlay container** that sits above the entire layout.

### Architecture

```
.tile-layout
  ├── .display-container (z-index: 0)
  │     ├── .tile-node[A] (normal pane)
  │     ├── .tile-node[B] (normal pane — the magnified pane's slot stays here but hidden)
  │     └── .tile-node[C] (normal pane)
  │
  ├── .magnify-backdrop (z-index: 50, full-screen blur overlay)
  │
  └── .magnify-container (z-index: 51, contains the maximized pane)
        └── [BlockFrame content — fully interactive]
```

### Key Design Decisions

1. **Separate container:** The maximized pane renders in `.magnify-container`, a sibling to `.display-container` — not inside it. This sidesteps all stacking context issues.

2. **Full interactivity:** No blur, no reduced opacity on the maximized pane. It looks and behaves exactly like a normal pane, just bigger.

3. **Backdrop behavior:**
   - Clicking the backdrop unmagnifies (same as today)
   - Backdrop has `backdrop-filter: blur(Xpx)` to dim other panes
   - Escape key also unmagnifies

4. **Size:** Default 90% of container (up from 80%), configurable. Consider 100% (true fullscreen within the layout area) as an option.

5. **Original slot:** The magnified pane's original tile-node position is preserved (not removed from the tree). It's just visually hidden while magnified.

### Implementation Plan

#### 1. TileLayout.tsx — Add magnify container

Render a new container outside `.display-container`:

```tsx
<div class="tile-layout">
    <div class="display-container" ref={...}>
        <ResizeHandleWrapper />
        <DisplayNodesWrapper />
    </div>

    {/* Magnify layer — outside display-container, above everything */}
    <Show when={magnifiedNodeId()}>
        <div class="magnify-backdrop" onClick={unmagnify} />
        <div class="magnify-container">
            {/* Render the magnified pane's BlockFrame here */}
            <MagnifiedPane layoutModel={layoutModel} />
        </div>
    </Show>

    <Placeholder />
    <OverlayNodeWrapper />
</div>
```

#### 2. MagnifiedPane component

A new component that renders the magnified pane's content directly:

```tsx
const MagnifiedPane = (props: { layoutModel: LayoutModel }) => {
    const magnifiedNodeId = () => props.layoutModel.magnifiedNodeIdAtom();
    const magnifiedNode = () => props.layoutModel.focusedNode();

    return (
        <Show when={magnifiedNode()}>
            {(node) => {
                const nodeModel = useNodeModel(props.layoutModel, node());
                return (
                    <div class="magnify-pane">
                        {props.layoutModel.renderContent(nodeModel)}
                    </div>
                );
            }}
        </Show>
    );
};
```

#### 3. CSS — tilelayout.scss

```scss
.magnify-backdrop {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 50;
    backdrop-filter: blur(var(--block-blur, 2px));
}

.magnify-container {
    position: absolute;
    z-index: 51;
    // Centered with configurable size
    top: 5%;
    left: 5%;
    width: 90%;
    height: 90%;
    border-radius: var(--block-border-radius);
    overflow: hidden;
}

.magnify-pane {
    width: 100%;
    height: 100%;
}
```

#### 4. Hide original tile-node while magnified

In `DisplayNode`, add a class or style to hide the tile-node when its content is magnified:

```tsx
class={clsx("tile-node", {
    dragging: isDragging(),
    "tile-hidden": isMagnified(),
})}
```

```scss
.tile-node.tile-hidden {
    visibility: hidden;
}
```

#### 5. Keyboard support

- **Escape** → unmagnify
- **Ctrl+Shift+M** → toggle magnify on focused pane (if not already bound)

#### 6. Remove old magnify z-index logic

In `layoutGeometry.ts`, remove the z-index assignment for magnified nodes since the magnified pane now renders in a separate container:

```typescript
// REMOVE this block:
if (model.magnifiedNodeId === node.id) {
    // ... z-index logic
    addlProps.transform = transform;
}
```

The geometry still needs to compute the original pane position so the tile-node slot is preserved.

### What NOT To Change

- Keep `magnifiedNodeId` on the tree state (persisted, survived tab switches)
- Keep the toggle mechanism via `magnifyNodeToggle`
- Keep the configurable size via `window:magnifiedblocksize`
- Keep the ephemeral node system as-is (it has the same z-index issues but is less critical)

---

## Alternative: Simpler Fix (Minimum Viable)

If the full overlay approach is too invasive, a simpler fix:

1. **Move NodeBackdrops and magnified tile-node OUTSIDE `.display-container`** into a sibling with higher z-index
2. Keep everything else the same
3. Just fix the stacking context isolation issue

This is less clean but fewer changes.

---

## File Changes (Full Approach)

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.tsx` | Add `MagnifiedPane` component, magnify container/backdrop, hide original tile-node |
| `frontend/layout/lib/tilelayout.scss` | Add `.magnify-backdrop`, `.magnify-container`, `.tile-hidden` styles |
| `frontend/layout/lib/layoutGeometry.ts` | Remove magnified node z-index/transform override |
| `frontend/app/block/block.scss` | Remove `.magnified` opacity/blur styles (pane is now normal-looking) |
| `frontend/app/theme.scss` | Add new z-index variables for magnify layer |

---

## Testing

1. Open 3+ panes on a tab
2. Click magnify on any pane → pane expands to 90%, backdrop blurs other panes
3. Type in terminal / interact with content → fully interactive
4. Click backdrop → unmagnifies
5. Press Escape → unmagnifies
6. Switch tabs and back → magnify state preserved
7. Magnify agent pane → agent content is interactive and on top
8. Magnify forge pane → forge content is interactive and on top
9. Magnify while another pane is focused → correct pane maximizes
10. Close magnified pane → unmagnifies first, then closes
