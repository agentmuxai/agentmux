# Spec: Magnify Button + Z-Index Bugs

**Bugs:**
1. Magnify icon/button in pane titlebar is blank (invisible)
2. Magnified pane appears behind other panes (e.g., agent pane blocking maximized forge)

**Status:** Root causes identified.

---

## Bug 1: Magnify Icon Blank

### Root Cause

`MagnifyIcon` in `frontend/app/element/magnify.tsx` imports the SVG with Vite's `?url` suffix and renders it as an `<img>` tag:

```typescript
import magnifyUrl from "../asset/magnify.svg?url";
// ...
<img src={magnifyUrl} style={{ width: "100%", height: "100%" }} />
```

The CSS in `block.scss` (lines 256-260) targets `svg #arrow1, #arrow2` with `fill: var(--main-text-color)`, but CSS **cannot penetrate the `<img>` shadow boundary**. The SVG has `fill="#000"` (black arrows) which is invisible on the dark background.

Additionally, `MagnifyIcon` destructures its props (`{ enabled }`), which in SolidJS captures the value statically at mount time.

### Fix

- Import SVG with `?raw` to get the markup as a string
- Render inline with `innerHTML` so CSS can style the SVG elements
- Use `props.enabled` instead of destructuring

---

## Bug 2: Magnified Pane Behind Other Panes

### Root Cause: Z-Index Conflict

The z-index hierarchy in `theme.scss` (lines 57-67) is:

| Variable | Value | Purpose |
|----------|-------|---------|
| `--zindex-layout-display-container` | 0 | Parent container |
| `--zindex-layout-last-magnified-node` | 1 | Previously magnified |
| `--zindex-layout-magnified-node-backdrop` | 6 | Blur backdrop |
| `--zindex-layout-magnified-node` | 7 | **Magnified pane** |
| `--zindex-layout-ephemeral-node` | 9 | Ephemeral pane |
| `--zindex-block-mask-inner` | 10 | Block mask |

**The critical conflict** is in `block.scss` line 445:

```scss
&.block-focused {
    position: relative;
    z-index: 10;  // HIGHER than magnified node's 7!
}
```

When a non-magnified pane has focus (`.block-focused`), it gets `z-index: 10` — **higher than the magnified node's z-index of 7**. This causes focused panes to render on top of the magnified pane.

### Why `translate3d` Makes It Worse

Every `.tile-node` gets `transform: translate3d(...)` via `setTransform()` in `utils.ts:74`. In CSS, `transform` creates a new **stacking context**, which means:

- Each tile-node is an isolated stacking context
- Z-index only competes between siblings within the same parent stacking context
- The `.display-container` is the parent, and all `.tile-node` divs are siblings within it
- The magnified tile-node has z-index 7, but a focused tile-node's child `.block-focused` has z-index 10

Since `.block-focused` is inside a tile-node stacking context, its z-index 10 shouldn't directly compete with sibling tile-nodes' z-indices. **However**, the tile-node itself may inherit/propagate this... Let me re-examine.

Actually, the real issue may be simpler: **normal tile-nodes don't get a z-index at all** (undefined from `setTransform` when no zIndex parameter is passed). In CSS, `z-index: auto` on a positioned element doesn't create a stacking context despite having `transform`. Wait — `transform` alone DOES create a stacking context. So all tile-nodes are stacking contexts.

With all tile-nodes being stacking contexts:
- Magnified tile-node: z-index 7
- Normal tile-nodes: z-index auto (treated as 0)

This should work — 7 > 0. But if the problem persists, it could be because:
1. The `addlProps()?.transform` accessor returns stale data after tab switch (dead memo issue)
2. The z-index isn't applied at the right time (timing issue with `updateTree`)

### Additional Z-Index Conflicts Found

| Element | Z-Index | File | Line |
|---------|---------|------|------|
| `.block-focused` | 10 | block.scss | 445 |
| `.block-mask` | 10 | block.scss | 418 |
| `.block-focused .block-mask` | 11 | block.scss | 450 |
| `.block-header-animation-wrap` | 100 | block.scss | 56 |
| `.agent-auth-overlay` | 100 | agent-view.scss | 1225 |
| `.term-stickers` | 20 | term.scss | 109 |

### Fix

Lower `.block-focused` z-index to below the magnified node threshold, or raise the magnified node z-index above all internal block z-indices.

---

## File Changes

| File | Change |
|------|--------|
| `frontend/app/element/magnify.tsx` | Inline SVG with `?raw` import, fix SolidJS prop destructuring |
| `frontend/app/block/block.scss` | Fix `.block-focused` z-index conflict |
| `frontend/app/theme.scss` | Potentially raise magnified node z-index |

---

## Testing

1. Each pane should show magnify icon (two arrows) in titlebar — visible on dark and light themes
2. Click magnify — pane expands to fill layout with blur backdrop
3. No other pane should render on top of the magnified pane
4. Click magnify again — pane returns to normal
5. Focus a non-magnified pane, magnify another — magnified pane stays on top
6. Tab switch and back — magnify icon still visible, magnify still works correctly
