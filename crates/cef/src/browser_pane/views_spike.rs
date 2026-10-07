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
    /// The pane's rect, physical client px, as last applied.
    rect: Rect,
}

/// DOM overlay rects (menus, modals…) per window, physical client px, from
/// `browser_panes_set_overlay_clip`.
static OVERLAY_CLIPS: LazyLock<Mutex<HashMap<String, Vec<Rect>>>> = LazyLock::new(Default::default);

/// `a` minus `b`: up to four rects covering what of `a` lies outside `b`.
pub(crate) fn subtract(a: &Rect, b: &Rect) -> Vec<Rect> {
    let (ax2, ay2, bx2, by2) = (a.x + a.width, a.y + a.height, b.x + b.width, b.y + b.height);
    let (ix, iy, ix2, iy2) = (a.x.max(b.x), a.y.max(b.y), ax2.min(bx2), ay2.min(by2));
    if ix >= ix2 || iy >= iy2 {
        return vec![a.clone()];
    }
    let mut out = Vec::new();
    if iy > a.y {
        out.push(Rect { x: a.x, y: a.y, width: a.width, height: iy - a.y });
    }
    if iy2 < ay2 {
        out.push(Rect { x: a.x, y: iy2, width: a.width, height: ay2 - iy2 });
    }
    if ix > a.x {
        out.push(Rect { x: a.x, y: iy, width: ix - a.x, height: iy2 - iy });
    }
    if ix2 < ax2 {
        out.push(Rect { x: ix2, y: iy, width: ax2 - ix2, height: iy2 - iy });
    }
    out
}

fn key(r: &Rect) -> (i32, i32, i32, i32) {
    (r.x, r.y, r.width, r.height)
}

/// The parts of `pane` not under any of `holes`, relative to `pane`'s origin.
/// `None` when nothing overlaps (no shape needed).
pub(crate) fn visible_parts(pane: &Rect, holes: &[Rect]) -> Option<Vec<Rect>> {
    let mut parts = vec![pane.clone()];
    let mut overlapped = false;
    for h in holes {
        let next: Vec<Rect> = parts.iter().flat_map(|p| subtract(p, h)).collect();
        if next.len() != parts.len() || next.iter().zip(&parts).any(|(a, b)| key(a) != key(b)) {
            overlapped = true;
        }
        parts = next;
    }
    if !overlapped {
        return None;
    }
    Some(
        parts
            .into_iter()
            .map(|r| Rect { x: r.x - pane.x, y: r.y - pane.y, width: r.width, height: r.height })
            .collect(),
    )
}

/// Whether to call the patched `CefOverlayController::SetShape`. Its slot
/// only exists in a libcef built with the AgentMux overlay-shape patch;
/// reading it from a stock libcef reads past the struct.
fn shape_enabled() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| std::env::var("AGENTMUX_PANE_VIEWS_SHAPE").as_deref() == Ok("1"));
    *ON
}

/// Call the patched `set_shape`: the slot right after the stock struct's last
/// one (`is_drawn`), per the patch's `added=15400` C API.
fn set_shape(controller: &OverlayController, rects_dip: &[Rect]) {
    if !shape_enabled() {
        return;
    }
    type SetShape = unsafe extern "system" fn(*mut cef::sys::_cef_overlay_controller_t, usize, *const cef::sys::_cef_rect_t);
    let raw = ImplOverlayController::get_raw(controller);
    let c_rects: Vec<cef::sys::_cef_rect_t> = rects_dip
        .iter()
        .map(|r| cef::sys::_cef_rect_t { x: r.x, y: r.y, width: r.width, height: r.height })
        .collect();
    unsafe {
        let slot = (raw as *const u8).add(std::mem::size_of::<cef::sys::_cef_overlay_controller_t>()) as *const Option<SetShape>;
        if let Some(f) = *slot {
            f(raw, c_rects.len(), c_rects.as_ptr());
        }
    }
}

/// Recompute one pane's shape from its rect and its window's overlay rects.
fn update_shape(state: &Arc<AppState>, o: &Overlay) {
    let Some(window) = crate::ui_tasks::get_window_on_ui(state, &o.window_label) else { return };
    let scale = window_scale(&window);
    let holes = OVERLAY_CLIPS.lock().get(&o.window_label).cloned().unwrap_or_default();
    let dip = |r: &Rect| {
        let s = |v: i32| (v as f32 / scale).round() as i32;
        Rect { x: s(r.x), y: s(r.y), width: s(r.width), height: s(r.height) }
    };
    match visible_parts(&o.rect, &holes) {
        None => set_shape(&o.controller, &[]),
        // Fully covered: one empty rect, since an empty list means "no shape".
        Some(parts) if parts.is_empty() => set_shape(&o.controller, &[Rect { x: 0, y: 0, width: 0, height: 0 }]),
        Some(parts) => set_shape(&o.controller, &parts.iter().map(dip).collect::<Vec<_>>()),
    }
}

