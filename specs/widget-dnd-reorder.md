# Widget Drag-and-Drop Reorder

**Status:** Draft
**Author:** AgentX
**Date:** 2026-03-11

---

## Overview

Allow users to reorder widgets in the header bar by dragging them left/right. The new order persists across restarts via `settings.json`.

---

## User Experience

### Interaction

1. User grabs any widget icon in the header bar and drags it horizontally.
2. A ghost/placeholder shows the drop position as the user drags.
3. On drop, the widget snaps to its new position and the bar reorders live.
4. Order persists — restarting the app preserves the custom order.

### Visual Design

- **Drag handle:** Entire widget button is draggable (no separate handle needed given the compact icon-only layout).
- **Drag ghost:** Semi-transparent clone of the widget icon follows the cursor.
- **Drop indicator:** A thin vertical line (2px, accent color) renders between widgets to show the insertion point.
- **Active drag state:** Dragged widget slot in the bar dims to ~30% opacity to show its origin.
- **Hover during drag:** Other widgets shift slightly to open a gap at the current insertion point (smooth CSS transition, ~120ms).

### Constraints

- Dragging is horizontal only (widget bar is a single row).
- `swarm` (hidden by default) participates in ordering even when hidden — its order slot is preserved.
- No drag between separate UI zones (widget bar only).

---

## Data Model

### Storage Key

Widget display order is persisted as a single `settings.json` key:

```json
"widget:order": ["agent", "forge", "terminal", "sysinfo", "settings", "help", "devtools", "swarm"]
```

- Value is an ordered array of widget base names (the part after `defwidget@`).
- Any widget not in the array falls back to its `display:order` from `widgets.json` (backwards compatible).
- Unknown names in the array are ignored (deleted widgets don't break anything).

### Fallback / Initial Value

If `widget:order` is absent from settings, the bar renders using `display:order` from `widgets.json` as today. On first drag-and-drop, the full current order is written to settings.

---

## Architecture

### Backend

No new RPC handlers needed. Reorder uses the existing `setconfig` handler:

```ts
// On drop, write the new order
RpcApi.setConfig({ "widget:order": newOrderArray });
```

`setconfig` already writes to `settings.json` and broadcasts a config event immediately (no latency).

### Frontend — Reading Order

`action-widgets.tsx` (or wherever widgets are rendered) currently sorts by `widget["display:order"]`. Change to:

1. Read `fullConfig.settings["widget:order"]` (string array).
2. If present, sort the widget list by index in that array.
3. If absent, fall back to `display:order` as before.

```ts
function getSortedWidgets(widgets: WidgetConfigType, settings: SettingsType): DefWidget[] {
    const defs = Object.entries(widgets).map(([key, def]) => ({ key, def }));
    const order: string[] | undefined = settings["widget:order"];
    if (order) {
        return defs.sort((a, b) => {
            const ai = order.indexOf(a.key.replace("defwidget@", ""));
            const bi = order.indexOf(b.key.replace("defwidget@", ""));
            const an = ai === -1 ? 999 : ai;
            const bn = bi === -1 ? 999 : bi;
            return an - bn;
        });
    }
    return defs.sort((a, b) => (a.def["display:order"] ?? 0) - (b.def["display:order"] ?? 0));
}
```

### Frontend — Drag-and-Drop

Use the [HTML5 Drag and Drop API](https://developer.mozilla.org/en-US/docs/Web/API/HTML_Drag_and_Drop_API) — no external library needed.

**State (local to the widget bar component):**

```ts
const [draggingKey, setDraggingKey] = useState<string | null>(null);
const [dropIndex, setDropIndex] = useState<number | null>(null);
```

**Widget button attributes:**

```tsx
<button
    draggable
    onDragStart={() => setDraggingKey(widgetKey)}
    onDragEnd={() => { setDraggingKey(null); setDropIndex(null); }}
    onDragOver={(e) => { e.preventDefault(); setDropIndex(computeDropIndex(e, index)); }}
    onDrop={() => commitReorder(draggingKey, dropIndex)}
    className={draggingKey === widgetKey ? "widget-dragging" : ""}
>
```

**`commitReorder`:**

```ts
function commitReorder(from: string, toIndex: number) {
    const next = [...currentOrder];
    const fromIndex = next.indexOf(from);
    next.splice(fromIndex, 1);
    next.splice(toIndex, 0, from);
    RpcApi.setConfig({ "widget:order": next });
    setDraggingKey(null);
    setDropIndex(null);
}
```

---

## Settings Type Update

`SettingsType` in `wconfig.rs` needs one new field:

```rust
#[serde(rename = "widget:order", skip_serializing_if = "Option::is_none")]
pub widget_order: Option<Vec<String>>,
```

This field goes through the existing `#[serde(flatten)] extra: HashMap` if we keep it untyped, so it may require **no Rust change at all** — the array will round-trip through `extra` as a `serde_json::Value`. Only if typed access is needed on the Rust side does it need an explicit field.

---

## Accessibility

- Each widget button already has a `title` attribute. Add `aria-grabbed` during drag.
- Keyboard reorder (stretch goal): `Ctrl+Left` / `Ctrl+Right` on a focused widget moves it one slot, same `setconfig` call.

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/window/action-widgets.tsx` | Add drag state, `getSortedWidgets`, drop handlers, drop indicator |
| `frontend/app/window/action-widgets.scss` | `.widget-dragging` opacity, drop indicator line, drag cursor |
| `frontend/app/types/` or `gotypes.d.ts` | Add `"widget:order"?: string[]` to `SettingsType` |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Optional: add typed `widget_order` field (may not be needed) |

No new RPC handlers. No migrations.

---

## Out of Scope

- Reordering across multiple rows (widget bar is single-row only).
- Drag-to-remove (use right-click hide instead).
- Touch/mobile drag (Tauri desktop app, mouse only).
- Animated reorder of hidden widgets (hidden widgets are invisible; their order slot is still preserved in the array).

---

## Open Questions

1. Should `swarm` (hidden) appear as a draggable slot in a "hidden widgets" overflow area, or just stay invisible and keep its array position silently?
2. Should the reset-to-default option (right-click → "Reset widget order") delete `widget:order` from settings, reverting to `display:order`?
