# Embedded Browser Panes — Linux & macOS Port

**Date:** 2026-05-03
**Status:** Spec / Proposal
**Repo state:** main @ `1d887341`, AgentMux v0.33.612
**Author:** AgentC (research delegated to a subagent; full source-cited research at `/tmp/cef-pane-research.md` — keep that file alongside review of this spec)

---

## Problem

`defwidget@browser` (the embedded-webpage pane) renders a black screen on Linux. The cause is structural: `agentmux-cef/src/browser_pane/creation.rs:90-94` short-circuits with a warning on every non-Windows platform:

```rust
#[cfg(not(target_os = "windows"))]
{
    tracing::warn!(block_id = %self.block_id, "browser panes not yet implemented on this platform");
    return;
}
```

The frontend creates the pane DIV and fires the `browser_pane_create` IPC; the Rust side returns without creating a CEF browser; the DIV stays empty. Same code path will black-screen on macOS when that build target is enabled.

The Windows implementation uses **native child windows**: `find_own_top_level_window()` → `WindowInfo::set_as_child(parent_hwnd, &rect)` → `browser_host_create_browser`. That paradigm has no working analogue on Wayland and is officially unsupported on macOS (see Research §1, §5, §6). The port has to choose a different mechanism.

---

## TL;DR

- **Use the CEF Views framework** for Linux + macOS pane creation: `browser_view_create(...)` returns a `CefBrowserView` that we add as a sibling child of the existing main `CefWindow` via `add_child_view`, and position with `view.set_bounds(rect)` in DIP.
- **Drives the decision: Wayland.** `WindowInfo::set_as_child` does not work on Wayland (cef#2804 unimplemented; even the proposed `wl_subsurface` mechanism is sharply constrained — no Hide/Show/SetBounds in the usual sense). Our patched `libcef.so` on `agentmux/7680-...` does not add this API.
- **Substrate is already in place.** AgentMux's main browser already uses `browser_view_create + add_child_view` (`agentmux-cef/src/app.rs:426` and `WindowDelegate::on_window_created`). Pane creation becomes "do the same thing, but for an additional view, and `set_bounds` it where the frontend pane lives."
- **`pane-overlay.ts` SetWindowRgn airspace dance becomes obsolete on Linux/macOS.** Sibling BrowserViews share Aura's compositor with the host UI, so DOM modals naturally sort by Views z-order — no region clipping needed. Eventually Windows can migrate too and `pane-overlay.ts` retires entirely.
- **Estimated change**: 1 new ~150-line module (`browser_pane/creation_views.rs`), 4-line `cfg!(target_os)` switch in the existing creation entry point, ~30-line frontend coordinate-system shim, ~50 lines added to `WindowDelegate` for the "add this view to the window when ready" plumbing, no changes to `pane-overlay.ts` for now (only Linux/macOS skip it). One bump, one binary patch to the host. No CEF source changes; the Views API we need is already exported by cef-dll-sys / cef 146.7.

---

## Why Views, not native-child-window — the hard constraints

Three independent reasons make the native-child-window path unusable for our deployment target. Any one of them would suffice; together they're decisive.

### 1. Wayland forbids OS-level child-window embedding (Research §1, §6)

From the CEF maintainer on cef#2804 about Ozone-Wayland:

> "Ozone/Wayland/X11 can only be used with views framework now, but it does not allow to embed host windows into client windows."

The proposed `wl_subsurface` API in the same issue is **not upstreamed** as of 2026-05. Even if it were, sub-surfaces are constrained to be passive overlays of the parent surface — they can't be hidden/shown/moved with the same freedom as an HWND. AgentMux deploys natively on Wayland (per the `cef_wayland.md` MEMORY note); a pane that can't be hidden/shown is unusable.

X11 fallback (`--ozone-platform=x11`) would technically work via GTK XID extraction + `set_as_child`, but regressing AgentMux to XWayland on Linux just to support panes is a non-starter.

### 2. macOS embedded non-Views windows are explicitly unsupported (Research §5)

Maintainer guidance on cef-forum t=19688:

> "macOS can only have one key window at a time, which makes it impossible for the Chromium window to receive focus at the same time as the app window."

Same-process NSView embedding works in practice but lives in a "works but unsupported" tier — focus and activation glitches are likely with our frameless ALLOY-style window and custom title bar. Views is the recommended path for macOS for the same reason as Linux.

### 3. The native path requires a per-platform code branch we'd own forever

Three platforms × the existing Windows code = three independent integrations to maintain (HWND, X11 XID via GTK, NSView). Each has subtle DPI rules and focus quirks. The Views path collapses this to one cross-platform code path per the CEF maintainer's repeated guidance (cef-forum t=19718):

> "This is substantially improved if you use the CEF Views framework. ... The only known way to resolve this issue is by using the Views framework."

---

## Why Views works for AgentMux specifically

Five things have to be true for the Views approach to fit. All five are.

1. **The Alloy runtime style allows multiple `CefBrowserView` siblings per `CefWindow`.** The constraint in `libcef/browser/views/browser_view_impl.cc` ("Cannot add multiple Chrome style BrowserViews") only applies to Chrome-style, which AgentMux does not use. Maintainer confirms on cef-forum t=19718: "You can add multiple CefBrowserView instances in the same CefWindow (with Alloy runtime)."

2. **Our main window is already a `CefWindow` hosting a `CefBrowserView` via `add_child_view`.** See `agentmux-cef/src/app.rs:426-437` and `WindowDelegate::on_window_created`. Pane addition is the same primitive a second time.

3. **`CefView::SetBounds` lets a child be positioned anywhere within the parent's coordinate space, in DIP.** From the CefView API docs: "Sets the bounds (size and position) of a View, where bounds are in parent coordinates, or DIP screen coordinates if there is no parent." Resize, move, and visibility changes from the frontend become single calls.

4. **Z-order between siblings is handled by Views, not by the OS compositor.** A pane BrowserView and a host BrowserView share Aura's compositor. DOM modals in the host paint above the pane naturally; the existing `pane-overlay.ts` SetWindowRgn workaround is unnecessary on this path. (The airspace problem only exists when native OS surfaces composite outside the WebView's compositor.)

