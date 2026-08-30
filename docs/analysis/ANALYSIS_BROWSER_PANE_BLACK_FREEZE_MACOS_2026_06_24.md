# Browser Pane: Black Appearance + UI Freeze on macOS/Linux

> **RESOLVED — fixed the same day this was written** (resolution note added
> 2026-08-29, docs-cleanup Phase 4; the document previously carried no
> indication it had been fixed at all):
> - **#1769** `fix(browser-pane): black flash + UI freeze on macOS/Linux
>   when opening a browser pane` — 2026-06-24, hours after this analysis.
> - **#1778** (`fix/browser-pane-deferred-bounds-macos`, merged 2026-06-27)
>   — follow-ups: deferred overlay bounds, and resizing
>   `NativeWidgetMacNSWindow` via ObjC for the residual macOS black screen.
>
> A retrospective on the fix exists alongside this file:
> `docs/analysis/RETRO_BROWSER_PANE_MACOS_FIX_2026_06_26.md`. It was never
> linked from here — which is why this analysis still read as an open bug.

**Date:** 2026-06-24
**Affected versions:** ≤ 0.47.4 (macOS/Linux only)
**Symptom:** Opening a browser pane causes it to render solid black and freezes all
click input in the parent window. The uptime clock and other animations continue
(JS event loop is alive; only native input is blocked).
**Fixed in:** This commit.

---

## Root Causes

Two independent bugs combine to produce the symptom.

### Bug 1 — `set_visible(1)` called before `layout()` (black flash + wrong-position input intercept)

`creation_views.rs::create_browser_pane_view` previously ordered the overlay
setup as:

```rust
overlay_controller.set_size(...);
overlay_controller.set_position(...);
overlay_controller.set_visible(1);   // ← visible before layout
parent_window.layout();
```

CEF's `DockingMode::CUSTOM` overlays do not commit their `set_size` /
`set_position` until the owning `Window::layout()` pass runs. Between
`set_visible(1)` and `layout()` the overlay is therefore visible at an
uncommitted position (empirically: 0×0 origin, varying size). During that
window the opaque black `background_color: 0xFF000000` baseline is painted
at the wrong location — visible to the user as a black flash. Worse, because
the overlay is a native CEF View, it captures pointer events for the region it
occupies, even at the wrong position, before layout corrects it.

**Fix:** swap order — `layout()` first, `set_visible(1)` after.

### Bug 2 — `on_after_created_browser_pane` is a no-op on macOS/Linux (full freeze)

`callbacks.rs::on_after_created_browser_pane` is the callback that fires when
the new pane's `CefBrowser` is created (triggered by `add_overlay_view` adding
the `BrowserView` to the widget hierarchy). On Windows it installs a
`WM_SETFOCUS` redirect subclass that intercepts Chromium's automatic focus
grant to the new browser and redirects it back to the parent window. On
macOS/Linux the body was:

```rust
#[cfg(not(target_os = "windows"))]
{
    let _ = (state, browser);  // complete no-op
}
```

Without the redirect, CEF gives focus to the newly created pane
`OverlayController`. On macOS, a focused `OverlayController` (created with
`can_activate=1`) absorbs keyboard events and — depending on CEF hit-test
state at the moment of creation — mouse events for the entire parent window
content area. The result: the user can see the UI rendered correctly (the JS
event loop is running, animations play, the uptime clock ticks) but no click
reaches the SolidJS frontend. All input goes to the pane browser which is
still loading (showing black).

The key evidence:
- Only 1 DOM element had `pointer-events: none` (`overlay-container`,
  the layout drag placeholder — this is normal when `activeDrag = false`).
- No block content elements were frozen at the CSS layer.
- Sending `browser_pane_close` via the IPC immediately unfroze the window,
  confirming the OverlayController was the sole input interceptor.

**Fix:** in `create_browser_pane_view`, after `add_overlay_view` returns (by
which point `on_after_created` has already fired), call
`main_browser.host().set_focus(1)` to return focus to the parent window's
browser. `on_after_created_browser_pane` now also has a non-Windows
implementation as a safety net for navigation-driven focus re-steals
(cross-origin redirects recreate the renderer process and can re-trigger
`on_after_created`).

---

## Why `activeDrag` was not the cause

An earlier hypothesis (documented in
`agentmux-0.47.4-unresponsive-bug-report.md`) identified `activeDrag` stuck
at `true` as a candidate, because `TileLayout.darwin.tsx` lacks the
`window.addEventListener("dragend", resetDragState)` safety net present in
`TileLayout.win32.tsx`. That path remains a latent risk (drag ending over
the native overlay could still strand `activeDrag`), but it was **not** the
cause of the reported freeze:

- DOM inspection via CDP confirmed `activeDrag = false` (the `overlay-container`
  had `pointer-events: none`, which is the correct state when NOT dragging).
- No block content elements had `pointer-events: none`.
- The freeze lifted immediately on `browser_pane_close`, not on a drag-reset.

The `activeDrag` safety-net gap on darwin/Linux is a separate issue worth
addressing but is out of scope for this fix.

---

## Fix Summary

**`agentmux-cef/src/browser_pane/creation_views.rs`**

1. Reorder `layout()` before `set_visible(1)` so the overlay is at the correct
   position when it first becomes visible (eliminates black flash at wrong coords).
2. After `add_overlay_view` returns, call `state.get_browser(&window_label)
   .host().set_focus(1)` to return focus to the main window browser.

**`agentmux-cef/src/browser_pane/callbacks.rs`**

Replace the `#[cfg(not(target_os = "windows"))]` no-op in
`on_after_created_browser_pane` with an implementation that looks up the pane's
parent window via `state.browser_pane_overlays` and calls `set_focus(1)` on
the parent browser — a safety net against navigation-driven focus re-steals.

---

## Recovery (without fix)

For a frozen instance (before patching):

```bash
# 1. Find the browser pane block ID in the objects DB
sqlite3 ~/.agentmux/channels/stable/versions/<ver>/data/db/objects.db \
  "SELECT oid FROM db_block WHERE json_extract(data,'$.meta.view')='browser';"

# 2. Close it via IPC (port and token from the window's URL params / CEF globals)
curl -X POST http://127.0.0.1:<ipc_port>/ipc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <ipc_token>" \
  -d '{"cmd":"browser_pane_close","args":{"block_id":"<id>"}}'
```

All other state (agent sessions, layout, history) is preserved.
