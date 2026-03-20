# Spec: Tab Drag-and-Drop Animation System

**Date:** 2026-03-20

---

## Goal

Replace the current static drag experience (tabs don't move during drag) with a fluid, elastic animation that:

1. **During drag** — tabs adjacent to the insertion point "squish" apart slightly to preview the drop position
2. **On drop** — the gap snaps open fully and the surrounding tabs spring back to their natural widths
3. **Dropped tab** — does a brief, subtle bounce/settle when it lands

---

## Visual Description

### Phase 1: Dragging (continuous)

As the cursor moves across the tab bar, the **nearest insertion point** (the gap between two tabs, or before/after the first/last) opens up gradually. The two tabs flanking the gap each compress slightly toward their edges — like the gap is pushing them apart.

- The gap width should track cursor proximity smoothly (CSS transition, not instant)
- The squish is proportional to how "confident" the insertion point is — i.e., the closer the cursor is to the midpoint between two tabs, the wider the gap
- The dragging tab itself renders at reduced opacity (already done) but its ghost stays under the cursor

**No abrupt jumps.** Tabs slide smoothly as the gap migrates from one position to another.

### Phase 2: Drop

When the user releases:

1. The gap at the insertion point **snaps fully open** — the two flanking tabs jump apart at full speed
2. The released tab **scales in** from slightly-smaller-than-natural (0.88×) to 1× with a short overshoot (spring: scale goes briefly to 1.04×, then settles at 1.0×)
3. The remaining tabs all return to their natural widths simultaneously with a quick ease-out

Total duration: ~250ms

### Phase 3: Cancel (ESC or release outside tab bar)

All tabs return to natural widths, no bounce on the dragged tab.

---

## Implementation Approach

### Core Mechanism: CSS custom properties + transitions

Each `tab-drop-wrapper` will expose two CSS custom properties that drive the animation:

```css
--tab-gap-before: 0px;   /* extra space inserted to the LEFT of this tab */
--tab-gap-after:  0px;   /* extra space inserted to the RIGHT of this tab */
```

These map to `padding-left` and `padding-right` on the wrapper. JS sets them reactively as the drag position changes.

### Gap size

The gap target width is **32px** (roughly one tab's worth of breathing room). During drag it opens proportionally:

```
gapFraction = clamp((1 - distanceToCursor / TAB_WIDTH), 0, 1)
gap = gapFraction * 32px
```

This means the gap is widest when the cursor is directly over the insertion point and closes as the cursor moves away.

### Which tabs get the gap

Given the nearest insertion point (between tab[i] and tab[i+1]):
- `tab[i]` gets `--tab-gap-after: {gap}px`
- `tab[i+1]` gets `--tab-gap-before: {gap}px`

If the insertion point is before the first tab, only tab[0] gets `--tab-gap-before`.
If the insertion point is after the last tab, only the last tab gets `--tab-gap-after`.

### CSS transitions

```css
.tab-drop-wrapper {
    padding-left:  var(--tab-gap-before, 0px);
    padding-right: var(--tab-gap-after, 0px);
    transition: padding-left 120ms ease-out, padding-right 120ms ease-out;
}
```

When the gap migrates (cursor crosses a tab midpoint), the old gap closes and the new one opens — CSS handles the interpolation, no JS animation loop needed.

### Drop bounce

The dropped tab wrapper gets a `tab-bounce` class immediately after drop, triggering:

```css
@keyframes tab-bounce {
    0%   { transform: scaleX(0.88); }
    55%  { transform: scaleX(1.04); }
    80%  { transform: scaleX(0.98); }
    100% { transform: scaleX(1.0);  }
}
.tab-drop-wrapper.tab-bounce {
    animation: tab-bounce 280ms cubic-bezier(0.34, 1.56, 0.64, 1) forwards;
    transform-origin: center bottom;
}
```

The class is removed after 280ms (or via `animationend`).

---

## Signals / State

Add to `droppable-tab.tsx` (or a shared context):

```typescript
// Replaces nearestHint with richer insertion info
export type InsertionPoint = {
    beforeTabId: string | null;  // tab to the LEFT of gap (null = gap at start)
    afterTabId:  string | null;  // tab to the RIGHT of gap (null = gap at end)
};

export const [insertionPoint, setInsertionPoint] = createSignal<InsertionPoint | null>(null);
```

Each `DroppableTab` reads `insertionPoint()` and derives its `--tab-gap-before` / `--tab-gap-after` from it:

```typescript
const gapBefore = () => {
    const ip = insertionPoint();
    return ip?.afterTabId === props.tabId ? GAP_PX : 0;
};
const gapAfter = () => {
    const ip = insertionPoint();
    return ip?.beforeTabId === props.tabId ? GAP_PX : 0;
};
```

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/tab/tabbar-dnd.ts` | Add `InsertionPoint` type; replace `nearestHint` signal with `insertionPoint`; update `computeInsertionPoint()` to return `{beforeTabId, afterTabId}` |
| `frontend/app/tab/droppable-tab.tsx` | Derive `gapBefore`/`gapAfter` from `insertionPoint`; apply as inline `--tab-gap-before`/`--tab-gap-after`; add `tab-bounce` class on drop |
| `frontend/app/tab/tabbar.tsx` | Monitor `onDrag` sets `insertionPoint` instead of `nearestHint`; on drop/cancel, clears it |
| `frontend/app/tab/tabbar.scss` | `padding-left/right` transitions on `.tab-drop-wrapper`; `@keyframes tab-bounce`; remove `tab-insert-left/right` line indicators (gap IS the indicator now) |

---

## What to Remove

- The `tab-insert-left` / `tab-insert-right` CSS line indicators (the 2px green bars) — the animated gap replaces them as the visual insertion cue
- The `tab-just-dropped` box-shadow pulse added in the previous session

---

## Testing

1. 3 tabs: drag tab A slowly across tab B and tab C — gap should smoothly migrate
2. Release between B and C — B springs right, C springs left, A bounces in
3. Drag to end of bar — gap opens after the last tab, drop bounces in there
4. Press ESC during drag — all gaps close, no bounce
5. Drag with only 2 tabs — single gap at the other tab, still bouncy
6. Cross-section drag (pinned → regular) — gap appears in the target section correctly
