// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_panes_set_rects`: move a window's panes in one step.
//!
//! The frontend used to send one `browser_pane_resize` per pane per frame, and
//! each was applied on its own by a cross-thread `SetWindowPos` from the IPC
//! thread that waited for the UI thread. A window drag over nine panes queued
//! up to 111 requests, so panes trailed the page by about half a second
//! (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md). Now the frontend
//! sends one batch per window with at most one in flight
//! (`frontend/app/view/browser/pane-rect-batcher.ts`), and here:
//!
//! - A batch merges into the window's pending rects (a pane's newer rect
//!   replaces its older one) and one UI task is scheduled for the window if
//!   none is waiting. Batches can't pile up on the UI thread.
//! - That task takes everything pending and applies it: on Windows every
//!   pane's wrapper moves in one `BeginDeferWindowPos`/`EndDeferWindowPos`
//!   per parent window, so they move together; elsewhere each pane goes
//!   through its usual Views resize, all in the one task.
//! - The IPC call returns once its rects have been applied, or after
//!   `APPLY_WAIT` if the UI thread is busy, so the page's next batch follows
//!   the host's pace without ever waiting on a stuck UI thread.
//! - A pane is moved only while it is live and still in the window that sent
//!   the batch: a rect from a window a pane was torn off from is dropped.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cef::*;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::state::AppState;

use super::BrowserPaneManager;

/// How long `set_rects` waits for its rects to be applied before answering anyway.
const APPLY_WAIT: Duration = Duration::from_millis(250);

/// One pane's new rect, in the window's physical client pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneBounds {
    pub block_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PaneBounds {
    fn rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, width: self.width, height: self.height }
    }
}

/// Parse `{ window_label, rects: [{ block_id, x, y, width, height }] }`.
pub(crate) fn parse_set_rects_args(args: &serde_json::Value) -> Result<(String, Vec<PaneBounds>), String> {
    let window_label = args
        .get("window_label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("browser_panes_set_rects: missing window_label")?
        .to_string();
    let rects = args
        .get("rects")
        .and_then(|v| v.as_array())
        .ok_or("browser_panes_set_rects: missing rects")?;
    let int = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let mut out = Vec::with_capacity(rects.len());
    for r in rects {
        let Some(block_id) = r.get("block_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(PaneBounds {
            block_id: block_id.to_string(),
            x: int(r, "x"),
            y: int(r, "y"),
            width: int(r, "width"),
            height: int(r, "height"),
        });
    }
    Ok((window_label, out))
}

/// What the page laid a batch out against, in the rects' physical pixels: its
/// viewport and the tile layout holding the panes.
///
/// The page learns a window's new size a frame or more after the window has
/// it, and a pane only moves once the page has laid out and sent its rect,
/// so during a drag the pane's content trails the page's chrome around it.
/// On macOS and Linux the host moves shown panes as soon as the window's size
/// changes (`follow_window_resize`, `window_resized`), stretching the page's
/// last rects by the change:
/// the tile layout keeps its insets from the window's edges and every pane
/// scales with it. The page's own rect for the new size replaces the guess.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) struct LayoutBase {
    viewport: (i32, i32),
    /// x, y, width, height of the tile layout; the whole viewport if unknown.
    layout: (i32, i32, i32, i32),
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
impl LayoutBase {
    /// `{ viewport: { width, height }, layout?: { x, y, width, height } }` from a batch.
    pub(crate) fn parse(args: &serde_json::Value) -> Option<Self> {
        let int = |v: &serde_json::Value, k: &str| v.get(k).and_then(|n| n.as_i64()).map(|n| n as i32);
        let vp = args.get("viewport")?;
        let viewport = (int(vp, "width")?, int(vp, "height")?);
        if viewport.0 <= 0 || viewport.1 <= 0 {
            return None;
        }
        let layout = args
            .get("layout")
            .and_then(|l| Some((int(l, "x")?, int(l, "y")?, int(l, "width")?, int(l, "height")?)))
            .filter(|l| l.2 > 0 && l.3 > 0)
            .unwrap_or((0, 0, viewport.0, viewport.1));
        Some(Self { viewport, layout })
    }

    /// The width of the viewport the batch was laid out in.
    pub(crate) fn viewport_width(&self) -> i32 {
        self.viewport.0
    }

