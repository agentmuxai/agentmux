# Browser Pane Architecture Decision

**Date:** 2026-04-17
**Status:** Decision needed

---

## What We've Proven

| Approach | Browser Creates? | Renders Correctly? | Blocks UI? |
|----------|-----------------|-------------------|------------|
| `add_child_view` on Window | YES (total: 2) | Fills entire window (FillLayout) | YES |
| `AddOverlayView` | NO (total: 1) | Black screen | YES (no input) |
| Frameless popup | YES | Correct size | Creates full AgentMux instance |
| iframe | N/A | X-Frame-Options blocks sites | Crashes app |

## Why Each Fails Architecturally

**add_child_view**: The Window's FillLayout controls ALL child views.
It's not a timing issue — the layout runs on every resize event. Any
"deferred bounds" hack will be overridden on the next resize. This is
fundamentally broken, not fixable with better timing.

**AddOverlayView**: CEF bug #3790. BrowserViews added as overlays never
initialize their renderer process. Not fixable on our side.

**Popup window**: Our `AgentMuxWindowDelegate` initializes a full app
instance (backend connection, workspace, tabs). Would need a completely
separate minimal WindowDelegate. Also creates OS-level window management
complexity (taskbar, focus, positioning).

---

## The Production-Proven Approach

### Native Child Window via CefBrowserHost::CreateBrowser

This is how **every production CEF embedding** (QCefView, CefSharp,
Spotify desktop, Discord, Figma) renders embedded browsers:

1. Get the main window's native OS handle (HWND on Windows)
2. Create a `CefWindowInfo` with that HWND as parent
3. Call `CefBrowserHost::CreateBrowser(windowInfo, client, url, settings)`
4. CEF creates a native child window inside the parent
5. Position/resize via OS APIs (SetWindowPos on Windows)

**Why this works:**
- Bypasses CEF Views framework entirely — no FillLayout, no layout manager
- The browser is a native OS child window with its own rendering
- Z-order is handled by the OS window manager (child windows render on top)
- Position/size is controlled by platform APIs, not CEF Views
- This is the approach used by QCefView (Qt+CEF), CefSharp (.NET+CEF),
  and Chromium's own `<webview>` tag in the Chrome browser

**Platform specifics:**
- Windows: `CefWindowInfo::SetAsChild(parent_hwnd, rect)`
- macOS: `CefWindowInfo::SetAsChild(parent_nsview, rect)`
- Linux: `CefWindowInfo::SetAsChild(parent_xwindow, rect)`

All three platforms are covered by a single API pattern.

### How to Get the Parent HWND

CEF's `BrowserHost` provides `GetWindowHandle()` which returns the
native OS window handle. We can get the main browser's HWND and use
it as the parent for the embedded browser.

```rust
// Get main window's native handle
let main_hwnd = main_browser.host().unwrap().window_handle();

// Create child browser
let mut window_info = CefWindowInfo::new();
window_info.set_as_child(main_hwnd, rect);

CefBrowserHost::CreateBrowser(
    &window_info,
    client,
    url,
    &settings,
    None, // extra_info
    None, // request_context
);
```

### Resize

```rust
// On Windows: reposition the child window
SetWindowPos(child_hwnd, NULL, x, y, width, height, SWP_NOZORDER);
```

The frontend's ResizeObserver sends rect updates via IPC. The Rust
handler calls `SetWindowPos` (Win32) to reposition.

### Close

```rust
browser_host.close_browser(true);
```

---

## Implementation Checklist

1. Check if `CefWindowInfo`, `set_as_child`, `window_handle` exist in cef-rs v146
2. If yes: implement using native child window
3. Get main window HWND via `main_browser.host().window_handle()`
4. Create `CefWindowInfo` with parent HWND
5. Call `CefBrowserHost::CreateBrowser`
6. Store child HWND for resize/close
7. Resize via platform API (SetWindowPos on Windows)
8. Close via `close_browser`

---

## Risk Assessment

**Low risk:** This is the standard CEF embedding pattern used by hundreds
of production apps. It doesn't interact with CEF Views at all — it uses
the native windowing layer that CEF was originally designed for (before
Views was added).

**Platform specificity:** Requires platform-specific resize code. But
we already have extensive `#[cfg(windows)]` code throughout the codebase.
macOS and Linux implementations follow the same pattern with different
API calls.
