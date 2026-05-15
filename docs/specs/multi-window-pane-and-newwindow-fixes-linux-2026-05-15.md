# Multi-window correctness on Linux: pane RequestContext + new-window client

**Status:** Spec — implemented in this PR
**Date:** 2026-05-15
**Owner:** asafebgi@gmail.com
**Branch:** `agentu/multi-window-fixes` off `main`

## TL;DR

Two related Linux-only bugs in the Views-based window/pane flow that surface
once you have more than one top-level window:

1. **Pane crash in the 2nd/3rd window.** Opening a `defwidget@browser` pane
   in any non-first window FATAL-crashes the CEF host with
   `base/observer_list.h:318 NOTREACHED — Observers can only be added once!`.
   Fixed in `agentmux-cef/src/browser_pane/creation_views.rs` by reusing the
   parent window's `RequestContext` instead of passing `None`.
2. **"New Window" creates a CEF browser but no OS window appears.** Backend
   reports success, status panel updates, but the window is never shown.
   Fixed in `agentmux-cef/src/ui_tasks.rs` by filtering
   `state.list_browsers()` to a top-level (non-pane) browser when picking the
   client to reuse, instead of `first_browser()`'s HashMap-iteration-order
   "any" browser.

Both bugs are Linux-only because both root causes route through the CEF
Views attachment path (`browser_view_create` + `AddOverlayView` /
`window_create_top_level`). Windows uses the native HWND-child path
(`browser_host_create_browser` with `WindowInfo::set_as_child`) which doesn't
touch the same machinery.

## Bug 1: Pane in 2nd window FATAL-crashes the host

### Symptom

```
[FATAL:base/observer_list.h:318] NOTREACHED hit. Observers can only be added once!
#6 base::ObserverList<>::AddObserver()
#7 CefWidgetImpl::AddAssociatedProfile()
#8 CefBrowserViewImpl::AddedToWidget()
#9 CefBrowserViewView::AddedToWidget()
#10 views::View::PropagateAddNotifications()
#11 views::View::AddChildViewAtImpl()
#12 CefOverlayViewHost::Init()
#13 CefWindowView::AddOverlayView()
#14 CefWindowImpl::AddOverlayView()
```

### Mechanism

Diagnostic probes added to `cef/libcef/browser/views/widget_impl.cc::AddAssociatedProfile`
and `cef/libcef/browser/views/browser_view_impl.cc::AddedToWidget` (kept in
the local CEF tree at `cef-build/.../cef/`) reproduced the crash with
concrete pointers:

```
New window #2: BrowserView 0x...624f00 → widget 0x...449d000,
   cef_widget 0x...449d418, profile 0x...280040,
   profile_orig 0x...376300, theme_service 0x...22e1540, map_size 0
New window #3: BrowserView 0x...59ecd20 → widget 0x...726900,
   cef_widget 0x...726d18, profile 0x...278900,
   profile_orig 0x...376300, theme_service 0x...22e1540, map_size 0
Pane in window #3:                       widget 0x...53d8d80,
   cef_widget 0x...726d18, profile 0x...376300,
   profile_orig 0x...376300, theme_service 0x...22e1540, map_size 1
FATAL NOTREACHED — Observers can only be added once!
```

The data shows three load-bearing facts:

- Every `RequestContext` agentmux creates via `create_isolated_request_context`
  returns a **different `Profile*`** pointer (`0x...280040`, `0x...278900`, …),
  but they all share the same `OriginalProfile()` (`0x...376300`).
- Chrome's `ThemeServiceFactory` is `BrowserContextKeyedServiceFactory` with
  `GetBrowserContextToUse` redirecting to the original profile, so all the
  per-window profiles map to **one shared `ThemeService` instance**
  (`0x...22e1540` across all six probe events).