    /// Where `rect`, laid out against this base, goes in a viewport of
    /// `width` x `height`.
    pub(crate) fn predict(&self, rect: &Rect, width: i32, height: i32) -> Rect {
        let (lx, ly, lw, lh) = self.layout;
        let sx = (lw + width - self.viewport.0).max(1) as f64 / lw as f64;
        let sy = (lh + height - self.viewport.1).max(1) as f64 / lh as f64;
        let map = |v: i32, origin: i32, s: f64| origin + ((v - origin) as f64 * s).round() as i32;
        let (x, y) = (map(rect.x, lx, sx), map(rect.y, ly, sy));
        let right = map(rect.x + rect.width, lx, sx);
        let bottom = map(rect.y + rect.height, ly, sy);
        Rect { x, y, width: (right - x).max(1), height: (bottom - y).max(1) }
    }
}

/// What each pane's latest rect was laid out against, by pane label. Kept
/// per pane, not per window: a batch can carry only some of a window's panes
/// (a display's scale changed and only some panes' timers have reported), and
/// a pane's rect must always be read against the base it was measured with.
#[cfg_attr(target_os = "windows", allow(dead_code))]
static LAYOUT_BASES: LazyLock<Mutex<HashMap<String, LayoutBase>>> = LazyLock::new(Default::default);

/// Remember what pane `label`'s rect was laid out against (`None`: a rect
/// with no base, which is used as sent). UI thread, as the rect is applied.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn note_layout_base(label: &str, base: Option<LayoutBase>) {
    let mut bases = LAYOUT_BASES.lock();
    match base {
        Some(b) => bases.insert(label.to_string(), b),
        None => bases.remove(label),
    };
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn layout_base(label: &str) -> Option<LayoutBase> {
    LAYOUT_BASES.lock().get(label).copied()
}

/// The window's size changed: move its shown panes to where the page will
/// put them, without waiting for it. UI thread only.
#[cfg(target_os = "macos")]
pub(crate) fn follow_window_resize(state: &Arc<AppState>, window_label: &str) {
    let panes: Vec<(String, cef::OverlayController)> = state
        .browser_pane_overlays
        .lock()
        .iter()
        .filter(|(_, (w, _))| w == window_label)
        .map(|(label, (_, c))| (label.clone(), c.clone()))
        .collect();
    for (label, controller) in panes {
        let Some(base) = layout_base(&label) else { continue };
        let Some((x, y, width, height)) = state.browser_pane_physical_rects.lock().get(&label).copied() else {
            continue;
        };
        let wnum = state.browser_pane_overlay_wnums.try_lock().and_then(|m| m.get(&label).copied()).unwrap_or(0);
        crate::ui_tasks::move_shown_pane_overlay(&controller, wnum, &Rect { x, y, width, height }, Some(&base));
    }
}

/// The window's size changed (Linux): move its shown panes to where the page
/// will put them, without waiting for it, and lay the window out once. The
/// page's own rects for the new size replace the guess (`apply`). UI thread
/// only.
#[cfg(target_os = "linux")]
pub(crate) fn window_resized(state: &Arc<AppState>, window_label: &str, window: &Window) {
    let started = std::time::Instant::now();
    let (width, height) = window_client_px(window);
    // The callback also fires when the window only moves (X11). Panes sit
    // relative to the window, so a move needs nothing from them.
    if CLIENT_SIZES.lock().insert(window_label.to_string(), (width, height)) == Some((width, height)) {
        return;
    }
    tracing::debug!(window = window_label, width, height, "[pane-bounds] window resized");
    let panes: Vec<(String, cef::OverlayController)> = state
        .browser_pane_overlays
        .lock()
        .iter()
        .filter(|(_, (w, _))| w == window_label)
        .map(|(label, (_, c))| (label.clone(), c.clone()))
        .collect();
    let mut moved = 0;
    for (label, controller) in panes {
        if controller.is_visible() == 0 {
            continue;
        }
        let Some(base) = layout_base(&label) else { continue };
        let Some((x, y, w, h)) = state.browser_pane_physical_rects.lock().get(&label).copied() else {
            continue;
        };
        let to = base.predict(&Rect { x, y, width: w, height: h }, width, height);
        controller.set_size(Some(&Size { width: to.width, height: to.height }));
        controller.set_position(Some(&Point { x: to.x, y: to.y }));
        moved += 1;
    }
    if moved > 0 {
        window.layout();
    }
    tracing::debug!(
        window = window_label,
        width,
        panes = moved,
        ms = started.elapsed().as_secs_f64() * 1000.0,
        "[pane-bounds] followed"
    );
}

/// Each window's client size when `window_resized` last saw it, by label.
#[cfg(target_os = "linux")]
static CLIENT_SIZES: LazyLock<Mutex<HashMap<String, (i32, i32)>>> = LazyLock::new(Default::default);

