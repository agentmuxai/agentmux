# Cross-Window Drag: Wiring Analysis

**Date:** 2026-03-30
**Status:** historical — records the wiring analysis as of 2026-03-30 (backend
complete, frontend needing 4 targeted fixes at that time). Not re-verified against
current code; treat the body as a snapshot, not a claim about today.

---

## Architecture Overview

Cross-window drag lets users drag a pane or tab out of one AgentMux window to create a new window (tear-off) or drop it into an existing window.

```
User drags pane outside window
  → CrossWindowDragMonitor detects dragend (or OLE fallback timer)
  → startCrossDrag() → backend creates DragSession
  → updateCrossDrag() → backend hit-tests all windows via Win32 GetWindowRect
  → backend broadcasts "cross-drag-update" to all windows
  → target window's DragOverlay shows drop indicator
  → user releases mouse
  → completeCrossDrag() → backend broadcasts "cross-drag-end"
  → target handles drop (MoveBlockToTab) or tear-off (TearOffBlock + openWindowAtPosition)
```

---

## Backend (CEF Host) — FULLY IMPLEMENTED

All 10 commands are routed in `agentmux-cef/src/ipc.rs:235-245` and implemented in `agentmux-cef/src/commands/drag.rs`:

| Command | Implementation | Status |
|---------|---------------|--------|
| `start_cross_drag` | UUID session, stores in `AppState.active_drag`, broadcasts `cross-drag-start` | Done |
| `update_cross_drag` | Hit-tests all browser HWNDs via `GetWindowRect`, broadcasts `cross-drag-update` | Done |
| `complete_cross_drag` | Takes session, broadcasts `cross-drag-end` with result=drop/tearoff | Done |
| `cancel_cross_drag` | Clears session, broadcasts `cross-drag-end` with result=cancel | Done |
| `get_cursor_point` | Win32 `GetCursorPos` | Done |
| `get_mouse_button_state` | Win32 `GetAsyncKeyState(VK_LBUTTON)` | Done |
| `set_drag_cursor` | `SetSystemCursor` — replaces no-drop with crosshair | Done |
| `restore_drag_cursor` | `SystemParametersInfoW(SPI_SETCURSORS)` | Done |
| `release_drag_capture` | `ReleaseCapture` + `WM_CANCELMODE` on all child windows | Done |
| `open_window_at_position` | Creates new CEF browser at screen coords with IPC creds + workspaceId in URL | Done |
| `set_js_drag_active` | No-op on Windows (Linux GTK guard) | Done |

State tracking: `DragSession` struct in `agentmux-cef/src/state.rs` with drag_id, drag_type (Pane|Tab), source info, payload.

Hit-testing: `hit_test_windows()` iterates `AppState.browsers` HashMap, gets HWND from each `browser.host()`, checks `GetWindowRect` bounds.

Event broadcasting: `events::emit_event_all_windows()` calls `execute_javascript()` on every browser instance, dispatching a `CustomEvent` on `window`.

---

## Frontend CEF API (`frontend/util/cef-api.ts`) — PARTIALLY WIRED

### Duplicate keys in the API object

The `createCefApi()` function returns an object with **two sets** of drag methods:

**First set (lines 423-433) — STUBS (no-ops):**
```typescript
setJsDragActive: async (_active: boolean) => {},
startCrossDrag: async () => "",
updateCrossDrag: async () => null as string | null,
completeCrossDrag: async () => {},
cancelCrossDrag: async (_dragId: string) => {},
openWindowAtPosition: async () => "",
setDragCursor: async () => {},
restoreDragCursor: async () => {},
releaseDragCapture: async () => {},
getMouseButtonState: async () => false,          // ← NO OVERRIDE
```

**Second set (lines 540-579) — REAL implementations via invokeCommand():**
```typescript
startCrossDrag: async (dragType, sourceWindow, ...) => invokeCommand("start_cross_drag", {...}),
updateCrossDrag: async (dragId, screenX, screenY) => invokeCommand("update_cross_drag", {...}),
completeCrossDrag: async (dragId, targetWindow, ...) => invokeCommand("complete_cross_drag", {...}),
cancelCrossDrag: async (dragId) => invokeCommand("cancel_cross_drag", {...}),
openWindowAtPosition: async (screenX, screenY, workspaceId?) => invokeCommand("open_window_at_position", {...}),
setDragCursor: async () => invokeCommand("set_drag_cursor"),
restoreDragCursor: async () => invokeCommand("restore_drag_cursor"),
releaseDragCapture: async () => invokeCommand("release_drag_capture"),
```

In JS, **last duplicate key wins**. So the real implementations override the stubs for everything **except `getMouseButtonState`**, which has no second definition.

