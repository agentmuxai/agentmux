# How Production Apps Embed Browsers with CEF

**Date:** 2026-04-17

---

## Two Proven Approaches

Every production CEF app uses one of two patterns:

### 1. Native Child Window (CefBrowserHost::CreateBrowser)
**Used by:** CefSharp, QCefView, Brackets, Spotify

Create a browser with a `CefWindowInfo` that specifies a parent native
window handle. The browser renders as an OS-level child window. Positioning
via platform APIs (SetWindowPos on Windows, NSView on macOS).

```
Parent HWND (main window)
  └─ Child HWND (CEF browser — renders google.com)
       └─ positioned via SetWindowPos(x, y, w, h)
```

### 2. Off-Screen Rendering (CefBrowserHost with windowless=true)
**Used by:** OBS Studio

Render to a pixel buffer. The host app paints it wherever it wants.
Input events are forwarded manually.

---

## What Each App Does

### CefSharp (.NET) — Native HWND
- `CefBrowserHost::CreateBrowser` with `CefWindowInfo::SetAsChild(parentHwnd, rect)`
- WPF version wraps in `HwndHost` (CreateWindowEx with WS_CHILD | WS_VISIBLE)
- Handles DPI scaling, focus management, resize
- **Source:** [CefSharp.Wpf.HwndHost](https://github.com/cefsharp/CefSharp.Wpf.HwndHost)

### QCefView (Qt) — Native HWND or OSR
- `CefBrowserHost::CreateBrowser` with parent native window
- Qt widget-based positioning
- Supports both windowed (child HWND) and windowless (OSR) modes
- **Source:** [QCefView](https://github.com/CefView/QCefView)

### OBS Studio — Off-Screen Rendering
- `CefBrowserHost::CreateBrowserSync` with windowless rendering
- Renders to shared texture for GPU compositing
- Uses custom OBS-patched CEF fork
- **Source:** [obs-browser](https://github.com/obsproject/obs-browser)

### Spotify — Native HWND
- Full UI is CEF-based web content
- Hosts CEF builds at cef-builds.spotifycdn.com
- Multi-process CEF3 with native window integration

### Figma — NOT CEF (Electron BrowserView)
- Uses Electron, not CEF
- Canvas is WebGL + WASM, not a separate browser

---

## What AgentMux Should Use

**Native Child Window (CefBrowserHost::CreateBrowser).**

Reasons:
1. Our main window already has a native HWND (CEF creates it)
2. `BrowserHost::window_handle()` gives us the parent HWND
3. Child HWND positioning is handled by the OS — no FillLayout, no
   Views layout manager interference
4. Z-order is native — child windows render on top of parent content
5. This is the pattern used by the two most popular CEF wrappers
   (CefSharp and QCefView)
6. Cross-platform: `CefWindowInfo::SetAsChild` works on all platforms

**NOT Off-Screen Rendering** because:
- Complex (manual input forwarding, pixel buffer management)
- No hardware acceleration
- Only needed for game engines (OBS) where the host app owns rendering

**NOT CEF Views** because:
- FillLayout conflict (proven broken)
- AddOverlayView doesn't create browsers (CEF bug #3790)
- Views is for apps where CEF owns the ENTIRE window layout — not for
  embedding a browser inside an existing UI

---

## Implementation API

### Check cef-rs v146 for:
- `WindowInfo` or `CefWindowInfo` struct
- `set_as_child(parent_handle, rect)` method
- `BrowserHost::create_browser` or `create_browser_sync` function
- `BrowserHost::window_handle()` to get the child HWND for resize

### Resize pattern:
```rust
#[cfg(windows)]
unsafe {
    SetWindowPos(child_hwnd, ptr::null_mut(), x, y, w, h, SWP_NOZORDER);
}
```

### Close pattern:
```rust
browser_host.close_browser(1);
```
