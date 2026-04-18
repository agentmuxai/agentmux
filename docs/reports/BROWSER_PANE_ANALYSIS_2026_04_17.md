# Browser Pane Implementation Analysis

**Date:** 2026-04-17
**Status:** Blocked — all approaches tried have fundamental issues

---

## Problem Statement

We need to render an external website (e.g., google.com) inside a pane
of the AgentMux window, alongside terminal, agent, and editor panes.
The website must be a real browser — full JavaScript, cookies, login
flows, not a dumbed-down preview.

---

## Approaches Tried

### 1. iframe (in the main BrowserView's DOM)

**Result:** CRASHES THE APP

The main BrowserView renders the SolidJS frontend. An `<iframe>` in
the DOM tries to load google.com. Google returns `X-Frame-Options:
SAMEORIGIN`. CEF's `on_load_error` fires for the sub-frame and
replaces the entire page with an error page, killing the UI.

**Fix attempted:** Skip `on_load_error` for sub-frames (`frame.is_main() != 1`).
This prevents the crash but the iframe still shows a blank page because
Chromium enforces X-Frame-Options at the renderer level — the content
simply doesn't render.

**Verdict:** Cannot work. X-Frame-Options is enforced by Chromium's
renderer, not by our error handler. Most external sites set it.

### 2. add_child_view (second BrowserView as child of CefWindow)

**Result:** FILLS ENTIRE WINDOW

`window.add_child_view(browser_view)` adds the new view to the
window's view hierarchy. CEF's layout manager auto-fills child views
to the window size, ignoring `set_bounds()`. The new browser covers
the entire window, hiding the main UI.

`set_bounds()` has no effect because the window's FillLayout (default)
overrides individual view bounds. CEF Views doesn't support absolute
positioning of child views within a window — it uses layout managers.

**Verdict:** Cannot work with the default layout. Would need to replace
the window's layout manager, which risks breaking the main BrowserView.

### 3. Frameless popup window (separate CefWindow)

**Result:** OPENS FULL AGENTMUX INSTANCE

`window_create_top_level` with `AgentMuxWindowDelegate` creates a
complete new AgentMux window (backend init, workspace, tabs). Not a
bare browser. Links navigate away, focus fights with the main window,
shows in taskbar.

**Could be fixed** by creating a minimal WindowDelegate that only
shows the BrowserView, but then:
- Separate window = separate taskbar entry (fixable with WS_EX_TOOLWINDOW)
- Focus management between two windows
- Must track main window position for repositioning
- Alt-tab shows two entries
- Not a "pane" — it's a floating window that simulates a pane

**Verdict:** Technically viable but architecturally wrong. The user
explicitly rejected this approach.

### 4. Off-screen rendering (OSR)

**Not attempted.** Render the browser to a pixel buffer, paint onto
a `<canvas>` in the DOM.

**Pros:** No z-order, no separate windows, no layout issues.
**Cons:** 
- Complex: must implement CefRenderHandler (OnPaint callback)
- Performance: pixel copy every frame (GPU → CPU → GPU)
- Input: must translate mouse/keyboard events manually
- No hardware acceleration in the embedded browser
- Significant implementation effort (weeks, not days)

**Verdict:** Correct but heavy. This is how game engines embed browsers.

---

## Approaches NOT Yet Tried

### 5. Replace window layout with BoxLayout/absolute positioning

Instead of `add_child_view` on the window (which uses FillLayout),
restructure the window's view hierarchy:

```
CefWindow
  └─ Panel (BoxLayout or custom layout)
       ├─ Main BrowserView (sized to fill most of the panel)
       └─ Pane BrowserView (sized to specific rect, positioned absolutely)
```

CEF Views supports `Panel` with `BoxLayout` and `FillLayout`. If we
create a Panel as the window's content view, and add BOTH BrowserViews
as children of the Panel with explicit layout constraints, the pane
browser can be positioned at a specific rect.

**Risk:** Changing the window's layout may break the main BrowserView's
resize behavior. The current code adds the main BrowserView directly
to the window, which auto-fills it. Introducing a Panel changes this.

### 6. Use `window.add_overlay_view` (CEF 127+)

CEF 127 added `CefWindow::AddOverlayView` which creates a view that
floats on top of other views at a specific position. This is exactly
what we need.

**Check:** Does cef-rs v146 expose `add_overlay_view`?

### 7. Native Win32 HWND embedding (Windows-only)

On Windows, every CefBrowser has an underlying HWND. Instead of using
CEF Views, create the browser with a specific parent HWND and position:

```rust
let hwnd = CreateWindowExW(...); // child window of main HWND
CefBrowserHost::CreateBrowser(hwnd, url, settings);
```

This bypasses CEF Views entirely and uses Win32 window management for
positioning. The browser HWND is a child of the main window's HWND,
so it renders inside it at the specified rect.

**Pros:** Direct, no layout manager issues, proven pattern (QCefView does this)
**Cons:** Windows-only (need different approach for macOS/Linux)

### 8. WebContentsView pattern (Electron-style)

Electron's `WebContentsView` creates a native child window positioned
within the parent, managed by the OS window manager. CEF can do the
same by creating a browser with `WindowInfo` specifying a parent HWND
and rect, instead of using Views framework.

---

## Recommendation

### Short-term: Option 6 (AddOverlayView) if available

Check if `cef-rs` v146 exposes `CefWindow::AddOverlayView`. If yes,
this is the cleanest solution — one line to add an overlay at a specific
position, with automatic z-ordering above the main BrowserView.

### If AddOverlayView is not available: Option 7 (native HWND)

On Windows, create the browser directly with a parent HWND. This is
the most reliable approach and is used by production CEF embeddings
(QCefView, CefSharp). Platform-specific but works immediately.

### Long-term: Option 5 (Panel restructure)

Restructure the window layout to use a Panel with explicit child
positioning. This is the most architecturally clean approach but
requires careful testing of the main BrowserView's resize behavior.

---

## Action Items

1. Check `cef-rs` v146 for `add_overlay_view` / `AddOverlayView`
2. If not available, check for `CefBrowserHost::CreateBrowser` with
   WindowInfo parent HWND
3. Prototype whichever is available first
4. If neither works, fall back to Panel restructure (option 5)
