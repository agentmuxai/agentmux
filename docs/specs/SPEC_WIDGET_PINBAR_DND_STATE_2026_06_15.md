# Spec: Widget Pin-Bar DnD State Machine — Robustness Rethink

**Date:** 2026-06-15  
**Status:** Proposed  
**Affected file:** `frontend/app/window/action-widgets.tsx`

---

## 1. Bug Description

**Symptom:** After unpinning a widget via right-click → context menu, hovering over the
remaining widgets causes the accent-colored drop indicator to appear. The indicator
disappears only when the user clicks a widget. The indicator should appear exclusively
during an active left-button drag.

**Steps to reproduce:**
1. Right-click any pinned widget → "Unpin from bar"
2. Move the mouse > 5 px from the click origin over any remaining widget slot
3. Observe: drop indicator appears as if dragging
4. Click any widget → indicator disappears

---

## 2. Root-Cause Analysis

### 2a. Primary bug — no button guard

`handlePointerDown` sets `dragStartRef` unconditionally for **all** pointer buttons:

```ts
const handlePointerDown = (key: string, e: PointerEvent) => {
    dragStartRef = { x: e.clientX, y: e.clientY, key };  // fires for button 0, 1, 2
};
```

A right-click (button 2) sets `dragStartRef`. The right-button `pointerup` fires on
the context-menu overlay (which appears synchronously and captures pointer focus), not
on the widget slot. `dragStartRef` is never cleared.

### 2b. Orphaned drag state after context-menu escape

Any interaction that transfers pointer focus away from the widget bar can orphan drag
state without triggering the cleanup path:

| Escape scenario | `pointerup` received by widget slot? |
|---|---|
| Right-click → custom context menu appears | No — overlay captures it |
| Window loses focus (Alt+Tab) | No — browser swallows it |
| OS-level interruption (notification) | No |
| Left-drag → release outside browser window | No |

`handlePointerCancel` handles one OS-level cancel but is not wired to any of the above.

### 2c. Ref/signal duplication — two sources of truth

The drag state is stored in **both** mutable module-level refs *and* reactive signals:

```ts
// Refs (synchronous, for event-handler reads)
let draggingKeyRef: string | null = null;
let dropIndexRef: number | null = null;
let dragStartRef: { x: number; y: number; key: string } | null = null;

// Signals (reactive, for render)
const [draggingKey, setDraggingKey] = createSignal<string | null>(null);
const [dropIndex, setDropIndex] = createSignal<number | null>(null);
```

Any code path that updates refs without updating signals (or vice versa) leaves the
render out of sync with the event system. The current cancel path (`handlePointerCancel`)
correctly resets both; the right-click escape path resets neither.

### 2d. No explicit state machine

The drag lifecycle is encoded as ad-hoc if/else checks inside event handlers rather
than a typed state machine. Valid states are not enumerated, so it is unclear which
transitions are legal, and guards against illegal transitions are missing.

---

## 3. Proposed Redesign

### 3a. Single typed state signal

Replace the three refs + two signals with one `createSignal<DragState>`:

```ts
type DragState =
    | { phase: "idle" }
    | { phase: "pending";  key: string; startX: number; startY: number; pointerId: number }
    | { phase: "dragging"; key: string; dropIndex: number };

const [dragState, setDragState] = createSignal<DragState>({ phase: "idle" });
```

Derived accessors for the render:
```ts
const draggingKey  = () => dragState().phase === "dragging" ? dragState().key      : null;
const dropIndex    = () => dragState().phase === "dragging" ? dragState().dropIndex : null;
```

Benefits:
- Single source of truth — no ref/signal divergence possible
- Solid signals are synchronously readable in event handlers; refs are unnecessary
- Impossible states (e.g. `dropIndex` set while `draggingKey` null) cannot occur

### 3b. Primary-button guard

```ts
const handlePointerDown = (key: string, e: PointerEvent) => {
    if (e.button !== 0) return;  // ignore right-click, middle-click, etc.
    setDragState({ phase: "pending", key, startX: e.clientX, startY: e.clientY, pointerId: e.pointerId });
};
```

Right-clicks never enter `pending` and therefore can never orphan drag state.

### 3c. Global fallback cleanup

While in `pending` or `dragging`, register a **global** `pointerup` listener as a
fallback cleanup. Unregister it when returning to `idle`.

```ts
createEffect(() => {
    const state = dragState();
    if (state.phase === "idle") return;

    const cancel = () => setDragState({ phase: "idle" });
    window.addEventListener("pointerup",     cancel, { capture: true });
    window.addEventListener("pointercancel", cancel, { capture: true });
    window.addEventListener("visibilitychange", cancel);

    onCleanup(() => {
        window.removeEventListener("pointerup",     cancel, { capture: true });
        window.removeEventListener("pointercancel", cancel, { capture: true });
        window.removeEventListener("visibilitychange", cancel);
    });
});
```

