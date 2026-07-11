// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux/macOS browser-pane creation via the CEF Views framework.
//!
//! On Linux Wayland the Windows-style `WindowInfo::set_as_child(parent_hwnd, rect)`
//! path doesn't work (cef#2804 — embedded Wayland subsurfaces aren't upstreamed,
//! and even when they ship the API is too constrained for our use case). On
//! macOS NSView embedding is officially unsupported by the CEF maintainer for
//! new code (cef-forum t=19688). The recommended path for both platforms is the
//! Views framework.
//!
//! ## Embedding mechanism: AddOverlayView, not AddChildView
//!
//! We tried `Window::add_child_view + View::set_bounds` first (PR #669 design
//! spec called for it) and the result was: pane rendered at full window size,
//! "flashing" on every Wayland frame, because CEF's Window has a default
//! FillLayout that re-asserts "fill parent" on every layout pass and clobbers
//! the explicit `set_bounds` we wrote. With multiple FillLayout children both
//! get full-parent-size bounds, so the pane stacked on top of the host UI at
//! full size.
//!
//! `Window::add_overlay_view(view, docking_mode, can_activate)` is the API
//! designed for the "view at arbitrary position above other children" case.
//! The returned `CefOverlayController` exposes `set_size(Size)` /
//! `set_position(Point)` / `set_visible(bool)` / `destroy()` that aren't
//! subject to the parent's auto-layout. We pass `DockingMode::CUSTOM` to
//! opt out of corner-docking — every other DockingMode forces auto-positioning
//! and silently ignores `set_size` / `set_position` / `set_bounds` calls.
//! Verified empirically during the spike: with TOP_LEFT the overlay filled
//! the whole window; with CUSTOM and just `set_bounds(rect)` the overlay
//! was 0×0; only the `set_size + set_position + window.layout()` triplet
//! reads back as the requested rect.
//!
//! Risks (see PR #669 spec §"Risks"):
//!   - cef#3790 — overlay BrowserView display regressions in CEF 125. Our
//!     libcef.so is 146.0.179, post-fix.
//!   - cef#4035 — transparent overlay BrowserViews not yet supported. We use
//!     opaque background, so this isn't a blocker.
//!
//! ## Lifecycle
//!
//! 1. `browser_view_create(client, url, settings, ..., delegate)` constructs
//!    a `CefBrowserView`. The underlying `CefBrowser` is created lazily —
//!    only after the view is added to a Widget hierarchy via `AddedToWidget`.
//! 2. Look up the cached primary `CefWindow` from `state.windows`
//!    (populated by `AgentMuxWindowDelegate::on_window_created`).
//! 3. `window.add_overlay_view(view, DockingMode::CUSTOM, can_activate=1)`
//!    returns an `OverlayController`. CEF then creates the underlying
//!    Browser, which fires `on_after_created` on our existing handler — same
//!    callback path as the Windows pane (`callbacks::on_after_created_browser_pane`).
//! 4. `controller.set_bounds(rect)` positions + sizes the pane within the
//!    window's content area. Bounds are in DIP relative to the window.
//! 5. Stash the `OverlayController` in `state.browser_pane_overlays` keyed by
//!    label, so subsequent `BrowserPaneManager::resize` / `close` can locate it.
//!
//! The host reducer's `RegisterBrowser` command fires from `client.rs::on_after_created`
//! (existing code) and registers the underlying `Browser` in `state.browsers`, so
//! the rest of the pane API (`navigate`, frame access, etc.) works through the
//! same Browser-handle path as on Windows.

use std::sync::Arc;

use cef::{
    browser_view_create, BrowserSettings, CefString, DockingMode, ImplBrowser, ImplBrowserHost,
    ImplOverlayController, ImplPanel, ImplView, ImplWindow, Point, Rect, RuntimeStyle, Size, View,
};

use crate::state::AppState;

