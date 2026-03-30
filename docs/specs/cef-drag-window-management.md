# Spec: CEF Drag, Drop, and Window Management

**Date:** 2026-03-29
**Status:** Analysis complete — phased implementation plan

---

## Problem

AgentMux has five intertwined drag/window systems that all depend on Tauri APIs.
The CEF host needs equivalents for each. They cannot be fixed independently —
they share state (`DragItemPayload`), coordinate across windows, and use
platform-specific OS APIs that differ between Tauri (WebView2) and CEF (Chromium).

---

## The Five Systems

### 1. Window Dragging (Title Bar)

**How it works:** Frameless window — user drags the custom title bar.

| | Tauri | CEF (current) | CEF (needed) |
|---|---|---|---|
| Mechanism | `data-tauri-drag-region` attr | `start_window_drag` IPC | Same IPC, wider drag area |
| API | WebView2 handles natively | `WM_NCLBUTTONDOWN` via Win32 | Same |
| Drag area | Entire `.window-header` | Only `.window-drag` indent div | Entire header |

**Issue:** `useWindowDrag.win32.ts` returns `onMouseDown` for CEF, but only
the `WindowDrag` component uses it (small indent area). The header div itself
gets `data-tauri-drag-region` spread via `dragProps` which CEF ignores.

**Fix:** Change `useWindowDrag.win32.ts` to return BOTH `data-tauri-drag-region`
(for Tauri) and `onMouseDown` (for CEF) simultaneously. Tauri ignores onMouseDown
on drag regions (the native handler fires first). CEF ignores data-tauri-drag-region.

```typescript
export function useWindowDrag() {
    return {
        dragProps: {
            "data-tauri-drag-region": true,
            onMouseDown: (e: MouseEvent) => {
                if (e.button !== 0) return;
                if (detectHost() !== "cef") return;
                invokeCommand("start_window_drag").catch(() => {});
            },
        },
    };
}
```

The `dragProps` are spread on:
- `.window-header` root div (window-header.tsx:38)
- `WindowDrag` left/right indent divs (windowdrag.tsx:26)
- System status bar areas (system-status.tsx:73, 108)

Child elements with `data-tauri-drag-region="false"` prevent drag in Tauri.
For CEF, those children naturally don't have `onMouseDown` so they don't trigger
drag. Interactive children (buttons, tabs) have their own click handlers that
call `e.stopPropagation()`.

### 2. Tab Dragging (In-Window Reorder)