/// Store a window's DOM overlay rects and reshape its Views panes. UI thread.
pub fn set_overlay_clip(state: &Arc<AppState>, window_label: &str, rects: Vec<Rect>) {
    OVERLAY_CLIPS.lock().insert(window_label.to_string(), rects);
    let overlays = OVERLAYS.lock();
    for o in overlays.values().filter(|o| o.window_label == window_label) {
        update_shape(state, o);
    }
}

wrap_task! {
    pub struct OverlayClipTask {
        state: Arc<AppState>,
        window_label: String,
        rects: Vec<Rect>,
    }

    impl Task {
        fn execute(&self) {
            set_overlay_clip(&self.state, &self.window_label, self.rects.clone());
        }
    }
}

/// From the `browser_panes_set_overlay_clip` IPC thread.
pub fn post_overlay_clip(state: &Arc<AppState>, window_label: &str, rects: &[(i32, i32, i32, i32)]) {
    if !enabled() {
        return;
    }
    let rects = rects.iter().map(|&(x, y, w, h)| Rect { x, y, width: w, height: h }).collect();
    let mut task = OverlayClipTask::new(state.clone(), window_label.to_string(), rects);
    post_task(ThreadId::UI, Some(&mut task));
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
    let overlay = Overlay { window_label, controller, rect };
    update_shape(&state, &overlay);
    OVERLAYS.lock().insert(label, overlay);
}

/// Move Views panes, then lay out each window once. UI thread.
pub fn apply(state: &Arc<AppState>, items: &[(String, Rect)]) {
    let started = std::time::Instant::now();
    let mut windows: Vec<String> = Vec::new();
    {
        let mut overlays = OVERLAYS.lock();
        for (label, rect) in items {
            let Some(o) = overlays.get_mut(label) else { continue };
            o.rect = rect.clone();
            let Some(window) = crate::ui_tasks::get_window_on_ui(state, &o.window_label) else { continue };
            let (pos, size) = to_dip(rect, window_scale(&window));
            o.controller.set_size(Some(&size));
            o.controller.set_position(Some(&pos));
            o.controller.set_visible(if rect.width > 0 && rect.height > 0 { 1 } else { 0 });
            update_shape(state, o);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    fn area(rs: &[Rect]) -> i32 {
        rs.iter().map(|r| r.width * r.height).sum()
    }

    #[test]
    fn nothing_overlapping_needs_no_shape() {
        assert!(visible_parts(&r(100, 100, 50, 50), &[r(0, 0, 10, 10)]).is_none());
    }

    #[test]
    fn a_menu_over_a_corner_leaves_the_rest_in_pane_coordinates() {
        let parts = visible_parts(&r(100, 100, 100, 100), &[r(150, 80, 100, 70)]).unwrap();
        assert_eq!(area(&parts), 100 * 100 - 50 * 50);
        assert!(parts.iter().all(|p| p.x >= 0 && p.y >= 0 && p.x + p.width <= 100 && p.y + p.height <= 100));
        assert!(!parts.iter().any(|p| p.x < 100 && p.x + p.width > 50 && p.y < 50 && p.y + p.height > 0));
    }

    #[test]
    fn a_menu_inside_the_pane_cuts_a_hole() {
        let parts = visible_parts(&r(0, 0, 100, 100), &[r(40, 40, 20, 20)]).unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(area(&parts), 100 * 100 - 20 * 20);
    }

    #[test]
    fn overlapping_menus_are_not_double_counted() {
        let parts = visible_parts(&r(0, 0, 100, 100), &[r(10, 10, 40, 40), r(30, 30, 40, 40)]).unwrap();
        assert_eq!(area(&parts), 100 * 100 - (40 * 40 + 40 * 40 - 20 * 20));
    }

    #[test]
    fn fully_covered_leaves_nothing() {
        assert!(visible_parts(&r(10, 10, 50, 50), &[r(0, 0, 100, 100)]).is_some_and(|v| v.is_empty()));
    }
}