### Missing from CEF API

| Method | Status | Needed By |
|--------|--------|-----------|
| `getMouseButtonState` | **Stub only** (returns `false`) | `CrossWindowDragMonitor.win32.tsx:70` |
| `getCursorPoint` | **Not in API at all** | `CrossWindowDragMonitor.win32.tsx:148` |

---

## Frontend CrossWindowDragMonitor (`frontend/app/drag/CrossWindowDragMonitor.win32.tsx`) — BROKEN ON CEF

The monitor imports directly from `@tauri-apps/api/core` instead of using `getApi()`:

### Problem 1: `get_mouse_button_state` (line 70-71)
```typescript
const { invoke } = await import("@tauri-apps/api/core");
isButtonPressed = await invoke<boolean>("get_mouse_button_state");
```
**Fails on CEF:** `@tauri-apps/api/core` is not available. Should use `getApi().getMouseButtonState()`.

### Problem 2: `get_cursor_point` (line 147-149)
```typescript
const { invoke } = await import("@tauri-apps/api/core");
cursorPoint = await invoke<{ x: number; y: number }>("get_cursor_point");
```
**Fails on CEF:** Same issue. Should use `getApi().getCursorPoint()`.

### DragOverlay (`frontend/app/drag/DragOverlay.tsx`) — WORKS

Already uses `getApi().listen()` for all event subscriptions. No Tauri-specific imports. Will work on both hosts.

---

## Required Changes

### Fix 1: Add `getMouseButtonState` real implementation to `cef-api.ts`

```typescript
// Replace stub at line 433 or add override in the second block (~line 578)
getMouseButtonState: async () => {
    return await invokeCommand<boolean>("get_mouse_button_state");
},
```

### Fix 2: Add `getCursorPoint` to `cef-api.ts`

```typescript
getCursorPoint: async () => {
    return await invokeCommand<{ x: number; y: number }>("get_cursor_point");
},
```

Also add to `tauri-api.ts` if not present (should wrap `invoke("get_cursor_point")`).

### Fix 3: Update `CrossWindowDragMonitor.win32.tsx` to use `getApi()`

Replace direct Tauri imports:

```typescript
// BEFORE (line 68-73):
const { invoke } = await import("@tauri-apps/api/core");
isButtonPressed = await invoke<boolean>("get_mouse_button_state");

// AFTER:
isButtonPressed = await getApi().getMouseButtonState();
```

```typescript
// BEFORE (line 146-149):
const { invoke } = await import("@tauri-apps/api/core");
cursorPoint = await invoke<{ x: number; y: number }>("get_cursor_point");

// AFTER:
cursorPoint = await getApi().getCursorPoint();
```

### Fix 4: Clean up dead stubs in `cef-api.ts`

Remove lines 423-433 (the first set of stub definitions). They're all overwritten by the real implementations at lines 540-579, except `getMouseButtonState` which should be added there instead.

---

## Files to Change

| File | Change | Risk |
|------|--------|------|
| `frontend/util/cef-api.ts:423-433` | Remove stubs, add `getMouseButtonState` + `getCursorPoint` to real block | Low |
| `frontend/util/tauri-api.ts` | Add `getCursorPoint` + `getMouseButtonState` if missing | Low |
| `frontend/app/drag/CrossWindowDragMonitor.win32.tsx:68-75` | Replace `import("@tauri-apps/api/core")` with `getApi()` | Low |
| `frontend/app/drag/CrossWindowDragMonitor.win32.tsx:146-149` | Replace `import("@tauri-apps/api/core")` with `getApi()` | Low |
| `frontend/types/custom.d.ts` | Add `getCursorPoint` + `getMouseButtonState` to `AppApi` interface | Low |

---

## Testing Plan

1. `task dev` — launch CEF host
2. Open a terminal pane, drag it out of the window
3. Expect: new window appears at cursor position with the terminal pane (scrollback preserved, PTY continues)
4. Open two windows, drag a pane from one to the other
5. Expect: pane moves to target window, DragOverlay shows during hover
6. Drag a tab out of a window
7. Expect: new window with that tab's full layout

### Fallback timer test (Windows-specific)
1. Drag a pane outside the window and release over Explorer (not another AgentMux window)
2. Wait 800ms — OLE fallback should fire
3. Expect: `get_mouse_button_state` returns false → tear-off triggers

---

## Related Specs

- `docs/specs/cef-drag-window-management.md` — full 5-system analysis
- `docs/specs/pane-popout-to-new-window.md` — pop-out button design (magnify → popout)
- `docs/retro/2026-03-20-secondary-window-dnd-regression.md` — WebView2 DnD fix history and Pragmatic DnD migration
