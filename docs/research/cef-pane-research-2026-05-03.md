# CEF Embedded Browser Pane Research — Linux & macOS

Research date: 2026-05-02. Target: AgentMux's port of the Windows native-child-window pane mechanism (`browser_pane/creation.rs`) to Linux (Ozone-Wayland on GNOME/Mutter) and macOS, against a frameless ALLOY-style main window that already uses `browser_view_create` + `add_child_view` for the primary browser.

---

## TL;DR

- **The native-child-window path (`WindowInfo::set_as_child(parent, rect)`) does not work on Linux Wayland and is officially unsupported on macOS.** It works on Linux X11 and on Windows. ([cef#2804](https://github.com/chromiumembedded/cef/issues/2804), [cef#3294](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19688))
- **CEF maintainer (`magreenblatt`) explicitly recommends the Views framework for new code**, and confirms multiple `CefBrowserView` instances may be added to a single `CefWindow` under the **Alloy** style. ([forum t=19718](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718), [cef#3681](https://github.com/chromiumembedded/cef/issues/3681))
- **AgentMux's existing ALLOY + `browser_view_create` infra is exactly the right substrate** to add multi-pane support on Linux/macOS by adding additional `CefBrowserView` children to the main `CefWindow` and positioning them with `View::set_bounds`.
- **macOS native NSView embedding with `set_as_child` does work in a single process**, but the CEF maintainer states embedded non-Views windows are not supported on macOS upstream and the recommended path is Views. ([forum t=19593](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19593), [forum t=19688](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19688))

---

## 1. CEF's two embedding paradigms for sub-browsers

### 1a. Native child window approach — `WindowInfo::set_as_child(parent_handle, rect)` + `browser_host_create_browser`

This is the Windows path AgentMux already uses (`browser_pane/creation.rs:84`).

| Platform | Parent handle type | Status |
| --- | --- | --- |
| Windows | HWND | Fully supported. AgentMux already uses this. |
| Linux X11 | X11 Window XID (cast to `CefWindowHandle`) | Supported with caveats. ([forum t=18048](https://magpcss.org/ceforum/viewtopic.php?t=18048)) |
| Linux Wayland | wl_surface + wl_display (proposed) | **Not implemented in upstream stable CEF.** ([cef#2804](https://github.com/chromiumembedded/cef/issues/2804)) |
| macOS | NSView* (cast to `CefWindowHandle`) | Works in-process; officially "not supported" upstream for new code. ([forum t=12727](https://magpcss.org/ceforum/viewtopic.php?f=6&t=12727), [t=19688](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19688)) |

**Linux X11 (forum t=18048)**: "The parent value is used to identify monitor info and to act as the parent window for dialogs, context menus, etc." Pattern is:

```c
GdkWindow* gdk_window = gtk_widget_get_window(gtk_handle);
Window x_window = GDK_WINDOW_XID(gdk_window);
// pass x_window as CefWindowHandle to SetAsChild
```

**Linux Wayland — the blocker** ([cef#2804](https://github.com/chromiumembedded/cef/issues/2804)):

> "Ozone/Wayland/X11 can only be used with views framework now, but it does not allow to embed host windows into client windows."

> "wl_subsurface is the way to go for supporting embedded Wayland windows. Clients should take a wl_surface they created and a wl_display and pass them to CEF. Depending on whether a parent wl_surface is passed or not, the CEF either will create a toplevel window or a subsurface that it will parent with the passed parent wl_surface."

This proposed mechanism is **not yet upstreamed**. As of January 2025 the only upstream Wayland progress is Toyota's ANGLE Wayland work for top-level windows ([Phoronix Jan 2025](https://www.phoronix.com/news/Chromium-CEF-Wayland-Progress)); embedded sub-surfaces remain a future item. Even when implemented, the constraints are sharp:

> "Calls that manipulate the window state (such as SetBounds(), Hide(), Show()) may not result in actual Wayland calls, as a subsurface is treated as an overlay above a parent surface that cannot be hidden, shown or placed to arbitrary areas."

In other words: even hypothetically, a Wayland subsurface is a constrained overlay, not a free-floating child window like an HWND.

**Interpretation for AgentMux**: a `set_as_child`-equivalent on Wayland does not exist today. Even on X11, the agentmux runtime is launched with native Wayland (Ozone-Wayland, per the user's MEMORY note about CEF Wayland) — so an X11-XID approach would require a deliberate `--ozone-platform=x11` regression. We should treat the native-child-window path as unavailable on Linux for our deployment target.

### 1b. Views-based approach — `browser_view_create` + `Panel::add_child_view`

AgentMux's main browser already uses this path: `browser_view_create()` returns a `CefBrowserView`; `WindowDelegate::on_window_created` adds it via `window.add_child_view(view)`.

Key facts from primary sources:

- `CefView::SetBounds(...)` "sets the bounds (size and position) of a View, where bounds are in parent coordinates, or DIP screen coordinates if there is no parent." ([CefView API docs](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefView.html))
- `CefBrowserView` inherits from `CefView`; it is created without a backing `CefBrowser`. **The browser instance is only created when the view is added to a Widget hierarchy** via `AddedToWidget()` ([cef/libcef/browser/views/browser_view_impl.cc](https://github.com/chromiumembedded/cef/blob/master/libcef/browser/views/browser_view_impl.cc)).
- A `CefPanel` (which `CefWindow` subclasses) supports `AddChildView`, `RemoveChildView`, `Layout` and child enumeration. ([CefPanel API](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefPanel.html))
- **Multi-browser per window is officially confirmed**: "You can add multiple `CefBrowserView` instances in the same `CefWindow` (with Alloy runtime). You can implement a tab strip using `CefButton` (e.g. clicking a button swaps the visible `CefBrowserView`)." — magreenblatt, [forum t=19718](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718).
- The Alloy/Chrome-style constraint (from `browser_view_impl.cc`):
  > "Cannot add Chrome style BrowserView to Alloy style Window"
  > "Cannot add multiple Chrome style BrowserViews"
  
  AgentMux uses Alloy throughout, so neither limit applies.
- BrowserView positioning at runtime is performed by calling `set_bounds(CefRect)` on the BrowserView (or by giving the parent panel a custom layout). Bounds change triggers a callback in `CefBrowserViewImpl` so the underlying browser resizes correctly.

**Interpretation for AgentMux**: this is the supported, cross-platform, future-proof path. With Alloy throughout, our main window can host the primary browser-view *and* N pane browser-views as siblings, each positioned by `set_bounds`.

---

## 2. What does cefclient do for embedded sub-browsers?

[`tests/cefclient/browser/views_window.cc`](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/views_window.cc) is the reference. Behavior:

- **One primary `CefBrowserView` per `CefWindow`.** It is added via `window->AddChildView(browser_view_)` in `OnWindowCreated`.
- Layout uses `CefWindow::SizeToPreferredSize()` for the overall window, with the BrowserView occupying the "main" slot via the default fill layout.
- **Popups are sent to new top-level windows** by default. `OnPopupBrowserViewCreated` returns `false` so CEF wraps the popup in its own `CefWindow::CreateTopLevelWindow`. The sample notes that returning `true` after adding the popup BrowserView yourself is fine — that is the hook for in-window popup embedding, but cefclient itself does not exercise it.
- cefclient does **not** demonstrate multi-pane-per-window in its default flow. The maintainer's guidance for that pattern is "use a `CefButton` to swap visible BrowserView" ([forum t=19718](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718)).
- On Linux specifically, the upstream cefclient native (non-Views) build is **not available in Ozone builds** ([Collabora 2019 upstream](https://www.collabora.com/news-and-blog/blog/2019/05/08/cef-on-wayland-upstreamed/) — "cefclient is not available in Ozone build"). Only the Views path is exercised.

**Interpretation**: cefclient's example doesn't cover our case directly. The primitives we need (multiple BrowserView siblings; `set_bounds`) are well-defined and work, but the reference sample only shows one BrowserView per window plus popups-as-new-windows.

---

## 3. Best-known CEF apps and how they embed sub-browsers

### Steam
Steam's UI uses CEF with a `BrowserView` JS API exposed through `SteamClient.BrowserView`:

> "BrowserView is a subpage embedded in the original webpage, similar to an iframe in a normal web page, but interaction with this object is implemented by Steam itself." ([SteamBrew docs](https://docs.steambrew.app/developers/environment))

Steam's `BrowserView.Create / LoadURL / SetBounds / SetVisible` is **a JS-level abstraction over native CEF BrowserViews** — i.e. they expose the same semantic to their JS layer that we'd be using natively. This is strong evidence that the multi-BrowserView-per-Window pattern scales to a complex production app. The DARKNAVY exploit writeup ([darknavy.org](https://www.darknavy.org/blog/exploiting_steam_usual_and_unusual_ways_in_the_cef_framework/)) corroborates the architecture.

### Spotify
Spotify uses CEF (since 2011, [Spotify Engineering](https://engineering.atspotify.com/2019/3/building-spotifys-new-web-player)) and runs the [`cef-builds.spotifycdn.com`](https://cef-builds.spotifycdn.com/index.html) public binary distribution. No public detail on multi-pane internal architecture.

### 1Password
1Password 8 on Linux uses **Electron**, not raw CEF ([1Password 8 blog](https://blog.1password.com/1password-8-the-story-so-far/)). Not relevant precedent for our problem.

**Interpretation**: Steam is the closest in-the-wild precedent and validates the Views + multiple BrowserViews approach as production-grade.

---

## 4. GTK / Qt CEF integration libraries

- **cefcapi** ([cztomczak/cefcapi](https://github.com/cztomczak/cefcapi)) — C-API example; uses GTK+X11 XID + `SetAsChild`. Pre-Views era.
- **cefpython** ([cztomczak/cefpython, WindowInfo.md](https://github.com/cztomczak/cefpython/blob/master/api/WindowInfo.md)) — `SetAsChild` on Linux requires an X11 XID extracted from a GTK widget. Pre-Wayland.
- **Qt + CEF on Linux** ([Qt forum t=74647](https://forum.qt.io/topic/74647/qt-cef-integration-on-linux)) — "CEF is built on Gtk2, so it requires gtk handle, not the winId()." Pattern is: create a GTK toplevel via `gtk_window_new`, pull its `XID` with `gdk_x11_drawable_get_xid`, hand to `SetAsChild`. **No Wayland path discussed.**
- **cef Rust crate** ([docs.rs/cef v147.1.0](https://docs.rs/cef)) — Tauri-stewarded bindings. Exposes `BrowserView`, `Panel`, `Window`, `BrowserViewDelegate`, `PanelDelegate`. The crate at v147 supports x86_64 + aarch64 on Linux/macOS/Windows. AgentMux already uses this crate (Cargo.lock shows `cef = "...147..."` per the codebase).
- **cef-ui** ([hytopiagg/cef-ui](https://github.com/hytopiagg/cef-ui)) — alternative Rust bindings via bindgen.
- **browser-window** ([bamidev/browser-window](https://github.com/bamidev/browser-window)) — high-level Rust toolkit; uses CEF as a backend; abstracts away direct CEF view APIs.

**Interpretation**: every GTK/Qt integration library targets the X11-XID path. None has a documented Wayland-subsurface path. The Rust `cef` crate AgentMux already uses exposes the Views API surface we need (`BrowserView`, `Window::add_child_view`, `View::set_bounds`).

---

## 5. macOS specifics

### 5a. NSView-based `set_as_child`

`SetAsChild` on macOS takes an NSView pointer cast to `CefWindowHandle` ([forum t=12727](https://magpcss.org/ceforum/viewtopic.php?f=6&t=12727)):

```objc
window_info.SetAsChild(self, 20, 20,
                       self.frame.size.width - 40,
                       self.frame.size.height - 40);
```

Same-process embedding works. Cross-process embedding does not ([forum t=19593](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19593)). Cocoa's "single key window per app" model is cited as the underlying reason ([forum t=19688](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19688)):

> "macOS can only have one key window at a time, which makes it impossible for the Chromium window to receive focus at the same time as the app window."

### 5b. Maintainer's stance on macOS embedding

[cef#3294](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19688) and forum t=19688 establish that **embedded non-Views windows on macOS are explicitly not supported**:

> "I've been reading up on the progress... and am happy to see that this is already supported in windows and linux." (about Linux X11; macOS still missing)

The recommended path on macOS is again Views. Same-process NSView embedding is a "works but unsupported" tier: focus/activation glitches are likely with custom title bars and frameless windows.

**Interpretation for AgentMux**: doing `set_as_child(NSView, rect)` would be a fragile parallel to the Windows code. Given AgentMux is single-process for the host UI, it would *technically* work, but the maintainer's guidance plus our existing ALLOY+Views infrastructure point firmly at the Views approach for macOS as well.

---

## 6. Wayland-specific considerations

1. **Sub-surfaces of a parent xdg_toplevel**: theoretically possible via `wl_subsurface` once CEF exposes the API ([cef#2804](https://github.com/chromiumembedded/cef/issues/2804)). Today, no upstream CEF binary supports this.

2. **Nested top-level windows from same client**: Wayland allows multiple top-levels per client (no protocol prohibition). What it forbids is **arbitrary X11-style reparenting** — there is no equivalent to "embed window A inside window B's coordinate system" except via `wl_subsurface`.

3. **What does Chromium-Ozone do for its DevTools window?** DevTools in Chromium is a separate top-level window when undocked, and an internally-rendered iframe-like region (using Chromium's own Views/Aura) when docked. Crucially, **docked DevTools uses Chromium's internal Views layout system, not OS-level child window embedding** — exactly the model CEF's Views framework exposes.

4. **AgentMux's patched libcef.so** (HEAD `5ab41b6` on `agentmux/7680-drag-rightclick-and-transparency`, per MEMORY): the patches are about transparency (`SetBackgroundOpaque(false)`) and drag (`CefWindow::BeginWindowDrag` -> `WmMoveResizeHandler::DispatchHostWindowDragMovement`). **None of the agentmux patches add the cef#2804 Wayland subsurface API**. Our libcef.so does not bypass the Wayland embedding limitation.

**Interpretation**: on Wayland, child-window embedding for sub-browsers is structurally unavailable; the only viable embedded-pane mechanism is the Views path.

---

## 7. Coordinate / DPI / overlay clipping

The Windows AgentMux pane mechanism uses `SetWindowRgn` to cut transparent holes through the pane HWND so DOM popovers, modals and dropdown menus paint through (`frontend/app/platform/pane-overlay.ts`, lines 1-16). This is the classic "airspace" workaround required because native HWNDs composite above the WebView regardless of CSS z-index.

### Native-child-window approach (Windows / X11):
- The pane window is a separate OS-level surface, composited above the host window content.
- Z-order is controlled at the OS level. CSS z-index has no effect.
- **Clipping must be explicit** (Win32 `SetWindowRgn`; X11 SHAPE extension). `pane-overlay.ts` exists exactly for this.
- DPI: each platform's per-monitor DPI rules apply directly to the pane window.

### Views approach:
- The BrowserView is a `views::View` inside Chromium's Aura/Views compositor — same compositor as the host window.
- **Z-order is controlled via the Views hierarchy and `CefWindow::AddOverlayView`**. ([CefWindow API](https://cef-builds.spotifycdn.com/docs/120.1/classCefWindow.html)) Overlay views are drawn at higher z than regular child views and support docking modes.
- **No airspace problem** — a sibling BrowserView and a sibling label/UI view share one compositor. CSS z-index (within a single browser) plus Views z-order (across browsers) is the entire model.
- DPI: CEF Views use DIP coordinates throughout ([CEF general usage docs](https://chromiumembedded.github.io/cef/general_usage.html)): "Screen/window coordinates are generally represented as density independent pixel (DIP) coordinates with upper-left origin. These DIP coordinates will be passed to Views APIs and browser callbacks." This is much simpler than the Windows pane-overlay flow, which currently does manual integer rounding in `pane-overlay.ts:53-60`.

### Relevant gotchas in Views overlays:
- [cef#3790](https://github.com/chromiumembedded/cef/issues/3790): regression in CEF 125 where overlay BrowserViews didn't display (closed-but-imperfect).
- [cef#4035](https://github.com/chromiumembedded/cef/issues/4035): transparent overlay BrowserViews not yet supported (`GetColor` enforces opaque). Open enhancement.
- AddOverlayView is hidden by default; you must call `CefOverlayController::SetVisible(true)`.

**Interpretation**: switching to Views *eliminates* the airspace problem, so `pane-overlay.ts`'s `SetWindowRgn` workaround can be retired on Linux/macOS — DOM modals naturally paint above sibling BrowserViews via Views z-order. We don't need region-clipping at all in the Views model. (This is a significant complexity reduction.)

---

## 8. Trade-off summary: Views vs Native-child-window for Linux/macOS

| Dimension | Views (`browser_view_create` + `add_child_view`) | Native child window (`set_as_child`) |
| --- | --- | --- |
| **Linux Wayland** | Works (current AgentMux deployment target) | **Does not work** — cef#2804 unimplemented |
| **Linux X11** | Works | Works (with GTK XID extraction) |
| **macOS** | Works, recommended | Works in-process, "not supported" upstream |
| **Windows** | Works | Works (current AgentMux pane path) |
| **Multi-pane per window** | Yes, with Alloy style — confirmed by maintainer | Yes |
| **Z-order vs DOM (airspace)** | No problem — single compositor | Major problem — requires region clipping (`pane-overlay.ts`) |
| **Resize lag** | maintainer: "substantially improved" with Views ([forum t=19718](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718)) | Visible lag in CEF 88+ |
| **DPI handling** | DIP everywhere | Per-platform raw pixel quirks |
| **Frameless / custom title bar** | `CefWindowDelegate::IsFrameless()` (already used for main window) | Manual per-platform |
| **Off-screen rendering compat** | Incompatible | Compatible |
| **Future direction (CEF 126+)** | Recommended; Chrome bootstrap defaults to Alloy style ([cef#3681](https://github.com/chromiumembedded/cef/issues/3681)) | Maintenance-mode for native; Wayland never coming on this path |
| **Cross-platform code** | Single path | Three platform-specific paths |

The maintainer's stance, repeated across multiple threads:
> "This is substantially improved if you use the CEF Views framework. ... The only known way to resolve this issue is by using the Views framework." — magreenblatt, [forum t=19718](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718)

---

## Recommendation

**For AgentMux's Linux + macOS embedded browser pane port: use the Views framework. Add each pane as a sibling `CefBrowserView` child of the existing main `CefWindow`, position it via `View::set_bounds(rect)`, and retire the `pane-overlay.ts` SetWindowRgn clipping mechanism on Linux/macOS.**

### Concrete justification

1. **Linux Wayland forces the decision.** The native-child-window path that powers `browser_pane/creation.rs:84` on Windows has no functional equivalent on Wayland today, and even the proposed `wl_subsurface` mechanism in [cef#2804](https://github.com/chromiumembedded/cef/issues/2804) is constrained (no Hide/Show/SetBounds in the usual sense). Our patched libcef.so does not implement the proposed Wayland API.
2. **AgentMux is already on the Views path for the main browser.** Adding additional `CefBrowserView` children is incremental — same `add_child_view`, same `WindowDelegate`, same Alloy runtime style. No new CEF subsystem to learn.
3. **The Alloy runtime allows N BrowserViews per Window.** The only constraint (from `browser_view_impl.cc`) is "no multiple Chrome-style BrowserViews per Widget" — we use Alloy.
4. **macOS gets the same code path for free.** No NSView swizzling, no Cocoa key-window dance, no maintainer-warning tier.
5. **The airspace / pane-overlay problem disappears.** A sibling BrowserView shares the Aura compositor with the host browser-view; DOM modals in either pane natively respect Views z-order. `pane-overlay.ts`'s SetWindowRgn dance can be skipped on Linux/macOS (it must remain for Windows).
6. **Steam validates the architecture.** Steam's `SteamClient.BrowserView` is a JS facade over the same multi-BrowserView-per-Window primitive.
7. **Resize, DPI, and frameless concerns are all handled better by Views.** Our existing `IsFrameless` delegate already proves this works for the host window.

### Architectural sketch (no code, just shape)

- A new task analogous to `CreateBrowserPaneTask` runs on the CEF UI thread.
- Instead of `WindowInfo::set_as_child + browser_host_create_browser`, it calls `browser_view_create(client, url, settings, None, None, Some(delegate))`.
- The returned `CefBrowserView` is added to the main window via the existing `WindowDelegate` plumbing — with a hook on the host window to call `window.add_child_view(pane_view)` from the UI thread (a thread-marshalled call analogous to today's `post_task(ThreadId::UI, ...)`).
- Position is set via `pane_view.set_bounds(CefRect { x, y, w, h })` in DIP, derived from the same frontend `getBoundingClientRect` flow that already exists.
- Resizes/visibility changes from the frontend translate to `pane_view.set_bounds` and `pane_view.set_visible` calls instead of HWND moves and SetWindowRgn updates.
- For pane-on-top-of-everything cases (modals over a pane), DOM modals just work — Views z-order is automatic for siblings; for cases needing absolute z-elevation, `CefWindow::AddOverlayView` is the escape hatch (with the cef#4035 transparent-overlay limitation noted).
- The Windows path remains as-is. A `cfg!(target_os)` switch in `BrowserPaneManager::create` selects native-child-window on Windows and Views on Linux/macOS. Once the Views path is proven, Windows can migrate too — at which point `pane-overlay.ts` can be retired entirely.

### Risks to monitor

- [cef#3790](https://github.com/chromiumembedded/cef/issues/3790) (overlay BrowserView display regressions) — only a risk if we use `AddOverlayView`; sibling `add_child_view` is the safer base.
- [cef#4035](https://github.com/chromiumembedded/cef/issues/4035) (transparent overlay BrowserViews not yet supported) — if we need transparent panes on top of other panes, this is a future enhancement to track. AgentMux's current usage doesn't require this.
- DPI rounding when translating from frontend pixel coords to CEF DIPs — Views uses DIP natively, so this is mostly handled, but cross-monitor moves on Linux multi-DPI setups should be tested.
- Frontend coordinate system change: the Windows path uses raw screen pixels relative to the parent HWND; the Views path uses DIP relative to the host window's content area. The pane-positioning frontend code needs a small abstraction layer.

---

## Source index (primary)

- [chromiumembedded/cef issue #2804 — Add support for embedded Ozone/Wayland windows](https://github.com/chromiumembedded/cef/issues/2804)
- [chromiumembedded/cef issue #3681 — Lightweight Alloy-style windows in Chrome runtime](https://github.com/chromiumembedded/cef/issues/3681)
- [chromiumembedded/cef issue #3790 — Overlay browser view not shown when using AddOverlayView](https://github.com/chromiumembedded/cef/issues/3790)
- [chromiumembedded/cef issue #4035 — views: Support transparent overlay BrowserViews](https://github.com/chromiumembedded/cef/issues/4035)
- [cef/libcef/browser/views/browser_view_impl.cc — multi-BrowserView constraints](https://github.com/chromiumembedded/cef/blob/master/libcef/browser/views/browser_view_impl.cc)
- [cef/include/views/cef_window.h](https://github.com/chromiumembedded/cef/blob/master/include/views/cef_window.h)
- [cef/tests/cefclient/browser/views_window.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/views_window.cc)
- [cef/tests/cefsimple/simple_app.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefsimple/simple_app.cc)
- [CEF general usage — DIP coordinates](https://chromiumembedded.github.io/cef/general_usage.html)
- [CefView API reference (SetBounds)](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefView.html)
- [CefPanel API reference (AddChildView)](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefPanel.html)
- [CefBrowserView API reference (v120)](https://cef-builds.spotifycdn.com/docs/120.0/classCefBrowserView.html)
- [CEF Forum t=19718 — view lag, multi-BrowserView per window confirmation](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718)
- [CEF Forum t=19688 — macOS embedded non-Views support status](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19688)
- [CEF Forum t=19593 — embedding cefsimple in NSWindow](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19593)
- [CEF Forum t=12727 — CEF browser inside NSWindow on macOS](https://magpcss.org/ceforum/viewtopic.php?f=6&t=12727)
- [CEF Forum t=18048 — embedding CEF into GTK on Linux](https://magpcss.org/ceforum/viewtopic.php?t=18048)
- [CEF Forum t=18750 — Chrome runtime vs Alloy runtime](https://magpcss.org/ceforum/viewtopic.php?f=17&t=18750)
- [CEF Forum t=19186 — managing AddOverlayView BrowserViews](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19186)
- [Qt forum t=74647 — Qt+CEF integration on Linux (X11 XID via GTK)](https://forum.qt.io/topic/74647/qt-cef-integration-on-linux)
- [Collabora 2019 — CEF on Wayland upstreamed (Views-only)](https://www.collabora.com/news-and-blog/blog/2019/05/08/cef-on-wayland-upstreamed/)
- [Phoronix Jan 2025 — CEF Wayland progress (Toyota / ANGLE)](https://www.phoronix.com/news/Chromium-CEF-Wayland-Progress)
- [SteamBrew docs — SteamClient.BrowserView](https://docs.steambrew.app/developers/environment)
- [DARKNAVY — Steam CEF exploitation writeup](https://www.darknavy.org/blog/exploiting_steam_usual_and_unusual_ways_in_the_cef_framework/)
- [docs.rs cef v147.1.0+147.0.10 — Rust bindings (Views API exposed)](https://docs.rs/cef)
- [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs)
- [cefpython WindowInfo (X11 XID pattern)](https://github.com/cztomczak/cefpython/blob/master/api/WindowInfo.md)
- [1Password 8 architecture (uses Electron, not raw CEF)](https://blog.1password.com/1password-8-the-story-so-far/)
- [Spotify Engineering — Building Spotify's New Web Player (CEF since 2011)](https://engineering.atspotify.com/2019/3/building-spotifys-new-web-player)