/// The window `window_label` is gone: forget its size.
#[cfg(target_os = "linux")]
pub(crate) fn forget_window(window_label: &str) {
    CLIENT_SIZES.lock().remove(window_label);
}

/// `window`'s client area in physical pixels: what the page's viewport
/// measures (`innerWidth * devicePixelRatio`).
#[cfg(target_os = "linux")]
pub(crate) fn window_client_px(window: &Window) -> (i32, i32) {
    let scale = window.display().map(|d| d.device_scale_factor()).filter(|s| *s > 0.0).unwrap_or(1.0) as f64;
    let client = window.client_area_bounds_in_screen();
    ((client.width as f64 * scale).round() as i32, (client.height as f64 * scale).round() as i32)
}

/// A window's rects waiting for the UI thread, and who is waiting on them.
#[derive(Default)]
struct Pending {
    /// In first-arrival order; a newer rect for a pane replaces its entry in place.
    /// Each with what its batch was laid out against; installed with the
    /// rect, so a window resize never reads a rect against another's base.
    rects: Vec<(PaneBounds, Option<LayoutBase>)>,
    waiters: Vec<oneshot::Sender<()>>,
    /// A UI task for this window is posted and hasn't run yet.
    scheduled: bool,
}

#[derive(Default)]
struct PendingByWindow(HashMap<String, Pending>);

impl PendingByWindow {
    /// Merge `rects` into `window`'s pending set. Returns the receiver that
    /// fires once they are applied, and whether the caller must post a task.
    fn enqueue(
        &mut self,
        window: &str,
        rects: Vec<PaneBounds>,
        base: Option<LayoutBase>,
    ) -> (oneshot::Receiver<()>, bool) {
        let p = self.0.entry(window.to_string()).or_default();
        for r in rects {
            match p.rects.iter_mut().find(|(e, _)| e.block_id == r.block_id) {
                Some(e) => *e = (r, base),
                None => p.rects.push((r, base)),
            }
        }
        let (tx, rx) = oneshot::channel();
        p.waiters.push(tx);
        let post = !p.scheduled;
        p.scheduled = true;
        (rx, post)
    }

    /// Everything pending for `window`; a later batch schedules a new task.
    fn take(&mut self, window: &str) -> Pending {
        self.0.remove(window).unwrap_or_default()
    }
}

static PENDING: LazyLock<Mutex<PendingByWindow>> = LazyLock::new(Default::default);

impl BrowserPaneManager {
    /// Move `window_label`'s panes to `rects` in one UI-thread step. Resolves
    /// when they have moved, or after `APPLY_WAIT`.
    pub async fn set_rects(
        &self,
        state: &Arc<AppState>,
        window_label: &str,
        rects: Vec<PaneBounds>,
        base: Option<LayoutBase>,
    ) {
        if rects.is_empty() {
            return;
        }
        let (applied, post) = PENDING.lock().enqueue(window_label, rects, base);
        if post {
            let mut task = ApplyPaneBoundsTask::new(state.clone(), window_label.to_string());
            if post_task(ThreadId::UI, Some(&mut task)) == 0 {
                // No UI thread to run it (shutting down): drop what's pending
                // so the window isn't left marked scheduled with no task, and
                // answer every waiter now.
                tracing::warn!(window = window_label, "[pane-bounds] post_task failed; dropping pending rects");
                for w in PENDING.lock().take(window_label).waiters {
                    let _ = w.send(());
                }
            }
        }
        let _ = tokio::time::timeout(APPLY_WAIT, applied).await;
    }
}

wrap_task! {
    pub struct ApplyPaneBoundsTask {
        state: Arc<AppState>,
        window_label: String,
    }

    impl Task {
        fn execute(&self) {
            let pending = PENDING.lock().take(&self.window_label);
            apply(&self.state, &self.window_label, &pending.rects);
            for w in pending.waiters {
                let _ = w.send(());
            }
        }
    }
}