**How it works:** Pragmatic-dnd (Atlassian's drag library) handles tab reorder
within a single window. No Tauri APIs needed.

| | Tauri | CEF |
|---|---|---|
| Drag library | `@atlaskit/pragmatic-drag-and-drop` | Same |
| Events | HTML5 DnD | HTML5 DnD |
| Status | Working | **Working** |

**Only issue:** `droppable-tab.tsx` calls `getApi().setJsDragActive(true/false)`
on drag start/end. This is stubbed in CEF (no-op). It's only needed for Linux
GTK guard — harmless to skip.

**No changes needed.**

### 3. Pane/Block Dragging (In-Window Split/Move)

**How it works:** Pragmatic-dnd on tile layout elements. Platform-split files
handle differences between WebView2/WKWebView/WebKitGTK.

| | Tauri | CEF |
|---|---|---|
| Drag library | Pragmatic-dnd | Same |
| Drag handle | Win32: whole tile, macOS: header, Linux: whole tile | Same as Win32 |
| Preview | `html-to-image` → PNG preview | Same |
| Status | Working | **Working** |

**No changes needed for in-window pane dragging.**

### 4. Cross-Window Drag (Pane/Tab Tear-Off)

**How it works:** When a drag leaves the source window, the frontend detects it
via `dragend` event, then coordinates with the backend to hit-test across windows,
optionally creating a new window (tear-off) or dropping into an existing one.

| | Tauri | CEF |
|---|---|---|
| Session management | `DragSession` in Rust state | **Stubbed** |
| Hit-testing | `hit_test_windows()` via Tauri window API | **Stubbed** |
| Event broadcast | Tauri emit to all windows | **Stubbed** |
| Window creation | `WebviewWindow::builder()` | **Stubbed** |
| Multi-window | Native (Tauri manages windows) | **Not implemented** |

**10 stubbed commands:**
`start_cross_drag`, `update_cross_drag`, `complete_cross_drag`,
`cancel_cross_drag`, `set_drag_cursor`, `restore_drag_cursor`,
`release_drag_capture`, `get_cursor_point`, `get_mouse_button_state`,
`set_js_drag_active`

**This is Phase 3 work.** Requires CEF multi-window support first.

### 5. File Drag-and-Drop (OS Files → Terminal)

**How it works:** Dragging files from Explorer onto a terminal pane copies them
to the terminal's working directory.

| | Tauri | CEF |
|---|---|---|
| Event source | `getCurrentWebview().onDragDropEvent()` | HTML5 `ondrop` |
| File paths | Full OS paths from Tauri event | File names only (browser security) |
| Copy command | `invoke("copy_file_to_dir")` | **Not possible without paths** |
| Status | Working | **Partial** (overlay works, copy doesn't) |

**CEF fix:** Implement `CefDragHandler::OnDragEnter()` in Rust to extract full
file paths from the OS drag data (`IDataObject` on Windows). Store paths in
state, expose via `get_drag_file_paths` IPC command. Frontend calls it on drop.

---

## Implementation Phases

### Phase 2.5 (This PR — Quick Wins)

**Window dragging fix:**
- Update `useWindowDrag.win32.ts` to return both `data-tauri-drag-region` and
  `onMouseDown` — works for both hosts simultaneously
- Remove the `-webkit-app-region` CSS approach (doesn't work in Views/Alloy)

**Window buttons fix:**
- Already fixed: `close_window`, `minimize_window`, `maximize_window` now use
  `find_own_top_level_window()` + Win32 API

**File drop paths (CEF):**
- Add `CefDragHandler` to `client.rs` that captures file paths on `OnDragEnter`
- Store in `AppState.drag_file_paths: Mutex<Vec<String>>`
- Add `get_drag_file_paths` IPC command
- Frontend `term.tsx` CEF drop handler calls `get_drag_file_paths` then
  `invokeCommand("copy_file_to_dir")` (this command also needs CEF implementation)
- Add `copy_file_to_dir` to CEF `commands/platform.rs` (simple `std::fs::copy`)

**Estimated effort:** 2-3 hours

### Phase 3 (Multi-Window + Cross-Window Drag)

**Prerequisites:**
1. CEF multi-window support (multiple browser instances in one process)
2. Window management (create, list, focus, close windows)
3. Inter-window event broadcasting

**Cross-window drag implementation:**
1. Port `DragSession` struct and state management to `agentmux-cef/src/drag.rs`
2. Implement `hit_test_windows()` using Win32 `EnumWindows` + `GetWindowRect`
3. Implement `start/update/complete/cancel_cross_drag` commands
4. Broadcast events via `execute_javascript` to each browser window
5. Port cursor management (`set_drag_cursor`, `restore_drag_cursor`)
6. Port Windows OLE fallback (`get_mouse_button_state`, `release_drag_capture`)

**Window creation:**
1. Implement `open_new_window` — spawn new CEF browser in same process
2. Implement `open_window_at_position` — same with specific screen coordinates
3. Implement `list_windows` — return all managed window labels
4. Implement `focus_window` — `SetForegroundWindow` on HWND

**Estimated effort:** 2-3 days

### Phase 4 (Polish)

- Tab tear-off animation (ghost tab follows cursor)
- Pane tear-off with preview
- Cross-window pane drop with split direction
- Edge docking (snap to screen edges)

---

## Direct Tauri Imports to Abstract

Files that still import directly from `@tauri-apps/*` (not through `ipc.ts`):

| File | Import | CEF Equivalent |
|------|--------|----------------|
| `term.tsx` | `@tauri-apps/api/webview` | `detectHost()` guard (done) |
| `term.tsx` | `@tauri-apps/api/core` | Dynamic import behind guard (done) |
| `CrossWindowDragMonitor.win32.tsx` | `@tauri-apps/api/core` | Phase 3 |
| `CrossWindowDragMonitor.linux.tsx` | `@tauri-apps/api/core` | Phase 3 |
| `action-widgets.tsx` | — | Switched to `invokeCommand` (done) |
| `tauri-api.ts` | `@tauri-apps/plugin-opener` | `open_external` IPC (partially done) |
| `clipboard.ts` | `@tauri-apps/plugin-clipboard-manager` | `navigator.clipboard` API (works in CEF) |
| `notification.ts` | `@tauri-apps/plugin-notification` | `Notification` API (works in CEF) |
| `log-pipe.ts` | `@tauri-apps/api/core` | `invokeCommand` (already routed via ipc.ts) |

---

## Architecture Decision: Shared DragProps

The key insight: `data-tauri-drag-region` and `onMouseDown` can coexist on the
same element. Tauri's native handler fires before JS and consumes the event.
CEF's JS handler fires normally. No platform-split needed for the drag props —
just return both from `useWindowDrag`.

For tab and pane drag: Pragmatic-dnd uses HTML5 DnD which works in both hosts.
No changes needed.

For cross-window drag: The `CrossWindowDragMonitor` platform files already handle
per-platform differences. CEF would need its own `.cef.tsx` variant (Phase 3).

---

## Files to Change (Phase 2.5)

| File | Change |
|------|--------|
| `frontend/app/hook/useWindowDrag.win32.ts` | Return both data-tauri-drag-region + onMouseDown |
| `frontend/app/window/window-header.win32.scss` | Remove -webkit-app-region (doesn't work) |
| `agentmux-cef/src/commands/platform.rs` | Add `copy_file_to_dir` command |
| `agentmux-cef/src/client.rs` | Add `CefDragHandler` for file path extraction |
| `agentmux-cef/src/state.rs` | Add `drag_file_paths: Mutex<Vec<String>>` |
| `agentmux-cef/src/ipc.rs` | Route `copy_file_to_dir`, `get_drag_file_paths` |
| `frontend/app/view/term/term.tsx` | CEF drop handler: get paths then copy |
