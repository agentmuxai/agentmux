# Secondary Windows: Switch from Native to CEF Views

**Date:** 2026-04-02
**Status:** Spec
**Depends on:** PR #272 (CEF Views deferred show)
**Problem:** Secondary windows (new window, tear-off) have no resize handlers

## Background

PR #268-#272 fixed the white flash by switching the main window from native mode to CEF Views with deferred `Show()`. The main window now has:
- No white flash
- Resize via CEF Views delegate (`can_resize`)
- Frameless via CEF Views delegate (`is_frameless`)
- Snap, minimize, maximize all working

Secondary windows (opened via "new window" button or pane tear-off) still use **native mode** (`browser_host_create_browser` with `WindowInfo`). They lack:
- Resize handlers (WM_NCHITTEST on parent is blocked by CEF child window)
- Proper frameless handling (WM_NCCALCSIZE hack doesn't work with CEF child)
- Consistent behavior with main window

## Root Cause

In native mode, CEF creates a child window (`Chrome_WidgetWin_0`) that fills the entire parent. Mouse events go to the child, not the parent. Our WndProc hooks on the parent (`WM_NCHITTEST`, `WM_NCCALCSIZE`) never receive mouse events, so resize doesn't work.

CEF Views handles this internally — it knows about the browser child and routes hit-testing correctly through its own framework.

## Proposed Fix

Switch secondary windows from native mode to CEF Views, matching the main window.

### Current Flow (native mode)

```
open_new_window() / open_window_at_position()
  → build WindowInfo { style: WS_POPUP | ... }
  → browser_host_create_browser(window_info, client, url, settings)
  → on_after_created: install_frameless_resize_hook (doesn't work)
  → on_load_end: ShowWindow via Win32 + WS_THICKFRAME (no resize)
```

### Proposed Flow (CEF Views)

```
open_new_window() / open_window_at_position()
  → browser_view_create(client, url, settings, delegate)
  → window_create_top_level(window_delegate)
  → on_window_created: add_child_view, set_bounds, do NOT show
  → on_load_end: window.show() via CEF Views API
```

### Files to Change

**1. `agentmux-cef/src/commands/window.rs` — `open_new_window()`**

Replace:
```rust
let window_info = cef::WindowInfo {
    runtime_style: cef::RuntimeStyle::ALLOY,
    window_name: cef::CefString::from("AgentMux"),
    style: WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
        | WS_MINIMIZEBOX | WS_MAXIMIZEBOX,
    bounds: cef::Rect { x, y, width, height },
    ..Default::default()
};
browser_host_create_browser(&window_info, client, &url, &settings, None, None);
```

With:
```rust
let browser_view = cef::browser_view_create(
    client, &url, &settings, None, None, &delegate,
);
let window_delegate = SecondaryWindowDelegate::new(
    browser_view, pos_x, pos_y, win_w, win_h,
);
cef::window_create_top_level(&window_delegate);
```

**2. `agentmux-cef/src/commands/drag.rs` — `open_window_at_position()`**

Same change — replace `browser_host_create_browser` with `browser_view_create` + `window_create_top_level`.

**3. `agentmux-cef/src/app.rs` — Add secondary window delegate**

The main window's `AgentMuxWindowDelegate` already exists. Secondary windows need the same behavior but:
- Positioned at (pos_x, pos_y) instead of centered 70%
- May have different size

Options:
- **Reuse `AgentMuxWindowDelegate`** with position/size parameters
- **Create `SecondaryWindowDelegate`** (cleaner separation)

Recommendation: Add position/size fields to `AgentMuxWindowDelegate`:
```rust
wrap_window_delegate! {
    pub struct AgentMuxWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        initial_bounds: Option<(i32, i32, i32, i32)>,  // NEW: (x, y, w, h)
    }
}
```

In `on_window_created`:
```rust
if let Some((x, y, w, h)) = self.initial_bounds {
    window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
} else {
    // Default: 70% of monitor
    if let Some((x, y, w, h)) = get_monitor_centered_70pct(window) {
        window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
    }
}
// Do NOT show — deferred to on_load_end
```

**4. `agentmux-cef/src/client.rs` — Simplify `on_load_end`**

Remove the Win32 `ShowWindow` fallback for native secondary windows. All windows use CEF Views now, so the `browser_view_get_for_browser` path always works:

```rust
// Show via CEF Views API — works for all windows
let mut browser_cloned = browser.cloned();
if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
    if let Some(window) = bv.window() {
        if window.is_visible() == 0 {
            window.show();
            // focus...
        }
    }
}
```

**5. `agentmux-cef/src/client.rs` — Remove `install_frameless_resize_hook` for secondary windows**

No longer needed — CEF Views handles frameless via delegate.

**6. `agentmux-cef/src/app.rs` — `AgentMuxBrowserViewDelegate::on_popup_browser_view_created`**

Already creates secondary windows via CEF Views for popups (devtools). May need to pass position/size for tear-off windows.

### What About `RequestContext` Isolation?

Currently, secondary windows use per-window `RequestContext` for renderer process isolation:
```rust
let request_context = cef::request_context_create_context(
    &request_context_settings, None,
);
browser_host_create_browser(..., Some(&request_context), ...);
```

`browser_view_create` also accepts a `RequestContext` parameter. Need to pass it through:
```rust
let browser_view = cef::browser_view_create(
    client, &url, &settings,
    None,                    // extra_info
    Some(&request_context),  // request_context — keep isolation
    &delegate,
);
```

### What We Remove

- `install_frameless_resize_hook` call for secondary windows
- `setup_native_frameless` (DwmExtendFrameIntoClientArea) — already removed
- Win32 `ShowWindow` + `WS_THICKFRAME` + `SWP_FRAMECHANGED` fallback in `on_load_end`
- WS_POPUP window style for secondary windows
- `get_secondary_window_size()` helper (replaced by delegate bounds)

### What We Keep

- `install_frameless_resize_hook` + `setup_native_frameless` functions (may be needed for `--use-native` flag or future use)
- `ORIGINAL_WNDPROCS` static (same)
- `get_offset_position()` helper (still needed for positioning)

## Test Plan

- [ ] Main window: no flash, resize, snap, frameless — unchanged
- [ ] New window button: opens, no flash, resize works, frameless
- [ ] Pane tear-off: opens at cursor position, no flash, resize works
- [ ] Secondary window close: doesn't affect main window
- [ ] Multiple secondary windows: each independent
- [ ] Page reload in secondary: doesn't re-show (is_visible guard)
- [ ] macOS/Linux: unaffected (CEF Views is cross-platform)

## Complexity

Medium — the main change is in `commands/window.rs` and `commands/drag.rs` replacing `browser_host_create_browser` with `browser_view_create` + `window_create_top_level`. The delegate already exists and just needs position/size parameterization.

Estimated: ~100 lines changed across 4 files.