/// Create a Views-based browser pane on the CEF UI thread.
///
/// Caller is `CreateBrowserPaneTask::execute` (`browser_pane/creation.rs`),
/// which is itself posted via `post_task(ThreadId::UI, ...)` from
/// `BrowserPaneManager::create`. So this function runs on the UI thread.
pub fn create_browser_pane_view(
    state: Arc<AppState>,
    block_id: String,
    label: String,
    url: String,
    rect: Rect,
    window_label: String,
) {
    tracing::info!(
        block_id = %block_id,
        label = %label,
        url = %url,
        window_label = %window_label,
        x = rect.x, y = rect.y, w = rect.width, h = rect.height,
        "[browser-pane] views: create_browser_pane_view begin"
    );

    // 1. Locate the parent CefWindow by its label.
    let parent_window = {
        let guard = state.windows.lock();
        guard.get(&window_label).cloned()
    };
    let parent_window = match parent_window {
        Some(w) => w,
        None => {
            tracing::error!(
                block_id = %block_id, label = %label, window_label = %window_label,
                "[browser-pane] views: no Window registered for this window_label — pane creation aborted"
            );
            return;
        }
    };

    // 2. Pre-create handoff to the host reducer — same as Windows path. on_after_created
    //    uses this to look up the pane's intended kind / parent linkage from the
    //    pending-creations queue.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind: crate::state::WindowKind::FullInstance,
                parent_instance_id: None,
            },
        },
    );

    // 3. Build the per-pane CEF Client (handler with is_browser_pane = true).
    let handler = crate::client::AgentMuxHandler::new_with_browser_pane(state.clone(), 0, true);
    let mut client = Some(crate::client::AgentMuxClient::new(handler, true));

    // 4. BrowserViewDelegate. Reuse the same delegate as the main browser —
    //    its on_popup_browser_view_created behavior (popups → new top-level
    //    windows) is what we want for panes too.
    let mut view_delegate =
        crate::app::AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY);

    let settings = BrowserSettings {
        windowless_frame_rate: 60,
        background_color: 0xFF000000, // opaque black baseline
        ..Default::default()
    };

    let url_cef = CefString::from(url.as_str());

    // 5. Resolve the parent window's RequestContext. Critical for the
    //    multi-window observer-list crash fix (see spec
    //    docs/specs/pane-shares-window-request-context-linux-2026-05-13.md):
    //    every isolated RequestContext yields a different `Profile*` pointer
    //    but they all share one `ThemeService` instance (chrome's
    //    `ThemeServiceFactory` redirects to the original profile). The pane
    //    used to pass `None` here, getting the global default Profile —
    //    different from the parent window's Profile — and
    //    `CefWidgetImpl::AddAssociatedProfile` would then re-add the widget
    //    as an observer of the shared ThemeService, tripping the
    //    "Observers can only be added once!" CHECK and FATAL-crashing the
    //    host. Reusing the parent window's RequestContext means the
    //    pane's Profile matches the window's main browser's Profile, so
    //    the map check fires and AddObserver is skipped.
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

    // 6. Create the BrowserView. Underlying Browser is constructed lazily on
    //    AddedToWidget below.
    let pane_view = match browser_view_create(
        client.as_mut(),
        Some(&url_cef),
        Some(&settings),
        None,
        request_context.as_mut(),
        Some(&mut view_delegate),
    ) {
        Some(v) => v,
        None => {
            tracing::error!(
                block_id = %block_id, label = %label,
                "[browser-pane] views: browser_view_create returned None"
            );
            return;
        }
    };
    tracing::info!(
        block_id = %block_id, label = %label,
        "[browser-pane] views: browser_view_create succeeded"
    );

    // 7. Add as an OVERLAY (not a regular child). Overlays are positioned
    //    via OverlayController::set_bounds rather than the parent's layout,
    //    so they cohabit cleanly with the host UI's full-window BrowserView.
    //    DockingMode::CUSTOM tells CEF "don't auto-dock, I'll set bounds
    //    myself" — the corner modes (TOP_LEFT etc.) IGNORE subsequent
    //    set_bounds calls (verified empirically: spike with TOP_LEFT showed
    //    set_bounds returning success but bounds() reading back as 0,0,0,0).
    //    can_activate=1 — pane should be able to receive keyboard focus
    //    (clicking inside the pane focuses it).
    let mut view = View::from(&pane_view);
    let overlay_controller = parent_window.add_overlay_view(
        Some(&mut view),
        DockingMode::CUSTOM,
        0, // can_activate=0: overlay never steals key/main status.
           // RWHVC::mouseDown: processes clicks because we swizzle
           // NativeWidgetMacNSWindow::isMainWindow → YES in the task.
    );

    let overlay_controller = match overlay_controller {
        Some(oc) => oc,
        None => {
            tracing::error!(
                block_id = %block_id, label = %label,
                "[browser-pane] views: add_overlay_view returned None — pane lost"
            );
            return;
        }
    };
    tracing::info!(
        block_id = %block_id, label = %label,
        "[browser-pane] views: add_overlay_view returned OverlayController"
    );

    // 7. Position the overlay.
    //
    // On macOS, the overlay's native NSView is not yet created at this point —
    // `add_overlay_view` posts native view creation asynchronously. Calling
    // `set_size` / `set_position` / `layout()` here is a best-effort attempt
    // that MAY commit the bounds on Linux but silently does nothing on macOS
    // (readback: oc_w=0 oc_h=0). Without committed bounds the overlay defaults
    // to filling the entire parent window, which (a) covers the UI with the
    // pane's opaque-black background_color and (b) intercepts all mouse events
    // for the window — the "black screen + UI freeze" bug.
    //
    // macOS strategy (v15): [NSWindow windowWithWindowNumber:] was removed in
    // macOS 26 Tahoe. Instead, we capture the overlay NSWindow NOW — it is still
    // present in [NSApp windows] because set_visible(0)/orderOut: has not been
    // called yet. We call setFrame:display:YES immediately (commits CEF Views
    // bounds via windowDidResize:), then call set_visible(0) to hide it.
    // The deferred SetPaneBoundsViewsTask calls set_visible(1) and reaffirms the
    // frame (in case the show path resets it), then restores key/focus.
    //
    // On macOS, set_size/set_position/layout are skipped — all no-ops or harmful:
    //   - set_size and set_position are no-ops on NativeWidgetMacNSWindow (CEF Views bug)
    //   - parent_window.layout() schedules an ASYNC layout pass that fires AFTER our
    //     SetPaneBoundsViewsTask and resets the overlay to wrong/full bounds, triggering
    //     CEF's event routing for the wrong area → full-window mouse freeze.
    #[cfg(not(target_os = "macos"))]
    {
        overlay_controller.set_size(Some(&Size { width: rect.width, height: rect.height }));
        overlay_controller.set_position(Some(&Point { x: rect.x, y: rect.y }));
        parent_window.layout();
    }

    // macOS: scan [NSApp windows] RIGHT NOW (overlay is still ordered-in) to find
    // the new NativeWidgetMacNSWindow and call setFrame before we hide it.
    #[cfg(target_os = "macos")]
    let overlay_wnum: isize = unsafe {
        use std::ffi::c_char;
        type Id  = *mut std::ffi::c_void;
        type Sel = *const std::ffi::c_void;

        extern "C" {
            fn sel_registerName(name: *const c_char) -> Sel;
            fn objc_msgSend();
            fn object_getClassName(obj: Id) -> *const c_char;
            fn objc_getClass(name: *const c_char) -> Id;
        }

        #[repr(C)] #[derive(Copy,Clone)] struct NSPoint { x: f64, y: f64 }
        #[repr(C)] #[derive(Copy,Clone)] struct NSSize  { w: f64, h: f64 }
        #[repr(C)] #[derive(Copy,Clone)] struct NSRect  { origin: NSPoint, size: NSSize }

        let get_id:        extern "C" fn(Id, Sel) -> Id        = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_usize:     extern "C" fn(Id, Sel) -> usize     = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_isize:     extern "C" fn(Id, Sel) -> isize     = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_bool:      extern "C" fn(Id, Sel) -> u8        = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_f64:       extern "C" fn(Id, Sel) -> f64       = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let obj_at:        extern "C" fn(Id, Sel, usize) -> Id = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_frame:     extern "C" fn(Id, Sel) -> NSRect    = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let set_frame_d:   extern "C" fn(Id, Sel, NSRect, u8)  = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);

        let sel_shared_app    = sel_registerName(b"sharedApplication\0".as_ptr() as _);
        let sel_windows_arr   = sel_registerName(b"windows\0".as_ptr() as _);
        let sel_count         = sel_registerName(b"count\0".as_ptr() as _);
        let sel_obj_at        = sel_registerName(b"objectAtIndex:\0".as_ptr() as _);
        let sel_frame         = sel_registerName(b"frame\0".as_ptr() as _);
        let sel_is_main       = sel_registerName(b"isMainWindow\0".as_ptr() as _);
        let sel_backing_scale = sel_registerName(b"backingScaleFactor\0".as_ptr() as _);
        let sel_set_frame_d   = sel_registerName(b"setFrame:display:\0".as_ptr() as _);
        let sel_win_number    = sel_registerName(b"windowNumber\0".as_ptr() as _);

        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
        let ns_app     = get_id(ns_app_cls, sel_shared_app);
        let all_wins   = get_id(ns_app, sel_windows_arr);
        let win_count  = if all_wins.is_null() { 0usize } else { get_usize(all_wins, sel_count) };

        // Find the newest NativeWidgetMacNSWindow (highest windowNumber = just created)
        // and the main CefNSWindow (for frame/scale reference).
        let mut main_win:   Id     = std::ptr::null_mut();
        let mut overlay_win: Id    = std::ptr::null_mut();
        let mut overlay_wnum: isize = 0;

        for i in 0..win_count {
            let win = obj_at(all_wins, sel_obj_at, i);
            if win.is_null() { continue; }
            let cls_ptr = object_getClassName(win);
            let cls = if !cls_ptr.is_null() {
                std::ffi::CStr::from_ptr(cls_ptr).to_str().unwrap_or("?")
            } else { "?" };
            let wn      = get_isize(win, sel_win_number);
            let is_main = get_bool(win, sel_is_main);
            let fr      = get_frame(win, sel_frame);
            tracing::info!(
                label = %label, i, win_count, class = cls,
                x = fr.origin.x, y = fr.origin.y, w = fr.size.w, h = fr.size.h,
                is_main, wn,
                "[browser-pane] ObjC pre-hide NSApp window"
            );
            if cls.contains("CefNSWindow") && is_main != 0 {
                main_win = win;
            }
            // Newest NativeWidgetMacNSWindow = highest windowNumber = the overlay we just created.
            if cls.contains("NativeWidgetMacNSWindow") && wn > overlay_wnum {
                overlay_win = win;
                overlay_wnum = wn;
            }
        }

        if !overlay_win.is_null() && !main_win.is_null() {
            let main_fr = get_frame(main_win, sel_frame);
            let scale = {
                let s = get_f64(main_win, sel_backing_scale);
                if s > 0.0 { s } else { 1.0 }
            };
            let pane_x = rect.x as f64 / scale;
            let pane_y = rect.y as f64 / scale;
            let pane_w = rect.width  as f64 / scale;
            let pane_h = rect.height as f64 / scale;
            let screen_x = main_fr.origin.x + pane_x;
            let screen_y = main_fr.origin.y + main_fr.size.h - pane_y - pane_h;
            let target = NSRect {
                origin: NSPoint { x: screen_x, y: screen_y },
                size:   NSSize  { w: pane_w, h: pane_h },
            };
            set_frame_d(overlay_win, sel_set_frame_d, target, 1u8);
            let new_fr = get_frame(overlay_win, sel_frame);
            tracing::info!(
                label = %label, scale, overlay_wnum,
                pane_x, pane_y, pane_w, pane_h, screen_x, screen_y,
                main_x = main_fr.origin.x, main_y = main_fr.origin.y,
                main_w = main_fr.size.w, main_h = main_fr.size.h,
                req_w = rect.width, req_h = rect.height,
                got_x = new_fr.origin.x, got_y = new_fr.origin.y,
                got_w = new_fr.size.w, got_h = new_fr.size.h,
                "[browser-pane] ObjC pre-hide setFrame applied"
            );
        } else {
            tracing::warn!(
                label = %label,
                overlay_found = !overlay_win.is_null(),
                main_found = !main_win.is_null(),
                win_count,
                "[browser-pane] ObjC pre-hide: could not find overlay or main window"
            );
        }

        overlay_wnum
    };
    #[cfg(not(target_os = "macos"))]
    let overlay_wnum: isize = 0;

    overlay_controller.set_visible(0); // deferred task will show after bounds commit
    tracing::info!(
        block_id = %block_id, label = %label,
        x = rect.x, y = rect.y, w = rect.width, h = rect.height,
        "[browser-pane] views: overlay set_size + set_position + layout() applied (visible=0; deferred show pending)"
    );

    // 7b. Return focus to the main window's BrowserView. CEF's default
    //     on_after_created behaviour (fired from add_overlay_view above when
    //     the BrowserView is added to the widget hierarchy) gives focus to the
    //     new pane browser. On Windows, AgentMuxHandler::on_after_created_browser_pane
    //     installs a WM_SETFOCUS redirect subclass that catches this steal.
    //     On macOS/Linux there is no equivalent subclass, so the OverlayController
    //     holds focus after creation and its BrowserView intercepts all keyboard
    //     (and, depending on CEF hit-test state, mouse) events for the parent window —
    //     causing the "black pane + frozen UI" bug. Explicitly refocus the main
    //     window browser here; the user can then click inside the pane to focus it.
    if let Some(main_browser) = state.get_browser(&window_label) {
        if let Some(mut host) = main_browser.host() {
            host.set_focus(1);
            tracing::info!(
                window_label = %window_label,
                "[browser-pane] views: returned focus to main window after pane creation"
            );
        }
    }

    // 8. Diagnostic: read back what CEF thinks of the overlay state.
    //    If is_drawn=0 or is_visible=0 with our settings, something below
    //    Views is rejecting the layout. If bounds are 0,0,0,0 the set_bounds
    //    didn't take. If bounds match our request but user sees black, the
    //    Wayland sub-surface compositing for AddOverlayView is broken.
    let oc_visible = overlay_controller.is_visible();
    let oc_drawn = overlay_controller.is_drawn();
    let oc_bounds = overlay_controller.bounds();
    // BrowserView's view methods come via the View facade.
    let pane_as_view = View::from(&pane_view);
    let view_bounds = pane_as_view.bounds();
    let view_drawn = pane_as_view.is_drawn();
    let view_attached = pane_as_view.is_attached();
    tracing::info!(
        block_id = %block_id, label = %label,
        oc_visible, oc_drawn,
        oc_x = oc_bounds.x, oc_y = oc_bounds.y, oc_w = oc_bounds.width, oc_h = oc_bounds.height,
        view_x = view_bounds.x, view_y = view_bounds.y, view_w = view_bounds.width, view_h = view_bounds.height,
        view_drawn, view_attached,
        "[browser-pane] views: post-create diagnostic readback"
    );

    // 9. Stash the controller AND the parent window's label for later
    //    resize/close lookups (so they can find the right Window for
    //    `layout()` calls without re-deriving from the pane name).
    state
        .browser_pane_overlays
        .lock()
        .insert(label.clone(), (window_label.clone(), overlay_controller));
    tracing::info!(
        block_id = %block_id, label = %label, window_label = %window_label,
        "[browser-pane] views: OverlayController stashed in state.browser_pane_overlays"
    );

    // Seed the physical-rect cache with the creation-time rect — see the
    // doc comment on `AppState::browser_pane_physical_rects` for why this
    // (not `controller.bounds()`) is the airspace-hide path's source of
    // truth for this pane's on-screen rect on macOS.
    #[cfg(target_os = "macos")]
    state
        .browser_pane_physical_rects
        .lock()
        .insert(label.clone(), (rect.x, rect.y, rect.width, rect.height));

    // 10. Deferred bounds + show: re-apply set_size / set_position / layout() / set_visible(1)
    //     on the next UI event-loop tick. On macOS the overlay's native NSView is
    //     created asynchronously during add_overlay_view; the first-tick sizing
    //     (step 7 above) is silently ignored because the native layer doesn't exist
    //     yet. Posting a task ensures the bounds land AFTER CEF has initialised the
    //     NSView. The task also re-issues set_focus(1) on the main browser in case
    //     a focus steal happened during the event-loop tick.
    crate::ui_tasks::post_set_pane_bounds_views(
        &state,
        &label,
        &window_label,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        0, // retry counter
        overlay_wnum,
    );
}

