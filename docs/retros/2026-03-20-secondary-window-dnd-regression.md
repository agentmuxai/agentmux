# Retro: Secondary Window DnD + Window Drag Regression
Date: 2026-03-20
Author: AgentA

---

## What Happened

### Original Problem
Drag-and-drop (tab reorder, pane move) did not work in secondary/tertiary app windows. First window worked fine. Logs showed `tab-drag started` firing repeatedly with no `tab drop` — the classic "stuck drag" pattern indicating `dragend` never fired.

### Root Cause of Original Bug
The main window config (`tauri.conf.json`) had `"dragDropEnabled": false`, which maps to `disable_drag_drop_handler()` in Tauri's Rust API. This disables WebView2's file-drop interception, which is required on Windows for HTML5 drag-and-drop to work in the webview.

Secondary windows were created via `WebviewWindowBuilder` in `window.rs` and `drag.rs` without this setting, so they got the default (`true` = file drop handler active = WebView2 intercepts drag events = HTML5 DnD broken).

### Fix Attempt 1 (Wrong)
Searched for the equivalent Rust builder method. Found two candidates:
- `drag_and_drop(bool)` — on `window_builder`
- `disable_drag_drop_handler()` — on `webview_builder`

Chose `.drag_and_drop(false)` without verifying which one maps to `dragDropEnabled`.

**This was wrong.** `drag_and_drop(false)` disables the **window-level drag system**, which includes the title-bar window-move behavior driven by `data-tauri-drag-region`. Result: secondary windows could no longer be moved by dragging the title bar.

### Regression Introduced
After applying `.drag_and_drop(false)`:
- Window drag (title bar move) broken in secondary windows
- Pane DnD also still broken (wrong method didn't fix the underlying issue)

---

## Why I Got It Wrong

### Failure mode: guessing from method names
Both methods sound like they disable drag. I saw two options and picked one by name similarity without:
1. Reading the Tauri docs or source for each method's actual effect
2. Understanding the distinction between window-builder and webview-builder
3. Testing before committing to a build

### The actual distinction

| Method | Target | Effect |
|--------|--------|--------|
| `.drag_and_drop(false)` | `window_builder` | Disables window-level drag (title bar move, resize) |
| `.disable_drag_drop_handler()` | `webview_builder` | Disables WebView2 file-drop handler — required for HTML5 DnD on Windows |

`tauri.conf.json`'s `"dragDropEnabled": false` maps to `disable_drag_drop_handler()`, NOT `drag_and_drop(false)`.

### Key signal I missed
The Tauri source grep showed:
```
pub fn drag_and_drop(mut self, enabled: bool) -> Self {
    self.window_builder = self.window_builder.drag_and_drop(enabled);

pub fn disable_drag_drop_handler(mut self) -> Self {
    self.webview_builder = self.webview_builder.disable_drag_drop_handler();
```

`window_builder` vs `webview_builder` is the critical tell. I didn't stop to read this distinction carefully.

---

## What the Correct Fix Is

In `src-tauri/src/commands/window.rs` and `src-tauri/src/commands/drag.rs`, when creating secondary windows:

```rust
// WRONG — disables window drag (title bar move)
.drag_and_drop(false)

// CORRECT — disables WebView2 file-drop handler, allows HTML5 DnD
.disable_drag_drop_handler()
```

This exactly mirrors what `"dragDropEnabled": false` does in `tauri.conf.json` for the main window.

---

## Plan Forward

### Immediate (this session)
1. ✅ Changed `.drag_and_drop(false)` → `.disable_drag_drop_handler()` in both files
2. Rebuild backend (Rust compile)
3. Bump version to 0.32.57
4. Build portable, copy to desktop, test:
   - Window drag works in all windows
   - Tab DnD works in secondary windows
   - Pane DnD works in secondary windows

### Process Changes
1. **Before using any API method found by grep:** Read the source context — specifically note whether the method touches `window_builder` vs `webview_builder`. These have completely different semantics.
2. **When adding platform-specific flags:** Cross-check against the JSON config equivalent (`tauri.conf.json`) to confirm the mapping is correct.
3. **Validate before full build:** Run `cargo check` or a minimal compile before triggering `task package:portable`.

### Code Comment (add to prevent future confusion)
In both `window.rs` and `drag.rs`, add a comment explaining WHY `disable_drag_drop_handler()` is needed:

```rust
// Required for HTML5 drag-and-drop to work in the webview on Windows.
// Without this, WebView2 intercepts drag events for OS file drops,
// which prevents pragmatic-dnd (and any HTML5 DnD) from receiving dragend.
// Mirrors "dragDropEnabled": false in tauri.conf.json for the main window.
.disable_drag_drop_handler()
```

---

## Log Access Reference
Backend logs (for future debugging):
- Dev builds: `%LOCALAPPDATA%\ai.agentmux.app.dev\instances\v<VERSION>\logs\agentmuxsrv-v<VERSION>.log.<DATE>`
- Frontend `[fe]` logs: go to the `task dev` terminal stdout (not the log file)
- To see frontend logs in portable builds: check agentmuxsrv log file, search for `[fe]`
