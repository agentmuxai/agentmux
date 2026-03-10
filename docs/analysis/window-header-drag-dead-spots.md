# Window Header Drag Dead Spots — Root Cause Analysis

**Date:** 2026-03-09
**Branch:** `agenta/drag-drop-files`
**Files involved:** `useWindowDrag.ts`, `windowdrag.tsx`, `window-header.tsx`, `tabbar.tsx`

---

## Symptoms

1. Cursor shows `grab` hand (cosmetic, already fixed in uncommitted changes → `cursor: default`).
2. Many dead spots on the header — drag sometimes works, sometimes does nothing.
3. Behavior feels non-deterministic / race-condition-like.

---

## Architecture Overview

```
<div.window-header  [onMouseDown → startDragging()]>
    <WindowDrag.left  [data-tauri-drag-region=true, onMouseDown → startDragging()]>
        — 10px wide strip —
    </WindowDrag>
    <TabBar  [data-tauri-drag-region="false"]>
        <DroppableTab> (useDrag + useDrop from react-dnd, draggable="true") </DroppableTab>
        ...
    </TabBar>
    <SystemStatus>
        <div.window-action-buttons  [data-tauri-drag-region="false"]>
```

---

## Root Causes (ranked by impact)

### 1. Dual drag mechanism — the core race condition

`useWindowDrag()` returns **both**:
- `dragProps = { "data-tauri-drag-region": true }` — Tauri's native WebView-level drag
- `onMouseDown` → `getCurrentWindow().startDragging()` — async JS IPC call

These are **two completely separate Tauri drag mechanisms**, and `WindowDrag` uses both simultaneously. When you click `WindowDrag.left`:

1. Tauri's WebView intercepts `mousedown` at OS level, sees `data-tauri-drag-region="true"`, calls OS window-move API **immediately** (synchronous, pre-JS).
2. React's `onMouseDown` bubbles up → `startDragging()` IPC call fires **asynchronously**.
3. By the time the IPC resolves, the OS drag may already be in flight.

This creates a race: sometimes the OS drag wins cleanly; sometimes the second `startDragging()` call collides with it and resets the drag state, causing the window to snap back or refuse to drag.

### 2. Double `startDragging()` call via event bubbling

When clicking `WindowDrag.left`:
1. `WindowDrag.onMouseDown` fires → `startDragging()` *(first call)*
2. Event bubbles to `window-header.onMouseDown` → `startDragging()` *(second call)*

`DRAG_EXCLUDE_SELECTOR` checks for `button, input, a, [data-tauri-drag-region="false"]`. A `div.window-drag` matches none of these, so the outer handler always fires too.

**Result:** Two JS `startDragging()` IPC calls + one Tauri native drag = three drag initiations for a single click.

### 3. `WindowDrag.left` is only 10px wide

```scss
// window-header.scss
--default-indent: 10px;
.window-drag { width: var(--default-indent); }
```

The only element guaranteed draggable (with `data-tauri-drag-region`) is a **10px strip** on the far left. The rest of the header (excluding TabBar and SystemStatus) relies entirely on `window-header.onMouseDown` → `startDragging()`. This is an async IPC call that can fail silently (see `.catch(() => {})`), making large parts of the header unreliable.

### 4. React-DND `draggable="true"` interferes with Tauri drag

`DroppableTab` registers tabs as HTML5 drag sources via `useDrag()`. React-DND's HTML5 backend sets `draggable="true"` on the tab DOM elements. When the browser sees `draggable="true"`:

- The browser's native HTML5 drag API activates on `mousedown`.
- This competes with Tauri's WebView drag intercept.
- Even though `tab-bar` has `data-tauri-drag-region="false"`, the JS `window-header.onMouseDown` still fires (TabBar has no `stopPropagation()`), calling `startDragging()` right as the HTML5 drag is being set up.
- Outcome: non-deterministic — whichever mechanism "wins" the event loop iteration determines if the window drags or the tab starts an HTML5 drag.

### 5. Area between tabs and SystemStatus has no `data-tauri-drag-region`

After the uncommitted changes, `window-header` no longer has `{...dragProps}` (i.e., no `data-tauri-drag-region`). The empty horizontal space between the last tab and `SystemStatus` is only covered by `window-header.onMouseDown` → `startDragging()`. This async path fails silently more often than the Tauri-native path.

---

## Why It Feels Like a Race Condition

It literally is one. `startDragging()` is an async Tauri IPC call:
```ts
getCurrentWindow().startDragging().catch(() => {});
```
The `.catch(() => {})` silently swallows all errors. If the call arrives after Tauri has already committed the window to a drag (or after the user has released the mouse), it fails silently. The timing depends on:
- Current JS event loop pressure
- IPC serialization latency
- Whether Tauri's native handler already grabbed the pointer

---

## Fix Strategy

**Pick ONE drag mechanism and remove the other everywhere.**

Tauri's `data-tauri-drag-region` is handled at the WebView/OS level, before JS runs. It is synchronous, reliable, and respects element nesting (innermost attribute wins). JS `startDragging()` is async, fire-and-forget, and conflicts with native drag.

### Recommended Fix

**1. Add `data-tauri-drag-region` to the outer `window-header` div (non-Linux)**

This makes the entire header draggable by default. Child elements with `data-tauri-drag-region="false"` (TabBar, SystemStatus buttons) will correctly block drag in their areas — Tauri already handles this hierarchy correctly.

**2. Remove `onMouseDown` from `window-header` entirely**

No JS `startDragging()` needed once the `data-tauri-drag-region` covers the header.

**3. Remove `onMouseDown` from `WindowDrag` component**

`WindowDrag` only needs `data-tauri-drag-region="true"` for its function. The JS handler is redundant and harmful.

**4. Keep `data-tauri-drag-region="false"` on `TabBar` and `SystemStatus`**

Already correct. These block Tauri from initiating drag when clicking tabs or window controls.

### Linux caveat

On Linux, `data-tauri-drag-region` causes a GTK pointer grab that swallows clicks. The comment in `useWindowDrag.ts` says "drag.rs handles it natively". So Linux should get `{}` dragProps (no attribute) and no JS handler either — Linux drag works through the GTK motion-notify at the Tauri backend level.

### Changes summary

| File | Change |
|------|--------|
| `useWindowDrag.ts` | Remove `onMouseDown` from return value (or make no-op on all platforms) |
| `windowdrag.tsx` | Remove `onMouseDown` prop, keep `{...dragProps}` |
| `window-header.tsx` | Add `{...dragProps}` back to outer div, remove `onMouseDown` |

### Alternative: Pure JS `startDragging()` everywhere

If `data-tauri-drag-region` causes other issues (e.g., blocking right-click context menus), can go the other direction: remove all `data-tauri-drag-region` attributes, put a single `onMouseDown` on `window-header` only, use `stopPropagation()` in TabBar/SystemStatus to prevent bubbling. This is more fragile because it's async and relies on correct propagation gating.

---

## Decision

`data-tauri-drag-region` approach is recommended. It's synchronous, native, tested by Tauri, and the nesting semantics are already correctly established in TabBar and SystemStatus.
