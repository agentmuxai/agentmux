# Fix Spec: Linux pane tear-off restricted to header only

**Date:** 2026-06-20
**Retro:** `docs/retro/2026-06-20-linux-pane-tearoff-anywhere.md`
**File:** `frontend/layout/lib/TileLayout.linux.tsx`

---

## Problem

On Linux, any click-drag on a pane body (terminal text, browser content, agent transcript, etc.) triggers a pane tear-off. Tear-off must only activate when the drag starts on the pane's title bar / header strip. macOS and Windows are unaffected.

---

## Root cause

`TileLayout.linux.tsx` registers pragmatic-dnd's `draggable()` on `tileNodeRef` — the full pane root element — making the entire pane surface a drag source. Any HTML5 dragstart from anywhere inside the pane fires `onDragStart`, which sets `currentDragPayload` and triggers tear-off on drop.

**Why the whole tile is draggable:** PR #182 removed the `dragHandle` restriction because WebKitGTK does not support `draggable="true"` on a child inside an explicit `draggable="false"` parent. pragmatic-dnd's `dragHandle` option sets exactly that pairing, so drag broke entirely on Linux after PR #180 restricted it to the header. The fix in #182 widened the drag surface back to the whole tile and left a TODO.

**Why it became destructive:** The original regression (whole-tile drag) predated the tear-off feature. PR #188 wired tear-off to the same `onDragStart` hook without a Linux-specific guard, so every incidental drag anywhere now tears the pane off.

---

## Correct fix

Register `draggable()` directly on the header element instead of on the tile root.

**Why this works around the WebKitGTK constraint:** The constraint is specifically about a `draggable="true"` child inside an *explicit* `draggable="false"` parent. pragmatic-dnd's `dragHandle` option creates that explicit `draggable="false"` attribute on the tile root — that's what WebKitGTK rejects. Registering directly on the header element puts `draggable="true"` only on the header; the tile root gets *no* `draggable` attribute (implicitly false). No explicit `draggable="false"` on any ancestor → no WebKitGTK issue.

This is the same pattern Windows uses (`TileLayout.win32.tsx`, PR #1450 area) for the identical WebView2 constraint.

---

## Implementation

### `frontend/layout/lib/TileLayout.linux.tsx`

Replace the `onMount` drag registration block:

**Before:**
```ts
const dragHandleRef = nodeModel.dragHandleRef;
onMount(() => {
    if (!tileNodeRef) return;
    let cleanupFn: (() => void) | null = null;

    const register = () => {
        cleanupFn = draggable({
            element: tileNodeRef,
            dragHandle: undefined,
            canDrag: () => !isEphemeral() && !isMagnified(),
            // ... handlers
        });
        return true;
    };

    register();
    onCleanup(() => cleanupFn?.());
});
```

**After:**
```ts
onMount(() => {
    if (!tileNodeRef) return;
    let cleanupFn: (() => void) | null = null;
    let registeredHandle: HTMLElement | null = null;

    const findHandle = (): HTMLElement | null =>
        tileNodeRef?.querySelector<HTMLElement>('[data-role="block-header"]') ?? null;

    const register = () => {
        const handle = findHandle();
        if (handle === registeredHandle) return;
        if (props.layoutModel.activeDrag()) return;   // never tear down mid-drag

        cleanupFn?.();
        cleanupFn = null;
        registeredHandle = null;

        if (!handle) return;

        registeredHandle = handle;
        cleanupFn = draggable({
            element: handle,                          // header only, not tileNodeRef
            canDrag: () => !isEphemeral() && !isMagnified(),
            // ... same handlers
        });
    };

    register();
    const interval = setInterval(register, 100);     // poll for async header mount
    onCleanup(() => {
        clearInterval(interval);
        cleanupFn?.();
    });
});
```

Key changes:
- `element: tileNodeRef` → `element: handle` (header querySelector result)
- Drop `dragHandle: undefined` (irrelevant when registering on header directly)
- Drop unused `dragHandleRef` variable
- Add `registeredHandle` guard (no-op if element unchanged)
- Add `activeDrag()` guard (no teardown mid-drag)
- Add `setInterval(register, 100)` polling + `clearInterval` in cleanup (header not in DOM at mount time)

---

## Test plan

1. **Drag on pane body** — drag terminal text, browser content, agent transcript. No tear-off must occur. Text selection should work normally.
2. **Drag on pane header** — drag the title bar of a pane. Tear-off must occur; the pane detaches into a floating window.
3. **In-window rearrange** — drag a pane header to another quadrant inside the same window. Pane repositions; no floater created.
4. **Cross-window drag** — drag a pane header to a different AgentMux window. Pane transfers to that window.
5. **Magnified pane** — drag inside or on the header of a magnified pane. No drag/tear-off (magnified panes are locked).
6. **Ephemeral pane** — drag inside or on the header of an ephemeral pane. No drag/tear-off.
7. **After error recovery** — trigger an ErrorBoundary in a pane (bad block data), then drag the header of a recovered pane. Drag still works (polling re-registers on the replacement header element).

---

## No-change scope

- `TileLayout.darwin.tsx` — unaffected (already registers on header element)
- `TileLayout.win32.tsx` — unaffected (same pattern, source of this fix)
- `CrossWindowDragMonitor.linux.tsx` — no change
- Rust / CEF / backend — no change
- CSS / SCSS — no change
