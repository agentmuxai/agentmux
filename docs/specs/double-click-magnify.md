# Spec: Double-Click to Magnify Pane

**Goal:** Double-clicking a pane's header bar toggles magnify (maximize/restore).

**Status:** Ready for implementation.

---

## Behavior

- **Double-click on header** → toggles magnify on that pane (same as clicking the magnify icon)
- **Single click on header** → unchanged (focuses pane, initiates drag if held)
- **Double-click on pane body** → no effect (would interfere with text selection, terminal input, etc.)

---

## Why Header Only

Double-clicking the pane body would conflict with:
- Terminal text selection (double-click selects a word)
- Editor text selection
- Agent/forge interactive elements
- Any view-specific double-click behavior

The header is a safe, unambiguous target — it has no double-click behavior today.

---

## Implementation

### File: `frontend/app/block/blockframe.tsx`

Add `onDblClick` handler to the `.block-frame-default-header` div (line ~300):

```tsx
<div
    class="block-frame-default-header"
    data-role="block-header"
    data-testid="block-header"
    ref={dragHandleRef ? (el) => { dragHandleRef.current = el; } : undefined}
    onContextMenu={onContextMenu}
    onDblClick={() => props.nodeModel.toggleMagnify()}
    style={headerStyle()}
>
```

### Drag Interaction

The header is also the drag handle (`dragHandleRef`). Double-click should NOT trigger a drag. HTML5 drag requires `mousedown → mousemove` — a double-click without movement won't start a drag, so there's no conflict.

### Edge Cases

1. **Already magnified** → double-click unmagnifies (toggle behavior, same as icon)
2. **Ephemeral pane** → drag is already disabled for ephemeral panes; double-click should still work for magnify
3. **Preview mode** → no header interaction in preview mode (not a concern, preview doesn't render headers)

---

## Testing

1. Double-click pane header → pane magnifies
2. Double-click again → pane unmagnifies
3. Single click header → pane focuses (no magnify)
4. Drag header → drag works (no magnify triggered)
5. Double-click header while magnified → unmagnifies
6. Double-click terminal body → selects word (no magnify)

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/block/blockframe.tsx` | Add `onDblClick` to `.block-frame-default-header` div |