This catches:
- Right-click pointerup that fires on an overlay
- Window focus loss (Alt+Tab, OS interruption)
- Release outside the browser window

The capture-phase `pointerup` listener fires before the overlay's bubble-phase listener,
so it reliably catches every pointer release regardless of which element receives it.

### 3d. Clear drag state explicitly on context-menu open

As a belt-and-suspenders measure, `handlePinnedContextMenu` and `handleBarContextMenu`
should reset drag state before opening any menu:

```ts
const handlePinnedContextMenu = (e: MouseEvent, key: string) => {
    e.preventDefault();
    e.stopPropagation();
    setDragState({ phase: "idle" });  // always reset — menu supersedes any pending drag
    armContextMenuDismiss(key);
    // ... rest unchanged
};
```

### 3e. State machine transition table

| From \ Event        | `pointerdown (btn=0)` | `pointermove (>5px)` | `pointerup`   | `pointercancel` / `visibilitychange` |
|---|---|---|---|---|
| `idle`              | → `pending`           | (no-op)              | (no-op)       | (no-op)                              |
| `pending`           | → `pending` (reset)   | → `dragging`         | → `idle`      | → `idle`                             |
| `dragging`          | (blocked by capture)  | update `dropIndex`   | → `idle` + commit reorder | → `idle` (cancel)         |

No transition not listed in this table should occur.

---

## 4. Render Changes

The render `<Show>` conditions need only check the reactive accessors — no logic change:

```tsx
// Before each slot:
<Show when={draggingKey() != null && dropIndex() === idx() && draggingKey() !== key}>
    <div class="action-widget-drop-indicator" />
</Show>

// After last slot:
<Show when={draggingKey() != null && dropIndex() === visiblePinnedWidgets().length}>
    <div class="action-widget-drop-indicator" />
</Show>
```

The slot class is unchanged:
```tsx
class={clsx("action-widget-slot", {
    dragging:       draggingKey() === key,
    "context-active": contextMenuActiveKey() === key,
})}
```

---

## 5. What Is NOT Changing

- The drop indicator's visual design (2px accent line)
- The DRAG_THRESHOLD (5 px)
- The reorder commit logic (splice + `SetConfigCommand`)
- The `contextMenuActiveKey` signal and `armContextMenuDismiss`
- The More dropdown, pin/unpin RPCs, tier collapse logic

---

## 6. Implementation Checklist

- [ ] Replace `dragStartRef` / `draggingKeyRef` / `dropIndexRef` / `draggingKey` signal / `dropIndex` signal with single `DragState` signal
- [ ] Add `e.button !== 0` guard in `handlePointerDown`
- [ ] Add global `pointerup`/`pointercancel`/`visibilitychange` cleanup effect
- [ ] Call `setDragState({ phase: "idle" })` at top of `handlePinnedContextMenu` and `handleBarContextMenu`
- [ ] Update `handlePointerMove` to use `dragState()` reads instead of refs
- [ ] Update `handlePointerUp` to use `dragState()` reads instead of refs
- [ ] Remove now-unnecessary `draggingKeyRef`, `dropIndexRef`, `dragStartRef` let declarations
- [ ] Add changeset: `patch "fix(widgets): repair phantom drop-indicator after right-click unpin"`
- [ ] Verify: right-click unpin → hover → no indicator
- [ ] Verify: left-drag reorder still works end-to-end
- [ ] Verify: drag → Alt+Tab → return → no stale indicator

---

## 7. Minimal Patch (If Full Redesign Is Deferred)

If the full state-machine redesign is deferred, apply this three-line patch as a
stopgap. It fixes the root cause (right-click sets dragStartRef) without touching
the broader architecture:

```diff
 const handlePointerDown = (key: string, e: PointerEvent) => {
+    if (e.button !== 0) return;
     dragStartRef = { x: e.clientX, y: e.clientY, key };
 };
```

And add a reset in `handlePinnedContextMenu`:
```diff
 const handlePinnedContextMenu = (e: MouseEvent, key: string) => {
     e.preventDefault();
     e.stopPropagation();
+    dragStartRef = null; draggingKeyRef = null; dropIndexRef = null;
+    setDraggingKey(null); setDropIndex(null);
     armContextMenuDismiss(key);
```

The stopgap does not address Alt+Tab escape, window-blur escape, or the
ref/signal duplication, but eliminates the reported symptom.
