# Tab Drag Cursor: Circle-Slash → Plus Sign

**Date:** 2026-03-24
**Status:** Ready to fix

---

## Symptom

When dragging a tab out of the tab bar and over the desktop (outside the AgentMux window), the cursor shows a **circle-slash** (🚫 "forbidden/no-drop"). It should show a **plus sign** (➕ "copy") to signal that dropping will tear off the tab into a new window.

---

## Root Cause

The HTML5 Drag-and-Drop API uses two properties to determine the cursor during a drag:

| Property | Set by | Controls |
|---|---|---|
| `dataTransfer.effectAllowed` | Source (at `dragstart`) | Which effects the source permits |
| `dataTransfer.dropEffect` | Target (at `dragover`) | Which effect the target accepts |

**The cursor shown = intersection of `effectAllowed` and `dropEffect`.**

When the cursor is over the desktop (outside WebView2), there is **no drop target** to set `dropEffect`. The browser/OLE falls back to `dropEffect = "none"`, which renders as the forbidden cursor — regardless of what effects are technically allowed.

The fix is to set `effectAllowed = "copy"` at `dragstart`. When `effectAllowed` is constrained to **only** `"copy"`, Windows OLE uses the copy cursor (arrow + +) even over non-WebView2 areas, because it communicates that the drag *always* means "create something new here."

---

## Why Neither Pane Nor Tab Drags Set effectAllowed

Both drag systems use the Atlaskit pragmatic-drag-and-drop `draggable()` adapter:

- **Tab drag:** `frontend/app/tab/droppable-tab.tsx` — `draggable({ element, onDragStart, ... })`
- **Pane drag:** `frontend/layout/lib/TileLayout.win32.tsx` — same adapter on the header element

The Atlaskit PDND `draggable()` API **does not expose `dataTransfer` directly**. The only drag-preview hook it offers is `onGenerateDragPreview({ nativeSetDragImage })` — which wraps `setDragImage` but not `effectAllowed`. So `effectAllowed` is never set, defaulting to `"uninitialized"` → browser treats it as `"all"` → OLE shows forbidden cursor over non-drop-target areas.

---

## Event Timing — Why a Native Listener Works

Atlaskit registers its `dragstart` handler on the **document** in **capture phase** (`{ capture: true }`). This fires as the event travels down the DOM.

A **bubble-phase** `dragstart` listener added directly on the draggable element fires after Atlaskit has already committed the drag. `dataTransfer.effectAllowed` can still be set at this point — the spec allows setting it any time during the `dragstart` event, until the event handler returns. So:

```
dragstart fires
  → document [capture] → Atlaskit sets up drag state     ← Atlaskit
  → element [bubble]   → we set effectAllowed = "copy"   ← our fix
dragstart returns → effectAllowed locked in for the lifetime of the drag
```

---

## Files to Change

### 1. `frontend/app/tab/droppable-tab.tsx` (primary fix)

In `onMount`, after registering the Atlaskit `draggable`, add a native listener:

```typescript
// Set effectAllowed = "copy" so Windows OLE shows the plus-sign cursor
// when dragging outside the WebView2 window (tearoff intent).
const handleNativeDragStart = (e: DragEvent) => {
    if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = "copy";
    }
};
tabWrapRef.addEventListener("dragstart", handleNativeDragStart);

onCleanup(() => {
    tabWrapRef.removeEventListener("dragstart", handleNativeDragStart);
    tabWrapperRefs.delete(props.tabId);
    cleanupDraggable();
});
```

> **Note:** Move `tabWrapperRefs.delete` and `cleanupDraggable()` into this single `onCleanup` block (they're currently in a separate `onCleanup`). Or keep two `onCleanup` calls — both are fine in SolidJS.

### 2. `frontend/layout/lib/TileLayout.win32.tsx` (pane drag — same issue)

Inside the `register()` function, after the `cleanupFn = draggable(...)` call, add:

```typescript
const handleNativeDragStart = (e: DragEvent) => {
    if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = "copy";
    }
};
handle.addEventListener("dragstart", handleNativeDragStart);

// Update cleanupFn to also remove the native listener:
const atlasDragCleanup = cleanupFn;
cleanupFn = () => {
    atlasDragCleanup?.();
    handle.removeEventListener("dragstart", handleNativeDragStart);
};
```

---

## What Doesn't Need to Change

- `CrossWindowDragMonitor.win32.tsx` — the monitor's fallback/tearoff logic is correct; only the cursor UX is wrong
- `tabbar.tsx` `monitorForElements` — this is the in-window drop handler; not involved in cursor appearance
- Any CSS — cursor during drag is controlled by the OS/browser via `effectAllowed`/`dropEffect`, not CSS

---

## Testing

1. Drag a tab out of the tab bar past the window edge → cursor should show **arrow + plus** (not circle-slash)
2. Drop on desktop → tab should tear off into a new window (existing behavior unchanged)
3. Drag a tab within the tab bar → insertion gap animation and reorder should work as before
4. Drag a pane header outside the window → should also show arrow + plus after TileLayout fix

---

## Confidence

**High.** The `effectAllowed = "copy"` technique for OLE/WebView2 is well-established. The event timing analysis (Atlaskit capture → our bubble) is reliable. The tearoff logic itself is untouched.