- `CefWidgetImpl::AddAssociatedProfile`'s `associated_profiles_` map is keyed
  on `Profile*`. A profile not in the map → falls through to
  `theme_service->AddObserver(this)`. When a pane attaches to window #3, the
  pane registers a **different `Profile*`** (the original, `0x...376300`,
  because `creation_views.rs` passed `None` for `request_context` → global
  default) than window #3's main browser already registered (`0x...278900`).
  Map miss → `AddObserver` called → CHECK trips because window #3's
  CefWidgetImpl already observes that shared `ThemeService` from registering
  its own profile.

### Fix

`agentmux-cef/src/browser_pane/creation_views.rs:152-163` — look up the
parent window's `RequestContext` and pass it to `browser_view_create`:

```rust
let parent_request_context = state
    .get_browser(&window_label)
    .and_then(|b| b.host())
    .and_then(|h| h.request_context());
tracing::info!(
    block_id = %block_id, label = %label,
    window_label = %window_label,
    has_parent_context = parent_request_context.is_some(),
    "[browser-pane] views: resolved parent window's RequestContext"
);
let mut request_context = parent_request_context;
let pane_view = match browser_view_create(
    client.as_mut(),
    Some(&url_cef),
    Some(&settings),
    None,
    request_context.as_mut(),   // <-- was None
    Some(&mut view_delegate),
) { … };
```

With this change, the pane's `Profile*` matches the parent window's main
browser's `Profile*`, the map check fires, `AddObserver` is skipped, and
the CHECK no longer trips. Also a free correctness win: the pane now
shares cookies/storage with the host UI of the window it lives in (it was
previously on the global default profile, which is silently inconsistent
with the per-window isolated context).

The parent window's main browser is registered under its own `window_label`
(top-level browsers register under `window-<uuid>` per `client/mod.rs:218`;
panes under `browser-pane-…`), so `state.get_browser(window_label)` returns
the right Browser. If the lookup misses (shouldn't happen — the pane is
created within a window that must already have its main browser registered
— but defensively), we fall back to `None` and the legacy behavior; the
`has_parent_context=false` log surfaces it.

### Windows impact: none

Windows takes the `browser_host_create_browser` path
(`agentmux-cef/src/browser_pane/creation.rs:113`), not `browser_view_create`.
`CefBrowserViewImpl::AddedToWidget` is never invoked → `AddAssociatedProfile`
is never invoked → the trip site is unreachable. `creation.rs` is
unchanged.

## Bug 2: "New Window" doesn't open a visible window

### Symptom

User clicks the hamburger → New Window. Backend logs report success:
`Browser created (total: N+1)`, `[create-window] window_create_top_level
returned`, `[frontend] Window type: new window`, status panel updates with
the new window entry. But **no OS window appears**. Repeated clicks pile up
hidden windows.

### Mechanism

`ui_tasks.rs::CreateWindowTask::execute` constructs the new window by
reusing an existing browser's CEF client:

```rust
// before
let client = self
    .state
    .first_browser()
    .and_then(|(_, b)| b.host().map(|h| h.client()));
```

`state.first_browser()` is `browsers.iter().next()` — non-deterministic
HashMap iteration order over **all** registered browsers, including pane
browsers. Pane browsers have a Client with `is_browser_pane=true`. When the
new window's main browser inherits this Client, its `on_load_end` callback
in `client/mod.rs:954` takes the early-return branch:

```rust
if self.is_browser_pane {
    if let Some(b) = browser.as_deref() {
        crate::browser_pane::callbacks::on_load_end_browser_pane(&self.state, b);
    }
    return;   // <-- skips the window.show() that follows
}
```

— and the `window.show()` call at `client/mod.rs:986` that wakes Alloy-style
windows is never executed. The window stays hidden. The empirical signature
in the host log is the absence of an `Injected IPC port …` line for the new
window's URL (that log is also gated behind the same `is_browser_pane`
branch).

The reason this only surfaces when the user has a browser pane open: with
no panes, `first_browser()` can only return a top-level. With at least one
pane open, HashMap iteration order can pick the pane.

### Fix

