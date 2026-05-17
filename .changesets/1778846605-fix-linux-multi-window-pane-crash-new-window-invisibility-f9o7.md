---
type: patch
---

fix(linux): multi-window pane crash + new-window invisibility + tab-switch overlay bleed

Three related Linux-only correctness fixes in the Views pane/window
machinery:

- **Pane crash in 2nd/3rd window** (`browser_pane/creation_views.rs`):
  opening a browser pane in any non-first window FATAL-crashed the CEF host
  with `observer_list.h:318 NOTREACHED — Observers can only be added once!`.
  Root cause: every isolated `RequestContext` yields a different `Profile*`
  pointer but they share one `ThemeService` instance (chrome's
  `ThemeServiceFactory` redirects to the original profile). The pane passed
  `None` for `request_context` → different `Profile*` than the parent
  window's main browser → `CefWidgetImpl::AddAssociatedProfile` map miss →
  re-`AddObserver` on the shared `ThemeService`. Fix: pane reuses the
  parent window's `RequestContext` via
  `state.get_browser(window_label).host().request_context()`.

- **"New Window" creates a CEF browser but no OS window appears**
  (`ui_tasks.rs`): with at least one pane open,
  `state.first_browser()`'s HashMap iteration order could pick a pane
  browser. The new window inherited its `is_browser_pane=true` client.
  `on_load_end`'s pane early-return then skipped the `window.show()`
  call. Fix: filter `list_browsers()` to a top-level (non-pane) browser
  when picking the client to reuse.

- **Inactive-tab browser pane bleeds into active tab**
  (`browser_pane/creation_views.rs::resize_browser_pane_view`): switching
  tabs left the previous tab's `OverlayController` drawing a borderless
  residual quad on top of the new tab's DOM. Root cause: frontend sends
  `browser_pane_resize(0,0,0,0)` when the placeholder goes `display:none`
  (getBoundingClientRect returns all zeros), but the backend only called
  `set_size(0,0) + set_position(0,0)` without `set_visible(0)`. On
  Wayland a 0-sized OverlayController at `set_visible(1)` still
  composites residual pixels. Fix: toggle `set_visible` based on rect
  dimensions (`width>0 && height>0`).

Spec: `docs/specs/multi-window-pane-and-newwindow-fixes-linux-2026-05-15.md`
