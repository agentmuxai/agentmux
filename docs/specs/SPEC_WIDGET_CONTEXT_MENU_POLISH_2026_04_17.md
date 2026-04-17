# SPEC: Widget Context Menu Polish

**Date:** 2026-04-17
**Status:** Draft

---

## Problem

Two UX issues with the widget sidebar context menu:

### 1. Generic "New Window" label

Right-clicking a widget shows "New Window" — but doesn't say WHICH widget
will open. The user can't tell if they're about to open a terminal, agent,
or editor in the new window.

**Current:**
```
New Window
─────────
Unpin from bar
```

**Expected:**
```
Open Terminal in New Window
───────────────────────────
Unpin from bar
```

### 2. Hover state drops on context menu open

When the user right-clicks a widget icon, the hover highlight disappears
as soon as the context menu opens. This is disorienting — the user loses
visual context of WHICH widget they right-clicked. The icon should stay
highlighted while its context menu is open.

---

## Design

### Label Fix

Use the widget's `label` field to build the menu item label:

```typescript
{ label: `Open ${widget.label} in New Window`, click: ... }
```

Capitalize the first letter of the label for consistency:
- "Open Agent in New Window"
- "Open Terminal in New Window"
- "Open Editor in New Window"
- "Open Browser in New Window"

Applies to both pinned widgets and More dropdown items.

### Hover State

Add a CSS class `context-menu-active` to the widget element when its
context menu is open. Remove it when the menu closes.

The `ContextMenuModel.showContextMenu()` API doesn't have an `onClose`
callback. Two approaches:

**Option A: CSS `:has()` pseudo-class (no JS needed)**

Not viable — the context menu is rendered in a Portal outside the widget
DOM tree, so `:has()` can't detect it.

**Option B: Signal-based active state**

Track which widget key has an active context menu:

```typescript
const [contextMenuActiveKey, setContextMenuActiveKey] = createSignal<string | null>(null);

const handlePinnedContextMenu = (e: MouseEvent, key: string) => {
    setContextMenuActiveKey(key);
    ContextMenuModel.showContextMenu(menuItems, e);
};

// Clear when context menu closes — listen for the global click that
// dismisses the menu
createEffect(() => {
    if (contextMenuActiveKey() == null) return;
    const handler = () => setContextMenuActiveKey(null);
    // Defer so the menu's own click handler runs first
    requestAnimationFrame(() => {
        document.addEventListener("mousedown", handler, { once: true });
    });
    onCleanup(() => document.removeEventListener("mousedown", handler));
});
```

Apply the class in JSX:
```tsx
<div
    class={`action-widget-slot${contextMenuActiveKey() === key ? " context-active" : ""}`}
>
```

SCSS:
```scss
.action-widget-slot.context-active {
    background: var(--highlight-bg-color, #333);
    border-radius: 4px;
}
```

**Recommended: Option B** — clean signal-based approach, works with the
existing Portal-based context menu.

---

## Implementation

One file: `frontend/app/window/action-widgets.tsx` + `action-widgets.scss`

### Changes

1. **Label**: Replace `"New Window"` with `` `Open ${capitalize(widget.label)} in New Window` `` in both `handlePinnedContextMenu` and `handleItemContextMenu`

2. **Hover state**: Add `contextMenuActiveKey` signal, set on right-click, clear on mousedown outside, apply CSS class

3. **SCSS**: Add `.context-active` style matching the existing hover style

### Passing widget to pinned context menu

The pinned context menu handler currently takes `(e, key)`. Need to pass
the widget too: `(e, key, widget)` — use the label from widget.

For the More dropdown, widget is already available in the closure.