`agentmux-cef/src/ui_tasks.rs:385-403` — filter `state.list_browsers()` to
non-pane browsers (top-level labels are `window-<uuid>`; panes are
`browser-pane-<uuid>-<n>`):

```rust
// Get client from an existing TOP-LEVEL browser. Cannot use
// `first_browser()` here: that does `HashMap::iter().next()` over the
// full browsers map, which includes pane browsers. Pane browsers have
// `is_browser_pane=true` on their client. If we inherited that, the
// new window's `on_load_end` would take the pane early-return branch
// (client/mod.rs:954) and never call `window.show()` — backend reports
// success (status panel updates) but no OS window appears. Filter on
// label prefix: top-level labels are `window-…` (per client/mod.rs:218),
// panes are `browser-pane-…`.
let client = self
    .state
    .list_browsers()
    .into_iter()
    .find(|(label, _)| !label.starts_with("browser-pane-"))
    .and_then(|(_, b)| b.host().map(|h| h.client()));
tracing::info!(
    label = %self.label,
    elapsed_us = t0.elapsed().as_micros() as u64,
    client_found = client.is_some(),
    "[create-window] got client"
);
```

The `client_found` field surfaces regressions in the host log if the
filter ever misses.

### Windows impact: none

Windows hits the same `CreateWindowTask` and so picks up the fix
identically. The `is_browser_pane` flag is shared with Windows-side panes
too (`creation.rs::AgentMuxClient::new(handler, /*is_browser_pane=*/true)`),
so this fix is a correctness win on Windows too if a user ever clicks "New
Window" while a pane is open. We just hadn't reproduced it there because
the on_load_end → `window.show()` path on Windows is a no-op (HWND windows
are shown via the WindowDelegate / native APIs earlier in the flow), so
the symptom would be invisible on Windows even without this fix. Either
way, no regression.

## Test plan

Multi-window pane (Bug 1):

- [ ] Launch agentmux.
- [ ] Open a second top-level window (hamburger → New Window).
- [ ] Open a third top-level window.
- [ ] In window #3 (or #2), open an embedded browser pane.
- [ ] Verify: pane loads, no FATAL, host process survives.
- [ ] Check log: `[browser-pane] views: resolved parent window's
      RequestContext has_parent_context=true`.

New Window with a pane open (Bug 2):

- [ ] Open a browser pane in the main window.
- [ ] Click hamburger → New Window.
- [ ] Verify: a new OS window actually appears.
- [ ] Check log: `[create-window] got client client_found=true`.
- [ ] Check log: `Injected IPC port …` fires for the new window.

Regression checks:

- [ ] Single-window session with no panes: New Window still works
      (only top-level in `list_browsers`).
- [ ] Multiple panes in the same window: still positioned correctly.
- [ ] Windows (separate machine): pane creation and New Window unchanged.

## References

- `cef/libcef/browser/views/widget_impl.cc::AddAssociatedProfile` — the
  trip site.
- `cef/libcef/browser/views/browser_view_impl.cc::AddedToWidget` — call
  site.
- `agentmux-cef/src/client/mod.rs:218` — top-level browsers register
  under `window_label`.
- `agentmux-cef/src/client/mod.rs:954` — `on_load_end` pane early-return
  that skipped `window.show()`.
- `agentmux-cef/src/commands/mod.rs::create_isolated_request_context` —
  per-window context factory (reused as-is).

## Diagnostic probes kept in local libcef.so

Two lightweight `LOG(INFO)` probes survive in our local CEF tree at
`~/cef-build/chromium_git/chromium/src/cef/libcef/browser/views/`:

- `widget_impl.cc::AddAssociatedProfile` — logs widget, profile (path,
  original), theme_service, map state on every attachment.
- `browser_view_impl.cc::AddedToWidget` — logs this BrowserView, resolved
  widget, cef_widget, is_alloy.

They fire only on widget attachment (~3× per window or pane creation), no
stack capture, no measurable overhead. They were the primary diagnostic
tool for narrowing Bug 1, and they're cheap enough to leave in place for
future investigations.
