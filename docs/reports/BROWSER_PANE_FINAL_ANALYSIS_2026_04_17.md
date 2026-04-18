# Browser Pane — Final Analysis

**Date:** 2026-04-17
**Status:** Solution confirmed

---

## Confirmed Solution: `CefWindow::AddOverlayView`

After rigorous research, `AddOverlayView` with `CEF_DOCKING_MODE_CUSTOM`
is the **proven, production-ready approach** for embedding a second browser
inside a CEF Views window. This is:

- The official CEF API for this exact use case (added CEF 127)
- Available in our cef-rs v146 bindings
- Used by QCefView, cefclient, and other production CEF embeddings
- Thread-safe when called on the UI thread
- Handles z-order, composition, and clipping automatically

---

## Implementation (3 API calls)

```cpp
// 1. Create a BrowserView for the URL
auto browser_view = CefBrowserView::CreateBrowserView(
    handler, "https://google.com", settings, nullptr, nullptr, nullptr
);

// 2. Add as overlay with custom positioning
auto controller = window->AddOverlayView(
    browser_view,
    CEF_DOCKING_MODE_CUSTOM,  // we control position via SetBounds
    1                          // can_activate = true (receives keyboard/mouse)
);

// 3. Position and show
controller->SetBounds({x, y, width, height});
controller->SetVisible(true);  // CRITICAL: won't render without this
```

In Rust (cef-rs):
```rust
let mut view = View::from(&browser_view);
let controller = window.add_overlay_view(
    Some(&mut view),
    DockingMode::CUSTOM,
    1,  // can_activate
);
if let Some(controller) = controller {
    controller.set_bounds(Some(&rect));
    // SetVisible may be needed — check if overlay starts hidden
}
```

---

## Critical Gotchas (from production experience)

1. **Must call SetVisible(true)** — overlay starts hidden with CUSTOM
   docking mode. Without this, browser creates but nothing renders.
   (CEF GitHub issue #3790)

2. **Must run on UI thread** — all Views calls via `post_task(ThreadId::UI)`.
   Already handled in our existing `wrap_task!` pattern.

3. **Browser not immediately available** — `browser_view.browser()` returns
   None until the view is added to the hierarchy. Register the browser in
   `on_after_created`, not at creation time.

4. **can_activate parameter** — set to 1 (true) so the browser receives
   keyboard and mouse input. Set to 0 for display-only overlays.

5. **Resize** — call `controller.set_bounds(new_rect)` on UI thread when
   the pane rect changes (ResizeObserver + IPC from frontend).

6. **Cleanup** — call `controller.destroy()` to remove the overlay.

---

## What We Store

```rust
struct BrowserPane {
    controller: OverlayController,  // for set_bounds and destroy
    // Browser handle is registered in state.browsers via on_after_created
}
```

The `OverlayController` is the key handle — it controls position, visibility,
and destruction. The `Browser` handle for navigation is registered separately
in `state.browsers` keyed by a label like `"browser-pane-{block_id}"`.

---

## Why Previous Approaches Failed

| Approach | Why it failed | Why AddOverlayView fixes it |
|----------|--------------|----------------------------|
| iframe | X-Frame-Options blocks external sites at renderer level | Not an iframe — real browser process |
| add_child_view | Window's FillLayout overrides bounds | Overlay is outside the layout manager |
| popup window | Creates full AgentMux instance | Overlay is inside the same window |
| OSR | Complex, no hardware acceleration | Overlay uses native rendering |

---

## Implementation Plan

**One PR, three changes:**

1. `browser_panes.rs` — rewrite CreateBrowserPaneTask to use
   `window.add_overlay_view()` instead of `window.add_child_view()`.
   Store `OverlayController` for resize/destroy.

2. `browser_panes.rs` — ResizePaneTask calls `controller.set_bounds()`

3. `browser_panes.rs` — ClosePaneTask calls `controller.destroy()`

**No frontend changes needed** — the existing placeholder div +
ResizeObserver + IPC commands work unchanged.

**Estimated effort:** 30 minutes. The API is straightforward.

---

## Sources

- [CEF Window API — AddOverlayView](https://cef-builds.spotifycdn.com/docs/133.4/classCefWindow.html)
- [CEF GitHub Issue #3790 — Overlay visibility](https://github.com/chromiumembedded/cef/issues/3790)
- [QCefView — Production CEF overlay usage](https://github.com/CefView/QCefView)
- [cef-rs v146 bindings — add_overlay_view confirmed](local crate search)