/// Apply on the UI thread: only live panes that still belong to `window_label`.
/// Each pane's base is installed with its rect (`note_layout_base`).
fn apply(state: &Arc<AppState>, window_label: &str, rects: &[(PaneBounds, Option<LayoutBase>)]) {
    #[cfg(not(target_os = "windows"))]
    let started = std::time::Instant::now();
    let mut live: Vec<(&PaneBounds, String)> = Vec::with_capacity(rects.len());
    for (r, base) in rects {
        if state.browser_pane_window_label(&r.block_id).as_deref() != Some(window_label) {
            continue;
        }
        if let Some(label) = state.live_browser_pane_label(&r.block_id) {
            note_layout_base(&label, *base);
            live.push((r, label));
        }
    }
    if live.is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        apply_windows(state, &live);
        // A pane laid out at 0×0 is on a tab that isn't the visible one; the
        // input owner gives the app the keyboard if that pane had it.
        for (r, _) in &live {
            let collapsed = r.width <= 0 || r.height <= 0;
            crate::input_owner::windows::pane_laid_out(state, window_label, &r.block_id, collapsed);
        }
    }
    #[cfg(target_os = "macos")]
    for (r, label) in &live {
        crate::browser_pane::creation_views::resize_browser_pane_view(state, label, r.rect());
    }
    // A pane laid out at 0×0 is on a tab that isn't the visible one; the input
    // owner gives the app the keyboard if that pane had it.
    #[cfg(not(target_os = "windows"))]
    for (r, _) in &live {
        let collapsed = r.width <= 0 || r.height <= 0;
        crate::input_owner::views::pane_laid_out(state, window_label, &r.block_id, collapsed);
    }
    // Linux: move every pane, then lay the window out once, not once per pane.
    #[cfg(target_os = "linux")]
    {
        let mut moved = false;
        for (r, label) in &live {
            moved |= crate::browser_pane::creation_views::resize_browser_pane_view_inner(state, label, r.rect(), false)
                .is_some();
        }
        if moved {
            if let Some(window) = state.windows.lock().get(window_label).cloned() {
                window.layout();
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let viewport_w = rects.iter().find_map(|(_, b)| b.map(|b| b.viewport_width()));
        tracing::debug!(
            window = window_label,
            panes = live.len(),
            ms = started.elapsed().as_secs_f64() * 1000.0,
            viewport_w,
            "[pane-bounds] applied"
        );
    }
}

/// Every pane's wrapper HWND in one deferred move per parent, with the same
/// flags `wrapper::resize_wrapper` uses (`SWP_NOACTIVATE`, insert-after null =
/// top, so a moved pane is raised as before). Each wrapper's own `WM_SIZE`
/// then sizes CEF's HWND inside it.
#[cfg(target_os = "windows")]
fn apply_windows(state: &Arc<AppState>, live: &[(&PaneBounds, String)]) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetParent, SWP_NOACTIVATE,
    };

    let started = std::time::Instant::now();
    let mut by_parent: Vec<(isize, Vec<(isize, Rect)>)> = Vec::new();
    let mut moved: Vec<&str> = Vec::new();
    for (r, label) in live {
        let Some(wrapper) = crate::browser_pane::wrapper::peek_wrapper_hwnd(label) else { continue };
        let parent = unsafe { GetParent(wrapper as *mut std::ffi::c_void) } as isize;
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, v)) => v.push((wrapper, r.rect())),
            None => by_parent.push((parent, vec![(wrapper, r.rect())])),
        }
        moved.push(label);
    }

    for (_, wrappers) in &by_parent {
        let mut hdwp = unsafe { BeginDeferWindowPos(wrappers.len() as i32) };
        for (hwnd, rect) in wrappers {
            if hdwp.is_null() {
                break;
            }
            hdwp = unsafe {
                DeferWindowPos(
                    hdwp,
                    *hwnd as *mut std::ffi::c_void,
                    std::ptr::null_mut(),
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    SWP_NOACTIVATE,
                )
            };
        }
        // A failed DeferWindowPos frees the batch and returns null: move each
        // wrapper on its own instead, which is what this replaced.
        if hdwp.is_null() || unsafe { EndDeferWindowPos(hdwp) } == 0 {
            tracing::warn!(panes = wrappers.len(), "[pane-bounds] deferred move failed; moving one by one");
            for (hwnd, rect) in wrappers {
                crate::browser_pane::wrapper::resize_wrapper(*hwnd as *mut std::ffi::c_void, rect);
            }
        }
    }

    tracing::debug!(panes = moved.len(), ms = started.elapsed().as_secs_f64() * 1000.0, "[pane-bounds] moved");
    for label in moved {
        if let Some(host) = state.get_browser(label).and_then(|b| b.host()) {
            host.notify_move_or_resize_started();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(id: &str, x: i32) -> PaneBounds {
        PaneBounds { block_id: id.to_string(), x, y: 0, width: 100, height: 50 }
    }

    #[test]
    fn parses_a_batch_and_skips_entries_without_a_block_id() {
        let args = serde_json::json!({
            "window_label": "main",
            "rects": [
                { "block_id": "a", "x": 1, "y": 2, "width": 3, "height": 4 },
                { "x": 9 },
                { "block_id": "", "x": 9 },
            ],
        });
        let (window, rects) = parse_set_rects_args(&args).unwrap();
        assert_eq!(window, "main");
        assert_eq!(rects, vec![PaneBounds { block_id: "a".into(), x: 1, y: 2, width: 3, height: 4 }]);
    }

    #[test]
    fn rejects_a_batch_without_a_window_or_rects() {
        assert!(parse_set_rects_args(&serde_json::json!({ "rects": [] })).is_err());
        assert!(parse_set_rects_args(&serde_json::json!({ "window_label": "main" })).is_err());
    }

    #[test]
    fn posts_one_task_per_window_until_it_runs() {
        let mut p = PendingByWindow::default();
        let (_a, post_a) = p.enqueue("main", vec![b("x", 1)], None);
        let (_b, post_b) = p.enqueue("main", vec![b("y", 2)], None);
        let (_c, post_c) = p.enqueue("floating-1", vec![b("z", 3)], None);
        assert!(post_a);
        assert!(!post_b);
        assert!(post_c);
        p.take("main");
        let (_d, post_d) = p.enqueue("main", vec![b("x", 4)], None);
        assert!(post_d);
    }

    #[test]
    fn a_newer_rect_replaces_a_panes_pending_one_in_place() {
        let mut p = PendingByWindow::default();
        let _ = p.enqueue("main", vec![b("x", 1), b("y", 2)], None);
        let _ = p.enqueue("main", vec![b("x", 9)], None);
        let taken = p.take("main");
        assert_eq!(taken.rects, vec![(b("x", 9), None), (b("y", 2), None)]);
        assert_eq!(taken.waiters.len(), 2);
    }

    fn t(r: Rect) -> (i32, i32, i32, i32) {
        (r.x, r.y, r.width, r.height)
    }

    fn base() -> LayoutBase {
        // A 1000x800 page whose tile layout leaves a 70 px top bar and a 50 px status bar.
        LayoutBase::parse(&serde_json::json!({
            "viewport": { "width": 1000, "height": 800 },
            "layout": { "x": 0, "y": 70, "width": 1000, "height": 680 },
        }))
        .unwrap()
    }

    #[test]
    fn the_same_size_keeps_a_rect() {
        let r = Rect { x: 500, y: 100, width: 500, height: 300 };
        assert_eq!(t(base().predict(&r, 1000, 800)), t(r));
    }

    #[test]
    fn a_right_edge_pane_follows_the_window_and_scales_with_the_layout() {
        let r = Rect { x: 500, y: 100, width: 500, height: 300 };
        assert_eq!(t(base().predict(&r, 800, 800)), (400, 100, 400, 300));
    }

    #[test]
    fn the_top_and_bottom_bars_keep_their_size() {
        // The pane fills the layout; 136 px less height all comes out of it.
        let r = Rect { x: 0, y: 70, width: 1000, height: 680 };
        assert_eq!(t(base().predict(&r, 1000, 664)), (0, 70, 1000, 544));
    }

    #[test]
    fn a_batch_without_a_layout_scales_with_the_viewport() {
        let b = LayoutBase::parse(&serde_json::json!({ "viewport": { "width": 1000, "height": 800 } })).unwrap();
        let r = Rect { x: 500, y: 0, width: 500, height: 800 };
        assert_eq!(t(b.predict(&r, 1200, 800)), (600, 0, 600, 800));
        assert!(LayoutBase::parse(&serde_json::json!({})).is_none());
    }

    #[test]
    fn each_pending_rect_keeps_its_own_batch_s_layout_base() {
        let mut p = PendingByWindow::default();
        let newer = LayoutBase::parse(&serde_json::json!({ "viewport": { "width": 900, "height": 800 } }));
        let _ = p.enqueue("main", vec![b("x", 1), b("y", 1)], Some(base()));
        let _ = p.enqueue("main", vec![b("y", 2)], newer);
        let _ = p.enqueue("main", vec![b("z", 3)], None);
        assert_eq!(
            p.take("main").rects,
            vec![(b("x", 1), Some(base())), (b("y", 2), newer), (b("z", 3), None)]
        );
    }

    #[test]
    fn taking_wakes_every_waiter() {
        let mut p = PendingByWindow::default();
        let (mut rx1, _) = p.enqueue("main", vec![b("x", 1)], None);
        let (mut rx2, _) = p.enqueue("main", vec![b("y", 2)], None);
        for w in p.take("main").waiters {
            w.send(()).unwrap();
        }
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }
}