/// Resize a Views-based browser pane (Linux/macOS only).
///
/// Called from `BrowserPaneManager::resize` on non-Windows. Looks up the
/// cached OverlayController by label and calls `set_bounds`.
///
/// Must run on the CEF UI thread. Caller is responsible for the
/// `post_task(ThreadId::UI, ...)` marshalling.
pub fn resize_browser_pane_view(state: &Arc<AppState>, label: &str, rect: Rect) {
    let entry = state.browser_pane_overlays.lock().get(label).cloned();
    let Some((window_label, controller)) = entry else {
        tracing::debug!(
            label = %label,
            "[browser-pane] views: resize requested but no OverlayController found (already closed?)"
        );
        return;
    };
    let pane_rect = (rect.x, rect.y, rect.width, rect.height);
    // Keep the airspace path's rect source (see `AppState::browser_pane_physical_rects`)
    // fresh on every resize — this is the only place non-Windows learns the
    // pane's current physical on-screen rect.
    #[cfg(target_os = "macos")]
    state.browser_pane_physical_rects.lock().insert(label.to_string(), pane_rect);
    let visible = crate::browser_panes::compute_pane_visible(state, &window_label, pane_rect);

    // On macOS, CEF's SetSize/SetPosition are permanent no-ops on NativeWidgetMacNSWindow,
    // and window.layout() triggers NativeWidgetMac::SetBounds() which resets the frame
    // back to wrong bounds (since CEF's Views never received a real size). Use the ObjC
    // NSWindow setFrame: path instead — same as the creation task.
    #[cfg(target_os = "macos")]
    if visible {
        tracing::debug!(
            label = %label, window_label = %window_label,
            "[browser-pane] resize_browser_pane_view: visible=true, scheduling SetPaneBoundsViewsTask (will unconditionally set_visible(1) on execute)"
        );
        let overlay_wnum = state.browser_pane_overlay_wnums
            .try_lock()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0);
        crate::ui_tasks::post_set_pane_bounds_views(
            state,
            label,
            &window_label,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            0,
            overlay_wnum,
        );
        return;
    }

    // On macOS, if NOT visible — just hide the overlay without a layout pass.
    #[cfg(target_os = "macos")]
    {
        controller.set_visible(0);
        tracing::debug!(
            label = %label, window_label = %window_label,
            "[browser-pane] resize_browser_pane_view: set_visible(0) (not visible)"
        );
        return;
    }

    // Linux: CEF Views SetSize/SetPosition work correctly.
    // OverlayController::set_bounds is silently ignored even with
    // DockingMode::CUSTOM (verified during initial spike — readback
    // showed bounds stayed 0,0,0,0 after set_bounds calls). The
    // working pattern is set_size + set_position separately, then
    // a window.layout() on the OWNING window to force the layout pass.
    #[cfg(not(target_os = "macos"))]
    {
        controller.set_size(Some(&Size { width: rect.width, height: rect.height }));
        controller.set_position(Some(&Point { x: rect.x, y: rect.y }));
        // Visibility = AND of the two independent hide reasons:
        //  - Zero-area rect (frontend placed the placeholder in `display:none`
        //    when the tab is inactive → getBoundingClientRect reports 0×0).
        //  - This pane's bounds intersect a registered overlay-clip rect for
        //    its window (DOM modal/menu/tooltip is on top of it).
        // Both are tracked in AppState; `compute_pane_visible` is the single
        // authoritative answer, shared with `SetPaneOverlayClipViewsTask`.
        // Without consulting overlay-clip state here, a positive-rect resize
        // (e.g. user drags a splitter while a modal is open) would clobber
        // the airspace's set_visible(0) — Codex review on PR #881.
        controller.set_visible(if visible { 1 } else { 0 });
        if let Some(window) = state.windows.lock().get(&window_label).cloned() {
            window.layout();
        }
        tracing::debug!(
            label = %label, window_label = %window_label,
            x = rect.x, y = rect.y, w = rect.width, h = rect.height,
            visible,
            "[browser-pane] views: resize applied (set_size + set_position + visibility + layout)"
        );
    }
}

