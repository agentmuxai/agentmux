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

    // 5. Create the BrowserView. Underlying Browser is constructed lazily on
    //    AddedToWidget below.
    let pane_view = match browser_view_create(
        client.as_mut(),
        Some(&url_cef),
        Some(&settings),
        None,
        None,
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

    // 6. Add as an OVERLAY (not a regular child). Overlays are positioned
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
        1, // can_activate
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

    // 7. Position the overlay. We tried `set_bounds(rect)` directly: silently
    //    ignored — readback shows 0,0,0,0. Trying separate set_size + set_position
    //    and explicit window.layout() to force a layout pass.
    overlay_controller.set_size(Some(&Size { width: rect.width, height: rect.height }));
    overlay_controller.set_position(Some(&Point { x: rect.x, y: rect.y }));
    overlay_controller.set_visible(1);
    parent_window.layout();
    tracing::info!(
        block_id = %block_id, label = %label,
        x = rect.x, y = rect.y, w = rect.width, h = rect.height,
        "[browser-pane] views: overlay set_size + set_position + set_visible + layout() applied"
    );

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
    // OverlayController::set_bounds is silently ignored even with
    // DockingMode::CUSTOM (verified during initial spike — readback
    // showed bounds stayed 0,0,0,0 after set_bounds calls). The
    // working pattern is set_size + set_position separately, then
    // a window.layout() on the OWNING window to force the layout pass.
    controller.set_size(Some(&Size { width: rect.width, height: rect.height }));
    controller.set_position(Some(&Point { x: rect.x, y: rect.y }));
    if let Some(window) = state.windows.lock().get(&window_label).cloned() {
        window.layout();
    }
    tracing::debug!(
        label = %label, window_label = %window_label,
        x = rect.x, y = rect.y, w = rect.width, h = rect.height,
        "[browser-pane] views: resize applied (set_size + set_position + layout)"
    );
}

/// Detach + drop a Views-based browser pane (Linux/macOS only).
///
/// Called from `BrowserPaneManager::close` on non-Windows. Order matters:
///
///   1. Look up the underlying `Browser` for this label and call
///      `BrowserHost::close_browser(force=1)`. Empirically `OverlayController::destroy`
///      DOES eventually trigger `on_before_close`, but per CEF's documentation
///      the destroy path is layout-tracking only and the Browser's lifecycle
///      is independent — without the explicit close request a Browser can
///      survive its overlay's destruction and stay registered in `state.browsers`
///      with stale callbacks (PR #682 codex P2).
///   2. Destroy the OverlayController to remove the overlay from the parent.
///   3. Drop the cached handle. `on_before_close` fires asynchronously on the
///      Browser and the existing `callbacks::on_before_close_browser_pane`
///      drains state.browsers + the reducer's pane map. If close_browser
///      fails (already-gone race), the destroy still cleans up the overlay.
///
/// Must run on the CEF UI thread.
pub fn detach_browser_pane_view(state: &Arc<AppState>, label: &str) {
    let entry = state.browser_pane_overlays.lock().remove(label);
    let Some((_window_label, controller)) = entry else {
        tracing::debug!(
            label = %label,
            "[browser-pane] views: detach requested but no OverlayController found"
        );
        return;
    };

    // Step 1: ask the Browser to close BEFORE destroying its overlay.
    if let Some(browser) = state.get_browser(label) {
        if let Some(host) = browser.host() {
            host.close_browser(1); // force=1 — pane is going away regardless
            tracing::debug!(label = %label, "[browser-pane] views: close_browser(force=1) requested");
        }
    } else {
        tracing::debug!(
            label = %label,
            "[browser-pane] views: no Browser registered at detach (already drained?)"
        );
    }

    // Step 2: destroy the overlay.
    controller.destroy();
    tracing::info!(
        label = %label,
        "[browser-pane] views: OverlayController destroyed"
    );
    drop(controller);
}
