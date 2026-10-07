// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPIKE, Windows only, off unless `AGENTMUX_PANE_VIEWS=1`: host a browser
//! pane as a CEF Views `BrowserView` overlay inside the main window (the path
//! Linux and macOS use, `creation_views.rs`) instead of a windowed child HWND.
//!
//! Question it answers (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md
//! §5.2): composited inside the window's own compositor, do panes resize
//! cheaper and in step with the page, with fewer swap chains? Only what a
//! measurement needs is here: create, move (batched), hide, close. Clipping
//! under DOM overlays, focus redirection and the Linux/macOS close dance are
//! not ported. A closed pane's overlay controller is hidden and kept alive,
//! never destroyed (destroying it while Chromium still holds the view is a
//! known FATAL on the other platforms; see `detach_browser_pane_view`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// Whether this run hosts new panes as Views overlays.
pub fn enabled() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| std::env::var("AGENTMUX_PANE_VIEWS").as_deref() == Ok("1"));
    *ON
}

struct Overlay {
    window_label: String,
    controller: OverlayController,
}

/// Live Views panes by label.
static OVERLAYS: LazyLock<Mutex<HashMap<String, Overlay>>> = LazyLock::new(Default::default);
/// Closed panes' controllers, kept alive (see the module doc).
static RETIRED: LazyLock<Mutex<Vec<OverlayController>>> = LazyLock::new(Default::default);
/// Every label hosted this way, recorded before its browser exists: CEF can
/// run `on_after_created` from inside `add_overlay_view`, and the HWND paths
/// it would otherwise take act on the main window.
static LABELS: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default);

/// Whether `label` is a pane this spike hosts.
pub fn is_views_pane(label: &str) -> bool {
    LABELS.lock().contains(label)
}

/// Whether `browser` is the browser of a pane this spike hosts.
pub fn is_views_browser(state: &Arc<AppState>, browser: &Browser) -> bool {
    crate::browser_pane::callbacks::resolve_pane_label(state, browser).is_some_and(|l| is_views_pane(&l))
}

fn window_scale(window: &Window) -> f32 {
    window.display().map(|d| d.device_scale_factor()).filter(|s| *s > 0.0).unwrap_or(1.0)
}

/// Physical client px (what the frontend sends) to the window's DIP.
fn to_dip(rect: &Rect, scale: f32) -> (Point, Size) {
    let s = |v: i32| (v as f32 / scale).round() as i32;
    (Point { x: s(rect.x), y: s(rect.y) }, Size { width: s(rect.width), height: s(rect.height) })
}

/// Create the pane on the UI thread (from `CreateBrowserPaneTask`).
pub fn create(state: Arc<AppState>, block_id: String, label: String, url: String, rect: Rect, window_label: String) {
    let Some(window) = crate::ui_tasks::get_window_on_ui(&state, &window_label) else {
        tracing::error!(%block_id, %label, %window_label, "[views-spike] no Views window for this label; pane not created");
        return;
    };
    state.host_dispatch(crate::reducer::HostCommand::EnqueuePendingWindowCreation {
        entry: crate::state::PendingWindowCreation {
            label: label.clone(),
            kind: crate::state::WindowKind::FullInstance,
            parent_instance_id: None,
        },
    });
    LABELS.lock().insert(label.clone());
    let handler = crate::client::AgentMuxHandler::new_for_creation(state.clone(), true, &label);
    let mut client = Some(crate::client::AgentMuxClient::new(handler, true, false));
    let mut view_delegate = crate::app::AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY);
    let settings = BrowserSettings { background_color: 0xFF000000, ..Default::default() };
    // The window's own RequestContext, as on Linux/macOS (shared Profile; see
    // `create_browser_pane_view` step 5 for the crash this avoids).
    let mut request_context = state.get_browser(&window_label).and_then(|b| b.host()).and_then(|h| h.request_context());
    let url_cef = CefString::from(url.as_str());
    let Some(pane_view) = browser_view_create(
        client.as_mut(),
        Some(&url_cef),
        Some(&settings),
        None,
        request_context.as_mut(),
        Some(&mut view_delegate),
    ) else {
        tracing::error!(%block_id, %label, "[views-spike] browser_view_create returned None");
        return;
    };
    let mut view = View::from(&pane_view);
    let Some(controller) = window.add_overlay_view(Some(&mut view), DockingMode::CUSTOM, 0) else {
        tracing::error!(%block_id, %label, "[views-spike] add_overlay_view returned None");
        return;
    };
    let (pos, size) = to_dip(&rect, window_scale(&window));
    controller.set_size(Some(&size));
    controller.set_position(Some(&pos));
    controller.set_visible(if rect.width > 0 && rect.height > 0 { 1 } else { 0 });
    window.layout();
    // add_overlay_view focuses the new browser; give focus back to the page.
    if let Some(host) = state.get_browser(&window_label).and_then(|b| b.host()) {
        host.set_focus(1);
    }
    let b = controller.bounds();
    tracing::info!(%block_id, %label, scale = window_scale(&window), x = b.x, y = b.y, w = b.width, h = b.height, "[views-spike] pane created as a Views overlay");
    OVERLAYS.lock().insert(label, Overlay { window_label, controller });
}

/// Move Views panes, then lay out each window once. UI thread.
pub fn apply(state: &Arc<AppState>, items: &[(String, Rect)]) {
    let started = std::time::Instant::now();
    let mut windows: Vec<String> = Vec::new();
    {
        let overlays = OVERLAYS.lock();
        for (label, rect) in items {
            let Some(o) = overlays.get(label) else { continue };
            let Some(window) = crate::ui_tasks::get_window_on_ui(state, &o.window_label) else { continue };
            let (pos, size) = to_dip(rect, window_scale(&window));
            o.controller.set_size(Some(&size));
            o.controller.set_position(Some(&pos));
            o.controller.set_visible(if rect.width > 0 && rect.height > 0 { 1 } else { 0 });
            if !windows.contains(&o.window_label) {
                windows.push(o.window_label.clone());
            }
        }
    }
    for w in &windows {
        if let Some(window) = crate::ui_tasks::get_window_on_ui(state, w) {
            window.layout();
        }
    }
    tracing::info!(panes = items.len(), ms = started.elapsed().as_secs_f64() * 1000.0, "[views-spike] moved");
}

/// Close a Views pane. UI thread.
pub fn close(state: &Arc<AppState>, label: &str) {
    LABELS.lock().remove(label);
    let Some(o) = OVERLAYS.lock().remove(label) else { return };
    o.controller.set_visible(0);
    if let Some(host) = state.get_browser(label).and_then(|b| b.host()) {
        host.close_browser(1);
    }
    RETIRED.lock().push(o.controller);
    tracing::info!(%label, "[views-spike] pane closed (controller hidden and retired)");
}

wrap_task! {
    pub struct CloseViewsPaneTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            close(&self.state, &self.label);
        }
    }
}

wrap_task! {
    pub struct ApplyViewsPaneTask {
        state: Arc<AppState>,
        label: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            apply(&self.state, &[(self.label.clone(), self.rect.clone())]);
        }
    }
}
