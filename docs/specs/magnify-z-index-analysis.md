# Analysis: Magnified Pane Z-Index Stacking Bug

**Bug:** When a pane is magnified (maximized), agent and forge panes appear on top of the magnified pane instead of behind it.

**Status:** Root cause narrowed down. Z-index chain verified correct via instrumentation.

---

## Instrumentation Results

Log pipe confirmed the full z-index chain works correctly:

```
[magnify:reducer] magnifiedNodeId changed: -> 81ebd04d-...
[magnify:geometry] node 81ebd04d is magnified, zIndex= var(--zindex-layout-magnified-node) size= 684px x 667px
[magnify:tile-node] node 81ebd04d zIndex= var(--zindex-layout-magnified-node) transform= translate3d(85px,83px, 0)
```

1. Reducer correctly sets `magnifiedNodeId`
2. Geometry correctly computes z-index 7 for the magnified node
3. Tile-node DOM element receives the z-index in inline style

**The z-index is applied. The bug is in CSS.**

---

## DOM Structure

```
.tile-layout (position: relative, overflow: hidden)
  └── .display-container (position: absolute, z-index: 0)
        ├── .resize-handle[0] (position: absolute, z-index: 3)
        ├── .resize-handle[1] ...
        ├── .tile-node[A] (position: absolute, transform: translate3d, z-index: undefined)
        │     └── .block.block-frame-default (position: relative)
        │           └── [view content: agent/forge/term]
        ├── .tile-node[B] (position: absolute, transform: translate3d, z-index: 7) ← MAGNIFIED
        │     └── .block.block-frame-default.magnified (position: relative)
        │           └── [view content]
        ├── .tile-node[C] (position: absolute, transform: translate3d, z-index: undefined)
        │     └── .block.block-frame-default.block-focused (position: relative, z-index: 10)
        │           └── [view content: agent/forge/term]
        └── .magnified-node-backdrop (position: absolute, z-index: 6)
```

## Z-Index Hierarchy (theme.scss lines 57-67)

| Level | Value | Element |
|-------|-------|---------|
| 0 | 0 | display-container |
| 1 | 1 | last-magnified-node |
| 2 | 3 | resize-handle |
| 3 | 6 | magnified-node-backdrop |
| 4 | **7** | **magnified-node** |
| 5 | 10 | block-mask-inner / block-focused |

## Stacking Context Analysis

Every `.tile-node` creates a stacking context because `setTransform()` applies:
- `position: absolute`
- `transform: translate3d(x, y, 0)`

CSS spec: `transform` property (non-none value) establishes a stacking context.

### Expected Behavior
- Magnified tile-node: z-index 7 → stacking level 7 in display-container
- Normal tile-nodes: z-index undefined/auto → stacking level 0 in display-container
- 7 > 0 → magnified should render above normal tiles

### What's Actually Happening
The magnified pane appears BEHIND other panes. Possible causes:

1. **CSS variable not resolving:** `var(--zindex-layout-magnified-node)` might not resolve to `7` at the tile-node level. Could be a scoping issue where the CSS custom property isn't inherited to the inline style.

2. **`overflow: hidden` on `.tile-node` (tilelayout.scss line 74):** While this shouldn't affect z-index stacking, it could visually clip content if dimensions aren't applied correctly.

3. **CSS `width: 100%; height: 100%` on `.tile-node` (tilelayout.scss lines 75-76):** These could potentially conflict with inline width/height from `setTransform()`, though inline styles should win.

4. **z-index with CSS variable in inline style:** SolidJS applies inline styles as properties on `element.style`. Setting `element.style.zIndex = "var(--zindex-layout-magnified-node)"` — does the browser resolve CSS variables in inline styles? **Yes, CSS variables work in inline styles per spec.** But there could be browser-specific quirks.

## Recommended Fix

Replace the CSS variable with a numeric literal to rule out variable resolution issues:

```typescript
// In layoutGeometry.ts, line 127:
// Before:
"var(--zindex-layout-magnified-node)"
// After (test):
7
```

If that doesn't fix it, the issue is elsewhere. Alternative fixes:
- Remove `overflow: hidden` from `.tile-node` for magnified nodes
- Remove `width: 100%; height: 100%` from `.tile-node` CSS (inline styles should suffice)
- Set explicit z-index on ALL tile-nodes (0 for normal, 7 for magnified) instead of leaving normal ones as undefined

---

## Files Examined

| File | Lines | Relevance |
|------|-------|-----------|
| `frontend/layout/lib/TileLayout.tsx` | 107-126 | DOM structure, render order |
| `frontend/layout/lib/TileLayout.tsx` | 187-195 | DisplayNodesWrapper — `<Key>` rendering |
| `frontend/layout/lib/TileLayout.tsx` | 295-310 | tile-node inline style application |
| `frontend/layout/lib/TileLayout.tsx` | 129-181 | NodeBackdrops component |
| `frontend/layout/lib/layoutGeometry.ts` | 115-135 | Magnified node z-index assignment |
| `frontend/layout/lib/utils.ts` | 63-88 | setTransform creates stacking context |
| `frontend/layout/lib/tilelayout.scss` | 72-107 | .tile-node CSS (overflow, width, height) |
| `frontend/layout/lib/tilelayout.scss` | 12-35 | Container z-indices |
| `frontend/app/theme.scss` | 57-67 | Z-index variable definitions |
| `frontend/app/block/block.scss` | 443-445 | block-focused z-index: 10 |