5. **Steam validates the architecture in production.** Steam's `SteamClient.BrowserView.Create / LoadURL / SetBounds / SetVisible` JS API is a thin facade over exactly this multi-`CefBrowserView`-per-Window primitive. SteamBrew docs explicitly compare it to "an iframe in a normal web page." Production-scale precedent.

---

## Design

### Architecture

```
┌──────────────────────── CefWindow (main, frameless ALLOY) ───────────────┐
│                                                                          │
│  ┌─────────── BrowserView (host UI — current main browser) ──────────┐   │
│  │  set_bounds(0, 0, win_w, win_h)  — fills the window               │   │
│  │  Renders the SolidJS frontend, which contains pane DIVs and       │   │
│  │  reports their on-screen rects to Rust via IPC.                   │   │
│  └───────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌─────── BrowserView (pane #1, label="browser-pane-<block_id>") ────┐   │
│  │  set_bounds(pane_x, pane_y, pane_w, pane_h)                       │   │
│  │  Loads the user's URL; sized to match the frontend pane DIV.      │   │
│  └───────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌─────── BrowserView (pane #N) ──────────────────────────────────────┐  │
│  │  ... etc ...                                                       │  │
│  └────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

**Key invariant:** the host UI BrowserView occupies the full window. Pane BrowserViews are added on top of (z-order: later siblings) the host UI BrowserView and positioned to cover specific rects. Because Views composites them in one pipeline, the frontend's CSS-positioned pane DIV is conceptually a "hole" through which the pane BrowserView shows — but at the OS level it's just two siblings, no clipping, no airspace.

### Module layout

| File | Purpose | Status |
|---|---|---|
| `agentmux-cef/src/browser_pane/creation.rs` | Existing entry point. Add a `cfg(target_os = "windows")` switch at line 78ish: Windows path stays as-is; Linux/macOS path delegates to the new `creation_views` module. | Edit |
| `agentmux-cef/src/browser_pane/creation_views.rs` | New module. Implements `create_browser_pane_view(state, block_id, label, url, rect)` running on the CEF UI thread. Calls `browser_view_create(client, url, settings, None, None, Some(view_delegate))`, hands the resulting `CefBrowserView` to the main `WindowDelegate` via a thread-marshalled `add_child_view + set_bounds` call. | New |
| `agentmux-cef/src/app.rs` | `AgentMuxWindowDelegate` gains a method to host pane BrowserViews. Two options below in "Open question 1"; pick after a quick spike. | Edit |
| `agentmux-cef/src/browser_pane/callbacks.rs` | The existing `on_after_created_browser_pane` / `on_before_close_browser_pane` / `on_load_end_browser_pane` callbacks already work generically over the `Browser` handle and don't depend on HWND. They should fire for Views-created panes too — but verify by tracing. | Verify, no change expected |
| `agentmux-cef/src/browser_pane/hwnd.rs` (379 lines) | Windows-specific HWND management for native pane windows. Untouched on this PR. (When Windows eventually migrates to Views, this whole file can be deleted.) | Untouched |
| `frontend/app/platform/pane-overlay.ts` | The SetWindowRgn-equivalent clipping mechanism. Skip its work on Linux/macOS — Views z-order handles it. **Today's pane-overlay.ts only fires `browser_panes_set_overlay_clip` IPC; that IPC's Rust handler should no-op on non-Windows.** | Edit (Rust handler), maybe edit (frontend) |
| `agentmux-cef/src/ipc.rs` | The five `browser_pane_*` IPCs (`create`, `navigate`, `resize`, `close`, `go_back`, `go_forward`) keep their current dispatch shape. `BrowserPaneManager::resize` switches on cfg to use `view.set_bounds` on Linux/macOS instead of HWND `SetWindowPos`. | Edit (small) |

### Data flow — pane creation, end-to-end

1. **Frontend:** user opens `defwidget@browser` widget. `BrowserViewModel` (`frontend/app/view/browser/browser-model.ts`) renders a pane DIV with a `block_id`. After mount, it measures the DIV's screen rect and fires `browser_pane_create` IPC with `{ block_id, url, rect: {x, y, w, h} }`.

2. **Rust IPC dispatcher** (`agentmux-cef/src/ipc.rs:312`): unchanged, calls `state.browser_panes.create(state, block_id, url, rect)`.

3. **`BrowserPaneManager::create`** (`agentmux-cef/src/browser_panes.rs:157`): unchanged. Reducer-routed `TryRegisterBrowserPaneLive` returns `Fresh(label)` (or `AlreadyLive`/`Closing`). Posts `CreateBrowserPaneTask` to `ThreadId::UI`.

4. **`CreateBrowserPaneTask::execute`** (`agentmux-cef/src/browser_pane/creation.rs:30`-ish): NEW switch at the top:
   ```
   #[cfg(target_os = "windows")]    →  existing HWND path
   #[cfg(not(target_os = "windows"))] →  delegate to creation_views::create_browser_pane_view
   ```

5. **`create_browser_pane_view` (NEW, `creation_views.rs`):**
   - Build `client = AgentMuxClient::new(handler, true)` (same handler-with-pane setup as Windows).
   - Build `view_delegate = AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY)` (the same delegate type already used for the main browser at `app.rs:172`).
   - Call `browser_view_create(client, &CefString::from(url), &BrowserSettings::default(), None, None, Some(view_delegate))` → returns `Option<BrowserView>`.
   - Stash the `BrowserView` in `state.browser_panes` keyed by `label` (so subsequent `resize`/`close`/`navigate` can find it).
   - Marshal an "add this view to the main window at this rect" call to the WindowDelegate (see Open question 1).

6. **WindowDelegate** (on the same UI thread): `window.add_child_view(&mut View::from(&pane_view))` and `pane_view.as_view().set_bounds(rect)`.

7. **CEF internally:** `CefBrowserViewImpl::AddedToWidget()` fires, which actually creates the underlying `CefBrowser`. Our `Client::on_after_created` runs (existing handler), our `on_load_end_browser_pane` runs when the URL finishes loading.

8. **Resize flow:** frontend observes its DIV's rect via `ResizeObserver`, fires `browser_pane_resize` IPC. Rust looks up `BrowserView` by label and calls `pane_view.as_view().set_bounds(new_rect)`. No HWND `SetWindowPos`, no `pane-overlay.ts` clip update needed on Linux/macOS.

9. **Close flow:** `browser_pane_close` IPC → Rust calls `pane_view.as_view().parent_view()?.remove_child_view(pane_view.as_view())` (or directly via the cached parent panel handle). The browser's `on_before_close` cleans up state via the existing reducer path.

### Coordinate model

| Surface | Coordinate system | Origin | Unit |
|---|---|---|---|
| Frontend pane DIV's `getBoundingClientRect()` | CSS pixels relative to the document's viewport | Top-left of viewport | CSS pixel |
| Today's Windows path | Screen pixels relative to parent HWND content area | Top-left of HWND client area | Raw pixel (DPI-scaled separately) |
| Views path (Linux/macOS) | DIP relative to parent View's bounds | Top-left of parent View | DIP |

The frontend already sends pane rects in CSS pixels relative to the viewport. The viewport in our model = the host BrowserView's content area = the parent of the pane BrowserView in the Views hierarchy. So **CSS pixels → DIPs is essentially identity at zoom factor 1.0**; if the user has set a CEF zoom factor, we multiply by it.

This is *simpler* than the Windows path (which has to deal with per-monitor DPI scaling separately from zoom). One conversion in the Rust resize handler:
```
dip_rect.x = (css_rect.x * zoom_factor) as i32;
... etc.
```

The zoom factor is queryable via `host.zoom_level()` on the host browser; cache and update via the existing `set_zoom_factor` IPC.

---

## Design decisions

### D1. Sibling BrowserView, not OverlayView

`CefWindow::AddOverlayView` is the alternative entry point — it adds a BrowserView at higher z than regular child views, with an `OverlayController` for visibility and docking modes. Why not use it?

- **Open issues:** cef#3790 (overlay BrowserView display regressions in some CEF versions), cef#4035 (transparent overlay BrowserViews not yet supported — the GetColor enforces opaque). Adding panes via the overlay path inherits these.
- **Z-order overkill:** OverlayView guarantees "above all regular children." We don't need that — a pane should be sandwiched between the host UI's background and the host UI's modals. Sibling z-order via `add_child_view` order is exactly what we want.
- **Layout coupling:** Overlay views have docking modes (`TopLeft`, `TopRight`, etc.) that conflict with pane positioning by absolute rect.

**Decision:** plain `add_child_view` + `set_bounds`. AddOverlayView is reserved as the escape hatch if we later need a pane to genuinely float above modals.

### D2. One BrowserView per pane, not one BrowserView reused via reload

Steam's BrowserView abstraction is per-pane and they don't reuse views across navigations. Reuse-with-reload is feasible (call `frame.load_url(new)` on the existing view) and we already use it in `BrowserPaneManager::create`'s `RegisterResult::AlreadyLive` branch. Keep that as-is for navigations within an existing pane; **create a fresh view only when a pane block-id is newly registered.**

### D3. Pane lifecycle tracked in state.browser_panes (existing), keyed by label

`BrowserPaneManager` (`browser_panes.rs`) already keys live panes by label. The Windows path stores the `Browser` handle there for IPC lookup. The Views path stores `BrowserView` (which has its own `Browser` accessor via `pane_view.browser()`). Same key, different value type. Use `enum PaneHandle { Native(Browser), View(BrowserView) }` (or platform-cfg the type) — the latter is cleaner because no other code needs to discriminate at runtime.

**Decision:** platform-cfg the stored type. `PaneHandle = Browser` on Windows, `PaneHandle = BrowserView` on Linux/macOS. Methods that need a `Browser` go through `pane_handle.browser()` (Windows: identity; Linux/macOS: the BrowserView's accessor).

### D4. `pane-overlay.ts` SetWindowRgn dance: skip on Linux/macOS, leave Windows alone

The clipping IPC `browser_panes_set_overlay_clip` (called from `pane-overlay.ts:50`) is Windows-specific: it cuts transparent regions through a native HWND. On Linux/macOS the pane is a sibling Views child — DOM modals in the host UI naturally cover it via Views z-order. Make the Rust handler a no-op on non-Windows. The frontend can keep firing the IPC; the handler just returns `Null`.

**Optimization (later):** the frontend can also stop computing/sending overlay clip rects on non-Windows once the Rust no-op is confirmed in production. Defer; it's free perf only when CPU is contended.

### D5. The Views resize path runs on the UI thread; resize IPCs marshal there

Today's `BrowserPaneManager::resize` calls into Windows APIs that must run on a specific thread (the UI thread). The Views path has the same constraint — `View::set_bounds` must run on the CEF UI thread. Use the same `post_task(ThreadId::UI, ...)` pattern. No new infra.

### D6. macOS uses the same code path as Linux

The Views API works identically on macOS, and the Cocoa key-window concern that affects native-child embedding doesn't apply to Views (the BrowserView lives inside the host's NSWindow, not as a separate top-level). One `cfg(not(target_os = "windows"))` branch covers both. Test on macOS once we have a build target; no separate code path expected.

### D7. Don't migrate Windows yet

The Windows native-child path works, ships, and doesn't bottleneck Linux/macOS work. Migrating it to Views would mean retiring `pane-overlay.ts` and `browser_pane/hwnd.rs` (379 lines), validating that resize behavior is identical, and re-testing all the multi-window taskbar grouping logic that interacts with HWNDs. That's a separate PR.

**This PR's scope:** Linux + macOS via Views. Windows untouched. A follow-up PR can converge.

---

## Open questions / verification before implementation

### 1. How does the WindowDelegate "host this new pane view" call get marshalled?

Two options:

**(a)** The `AgentMuxWindowDelegate` keeps a list of pending pane views (`RefCell<Vec<BrowserView>>`). After delegate construction, before window mount, the pane creation task pushes views in. `on_window_created` adds them. **Problem:** panes can be created long after the main window has been created (a user clicks "open browser pane" mid-session). We need a hook for late additions.

**(b)** Cache a handle to the main `Window` itself in `AppState` (e.g. `state.main_window: Mutex<Option<Window>>`) — populated by `WindowDelegate::on_window_created`. The pane creation task reads the handle on the UI thread and calls `window.add_child_view(view)` directly.

**Decision needed in spike:** (b) is cleaner but requires storing a `cef::Window` (refcounted via `RefGuard`) in `AppState` — verify the type is `Send + Sync` enough or wrap appropriately. The cef Rust crate's `Rc` trait already supports cross-thread refcounting; should be fine.

### 2. What `BrowserViewDelegate` fields are required?

Today's `AgentMuxBrowserViewDelegate` (`app.rs:172`) overrides `on_popup_browser_view_created` to wrap popups in their own top-level windows. For pane BrowserViews, popup behavior should match (a popup in a pane → new top-level window, not embedded in the pane). Reuse the existing delegate type; no new override needed. Confirm via spike.

### 3. Click + focus routing

When the user clicks inside a pane BrowserView, it should get focus. CEF Views handles this automatically via the Aura focus manager — clicks on a child view focus that view. Confirm: clicking the host UI again returns focus to the host. No special code expected; verify via manual test.

### 4. Pane ordering when multiple panes overlap

If two panes overlap (two BrowserViews with overlapping bounds in the same parent), the later-added one is on top. Match this to the frontend's z-order intent. Frontend likely doesn't expect overlapping panes today, but confirm and document.

### 5. Hidden / minimized panes

Frontend sometimes hides a pane (e.g. tab not active). On Windows we use `ShowWindow(hwnd, SW_HIDE)`. On Views: `view.set_visible(false)` or `view.set_bounds({0, 0, 0, 0})`. The first is cleaner; verify the BrowserView pauses rendering when `set_visible(false)` (it should; that's the Views contract).

---

## Implementation plan

1. **Spike (1-2 hours):** in a throwaway branch, hard-code a single pane creation in `WindowDelegate::on_window_created` to validate that:
   - `browser_view_create` returns a usable view on Linux.
   - `window.add_child_view(view)` plus `view.set_bounds(rect)` displays the URL at the right rect.
   - Click-to-focus and resize-to-bounds work.
   - DOM modals in the host UI render above the pane.

2. **Resolve open question 1** (WindowDelegate handle storage). Pick (a) or (b) based on the spike.

3. **Implement** `agentmux-cef/src/browser_pane/creation_views.rs` with `create_browser_pane_view(state, block_id, label, url, rect)`.

4. **Edit** `agentmux-cef/src/browser_pane/creation.rs` to delegate non-Windows to the new module.

5. **Edit** `BrowserPaneManager::resize / close / navigate` to take the Views path on Linux/macOS. Likely a `match` on the cfg-typed `PaneHandle` enum.

6. **Edit** `pane_overlay`'s Rust handler in `ipc.rs` to no-op on non-Windows.

7. **Frontend coordinate-system shim:** verify the existing `getBoundingClientRect`-derived rect can pass directly to the IPC. If yes, no frontend change. If no, wrap the rect with a zoom-factor multiplier.

8. **Test plan** below.

9. **Bump patch + build AppImage + open PR.** AppImage build flow already in place from PR #669.

---

## Test plan

On Linux/Wayland (GNOME/Mutter):

- [ ] Open `defwidget@browser`. Pane shows the URL (was black before).
- [ ] Resize the pane (drag the splitter). BrowserView resizes to match.
- [ ] Resize the main window. Pane BrowserView reflows correctly.
- [ ] Open a second browser pane in the same tab. Both render correctly side-by-side.
- [ ] Open a DOM modal (e.g. an agent picker overlay). Modal renders above the pane.
- [ ] Click inside the pane → pane gets focus, scrolling works.
- [ ] Click outside (host UI) → focus returns to host UI.
- [ ] Right-click in the pane → pane's contextmenu fires (not the host's).
- [ ] Right-click on the host UI header → host's contextmenu fires (not the pane's).
- [ ] Navigate within the pane (URL bar). Pane navigates; host doesn't.
- [ ] Close the pane (X button). Pane disappears; host BrowserView still works; no zombie BrowserView in `state.browser_panes`.
- [ ] Open a second window from the status bar (PR #666 fix). Open a pane in the second window. Both windows have independent panes.
- [ ] Close the second window with a pane open. No host crash; no zombie state.

When macOS becomes a build target:

- [ ] Same tests pass on macOS.

---

## Risks / non-goals

- **Risk: cef#3790 / cef#4035** — overlay-related regressions in some CEF versions. Mitigation: we use `add_child_view`, not `AddOverlayView`. If we later need true overlay behavior, watch those issues.
- **Risk: BrowserView lifecycle subtleties** — Views creates the underlying `CefBrowser` only on `AddedToWidget`. If we cache the `BrowserView` before adding it to the window, calling `browser()` on it returns `None` until the add. Order operations carefully (add to window first, then cache).
- **Non-goal: migrating Windows.** Out of scope. Windows native-child path stays. `pane-overlay.ts` stays for Windows.
- **Non-goal: transparent panes overlapping other panes.** cef#4035 limitation. Not currently used.
- **Non-goal: cross-process panes.** All panes live in the host process, same as Windows today.

---

## File-by-file change summary

**New:**
- `agentmux-cef/src/browser_pane/creation_views.rs` (~150 lines)

**Edited:**
- `agentmux-cef/src/browser_pane/creation.rs` — add `cfg(target_os = "windows")` switch at the top of `CreateBrowserPaneTask::execute`; delegate non-Windows to `creation_views`.
- `agentmux-cef/src/browser_pane/mod.rs` — re-export the new module.
- `agentmux-cef/src/browser_panes.rs` — `PaneHandle` enum or platform-cfg type alias; resize/close/navigate methods updated to use Views API on non-Windows.
- `agentmux-cef/src/app.rs` — `AgentMuxWindowDelegate` plumbing per Open Question 1; or `AppState` gains a `main_window` handle.
- `agentmux-cef/src/ipc.rs` — `browser_panes_set_overlay_clip` handler no-ops on non-Windows.

**Untouched:**
- `agentmux-cef/src/browser_pane/hwnd.rs` (Windows-only).
- `agentmux-cef/src/browser_pane/callbacks.rs` (works generically over `Browser`).
- `frontend/app/platform/pane-overlay.ts` (kept as-is; Rust handler ignores its IPC on non-Windows).
- macOS / Windows packaging.

---

## Source references

The full source-cited research backing this spec lives at `/tmp/cef-pane-research.md` (also archive at `docs/research/cef-pane-research-2026-05-03.md` if needed). Primary sources:

- [chromiumembedded/cef issue #2804 — embedded Ozone/Wayland windows](https://github.com/chromiumembedded/cef/issues/2804)
- [chromiumembedded/cef issue #3681 — Lightweight Alloy-style windows in Chrome runtime](https://github.com/chromiumembedded/cef/issues/3681)
- [CEF Forum t=19718 — multi-BrowserView per window confirmation by maintainer](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718)
- [CEF Forum t=19688 — macOS embedded non-Views support status](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19688)
- [Collabora 2019 — CEF on Wayland upstreamed (Views-only)](https://www.collabora.com/news-and-blog/blog/2019/05/08/cef-on-wayland-upstreamed/)
- [SteamBrew docs — SteamClient.BrowserView](https://docs.steambrew.app/developers/environment)
- [cef/libcef/browser/views/browser_view_impl.cc — multi-BrowserView constraints](https://github.com/chromiumembedded/cef/blob/master/libcef/browser/views/browser_view_impl.cc)
- [cef/tests/cefclient/browser/views_window.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/views_window.cc)
- [CefView::SetBounds docs](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefView.html)
- [CefBrowserView API reference](https://cef-builds.spotifycdn.com/docs/120.0/classCefBrowserView.html)
