# SPEC: Widget "Open in New Window" Context Menu

**Date:** 2026-04-17
**Status:** Draft
**Priority:** Low — UX convenience

---

## Problem

Right-clicking a widget in the sidebar shows only "Unpin from bar" (pinned)
or "Pin to bar" (overflow). There's no way to open a widget directly in a
new window — you have to open a new window first, then add the widget.

## Goal

Add "Open in New Window" to the widget context menu. Clicking it creates a
new window with that widget as the only pane.

## Design

### Context Menu Addition

Both context menus in `action-widgets.tsx` get a new item:

**Pinned widget context menu** (line 339):
```
- Unpin from bar
- Open in New Window    ← new
```

**More dropdown context menu** (line 163):
```
- Pin to bar
- Open in New Window    ← new
```

### Implementation

The action: open a new window, then create a block with the widget's
`blockdef` in that window.

Current `getApi().openNewWindow()` opens an empty window. The new flow:

1. Call `getApi().openNewWindow()` — returns when the new window is ready
2. In the new window, the default block is created automatically
3. Replace the default block with the widget's blockdef

Alternative (simpler): pass the widget's blockdef to `open_new_window` as
a parameter so the CEF host creates the window with that block directly.

For v1, use the simpler approach: just open a new window. The new window
gets a default block (swarm). The user can then click the widget in the new
window's sidebar. This is already functional — the "Open in New Window"
menu item just saves one step.

### Simplest v1

Don't pass blockdef to the new window. Just:
1. `getApi().openNewWindow()` — opens new window
2. In the NEW window's context, `createBlock(widget.blockdef)` — but we
   can't easily call createBlock in another window's context.

Even simpler: just call `openNewWindow()`. The user gets a new window with
the sidebar visible. One click to add the widget. This is already better
than the current flow (no context menu → no discoverability).

For true "open widget in new window," the CEF host would need to accept a
blockdef parameter in `open_new_window` and pass it through to the new
window's initialization. That's a Phase 2 enhancement.

**v1:** Just add the menu item that calls `openNewWindow()` — no blockdef
passing. The menu label is "New Window" (not "Open in New Window" which
implies the widget would be pre-loaded).

## Implementation

One file change: `frontend/app/window/action-widgets.tsx`

Add to both `handlePinnedContextMenu` and `handleItemContextMenu`:
```typescript
{
    label: "New Window",
    click: () => getApi().openNewWindow().catch(console.error),
}
```