/// Detach a Views-based browser pane (Linux/macOS only).
///
/// Called from `BrowserPaneManager::close` on non-Windows. Order matters:
///
///   1. Pop the controller out of `state.browser_pane_overlays` so future
///      resize / focus / overlay-clip calls become no-ops.
///   2. Stash the controller in `state.pending_overlay_destroy`. Keeping it
///      alive here is critical: dropping or destroying it now would yank
///      the BrowserView out of its parent Window's hierarchy while
///      Chromium still has UI-thread tasks holding `WeakPtr<View>` to it,
///      tripping `weak_ptr.h:250 Check failed: ref_.IsValid()` and FATALing
///      the host. Reproducers: pane-close-then-pool-spawn (0.33.721) and
///      tab tear-off (0.33.722).
///   3. Call `BrowserHost::close_browser(force=1)`. This triggers async
///      Browser teardown; CEF will eventually call our `on_before_close`
///      handler, which drains `pending_overlay_destroy[label]` and runs
///      the actual `controller.destroy()` — by that point the Browser is
///      fully gone and any queued WeakPtr-bearing tasks have drained, so
///      destroy can no longer race.
///
/// Must run on the CEF UI thread.
pub fn detach_browser_pane_view(state: &Arc<AppState>, label: &str) {
    let entry = state.browser_pane_overlays.lock().remove(label);
    #[cfg(target_os = "macos")]
    state.browser_pane_physical_rects.lock().remove(label);
    let Some((window_label, controller)) = entry else {
        tracing::debug!(
            label = %label,
            "[browser-pane] views: detach requested but no OverlayController found"
        );
        return;
    };

    // Remove this pane's per-window statics entry.  Keyed by pane label so two
    // panes on the same window each hold an independent entry; the sendEvent:
    // gate is deactivated only when all pane entries are gone.
    crate::ui_tasks::clear_pane_swizzle_statics(label);
    #[cfg(target_os = "macos")]
    { state.browser_pane_overlay_wnums.try_lock().map(|mut m| m.remove(label)); }

    // Whether to defer destroy depends on whether there's a live Browser.
    //   - Live Browser + host: close_browser(force=1) fires; on_before_close
    //     will land asynchronously; stash so the callback finds the
    //     controller and runs destroy() then. (Synchronous destroy here
    //     races Chromium's queued WeakPtr<View> tasks → weak_ptr.h:250
    //     FATAL; that's the whole reason for this dance.)
    //   - No live Browser (already drained, or host gone): on_before_close
    //     won't fire — stashing would leak the controller in
    //     pending_overlay_destroy forever (reagent P1 on PR #788).
    //     No Browser means no queued WeakPtr tasks against this view, so
    //     destroying synchronously is safe.
    let live_host = state
        .get_browser(label)
        .and_then(|mut b| b.host());

    if let Some(host) = live_host {
        state
            .pending_overlay_destroy
            .lock()
            .insert(label.to_string(), controller);
        host.close_browser(1);
        tracing::info!(
            label = %label,
            "[browser-pane] views: close_browser(force=1) requested; OverlayController stashed for deferred destroy at on_before_close"
        );
    } else {
        // No Browser / no host → on_before_close won't fire. Destroy now to
        // avoid the leak. No race risk: there's no live Browser holding
        // WeakPtrs to our BrowserView.
        controller.destroy();
        tracing::info!(
            label = %label,
            "[browser-pane] views: no live Browser at detach — OverlayController destroyed synchronously"
        );
    }
}

