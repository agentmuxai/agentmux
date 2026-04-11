# Window Edge Resize Handlers — Analysis

**Date:** 2026-04-02
**Status:** Not working — windows cannot be resized by dragging edges
**Pane resize:** Works (internal SolidJS drag handles)

## Problem

After the white flash fix (PR #268, #271), window edge resize no longer works. Users cannot drag the window edges or corners to resize. Pane resize (internal dividers) still works — this is purely a Win32 window-level issue.

## What Changed Across PRs #268, #270, #271

**Before (CEF Views mode):** The main window used CEF Views framework (`window_create_top_level`). Resize was handled by CEF internally via the `WindowDelegate::can_resize() -> 1` callback. No Win32 styles needed — CEF managed everything.

**PR #268:** Switched main window from CEF Views to native Win32 mode (`browser_host_create_browser` with `WindowInfo`). This gave us control over `WS_VISIBLE` to fix the white flash. But it also meant resize now depends on Win32 styles (`WS_THICKFRAME`) instead of CEF's internal delegate.

**PR #270:** Removed `--disable-gpu-compositing` (unrelated to resize).

**PR #271:** Removed `WS_THICKFRAME` from window creation (caused white L-shaped border flash). Moved it to `on_load_end` via `SetWindowLongPtrW` + `SWP_FRAMECHANGED`. Also removed `DwmExtendFrameIntoClientArea` from all windows and `setup_native_frameless` from secondary windows.

**Net effect:** Both resize mechanisms are gone:
- CEF Views `can_resize` delegate → no longer used (native mode)
- Win32 `WS_THICKFRAME` at creation → removed (causes flash)
- Win32 `WS_THICKFRAME` post-show → added but not working (see analysis below)
- `DwmExtendFrameIntoClientArea` + `WS_THICKFRAME` combo → removed (causes flash)

## Root Cause

`WS_THICKFRAME` is required for Windows to process `WM_NCHITTEST` hit-test results (HTLEFT, HTRIGHT, etc.) as resize actions. Without it, returning HTLEFT from WM_NCHITTEST is ignored by the window manager.

### Timeline

1. **Original (CEF Views):** CEF Views handled resize internally — no Win32 style needed
2. **PR #268:** Switched to native window mode. Added `WS_THICKFRAME` at creation for resize
3. **PR #271:** Removed `WS_THICKFRAME` from creation (caused white border flash). Added it back in `on_load_end` via `SetWindowLongPtrW` + `SetWindowPos(SWP_FRAMECHANGED)`

### Why Post-Show WS_THICKFRAME Doesn't Work

Adding `WS_THICKFRAME` after the window is visible via `SetWindowLongPtrW` + `SWP_FRAMECHANGED` should theoretically work. Possible reasons it doesn't:

1. **CEF intercepts WM_NCHITTEST** — CEF's browser HWND may be handling hit-testing internally and not forwarding to the parent window's `WM_NCHITTEST` handler
2. **WndProc hook order** — Our `install_frameless_resize_hook` replaces the WndProc BEFORE `WS_THICKFRAME` is added. The hook returns HTLEFT/etc but Windows may need the thick frame style present when the hook is installed
3. **Child window covers entire client area** — CEF creates a child window (Chrome_WidgetWin) that fills the entire client area. Mouse events go to the child, not the parent. The child doesn't forward `WM_NCHITTEST` to the parent
4. **SWP_FRAMECHANGED timing** — The `SetWindowPos` call may need to happen on the UI thread at a specific point in the message loop

## WndProc Hook Implementation

Current hook in `client.rs` (`install_frameless_resize_hook`):

```rust
WM_NCCALCSIZE if wparam == 1 => return 0;  // Remove non-client area
WM_NCACTIVATE => return 1;                  // Suppress DWM border
WM_NCHITTEST => {
    // Check cursor against window rect edges (6px border)
    // Return HTLEFT, HTRIGHT, HTTOP, HTBOTTOM, or corner variants
    // Falls through to original WndProc if not on edge
}
```

This hook is installed on the **browser HWND** (from `host.window_handle()`), which in native mode IS the top-level window. The hook should work — the question is whether `WM_NCHITTEST` messages reach it.

## Possible Fixes

### Option A: Verify WS_THICKFRAME is actually applied

Add logging after `SetWindowLongPtrW` to confirm the style was set:
```rust
let new_style = GetWindowLongPtrW(target, GWL_STYLE);
tracing::info!("WS_THICKFRAME set: {}", (new_style & WS_THICKFRAME as isize) != 0);
```

### Option B: Re-install WndProc hook after WS_THICKFRAME

The hook may need to be installed AFTER `WS_THICKFRAME` is added. Currently:
1. `on_after_created`: install hook (no WS_THICKFRAME yet)
2. `on_load_end`: add WS_THICKFRAME

Try reversing: add `WS_THICKFRAME` first, then re-install the hook.

### Option C: Handle resize via WM_SYSCOMMAND

Instead of relying on `WM_NCHITTEST` + `WS_THICKFRAME`, manually initiate resize:
```rust
WM_LBUTTONDOWN => {
    // If cursor near edge, send WM_SYSCOMMAND with SC_SIZE + direction
    SendMessageW(hwnd, WM_SYSCOMMAND, SC_SIZE | direction, 0);
}
```
This doesn't require `WS_THICKFRAME` at all.

### Option D: Transparent resize border overlay

Create a separate transparent layered window around the main window that handles resize. This is what some apps (e.g., Spotify) do for frameless windows.

### Option E: Move WS_THICKFRAME to creation, use DWMWA_CLOAK for border

Create with `WS_THICKFRAME` (border exists but window is cloaked). The cloak hides both the content flash AND the border. Uncloak after `WM_NCCALCSIZE` has already hidden the border via the hook.

## Recommended Next Step

Start with **Option A** (verify the style is set) then **Option B** (re-install hook after style). If neither works, **Option C** (WM_SYSCOMMAND) is the most reliable — it doesn't depend on window styles at all.

## References

- [WM_NCHITTEST docs](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-nchittest)
- [Custom Window Frame Using DWM](https://learn.microsoft.com/en-us/windows/win32/dwm/customframe)
- [Frameless window resize without WS_THICKFRAME](https://www.tutorialpedia.org/blog/create-window-without-titlebar-with-resizable-border-and-without-bogus-6px-white-stripe/)
