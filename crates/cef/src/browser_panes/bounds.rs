// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_panes_set_rects`: move a window's panes in one step.
//!
//! The frontend used to send one `browser_pane_resize` per pane per frame, and
//! each was applied on its own by a cross-thread `SetWindowPos` from the IPC
//! thread that waited for the UI thread. A window drag over nine panes queued
//! up to 111 requests, so panes trailed the page by about half a second
//! (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md). Now the frontend
//! sends one batch per window, at most 3 out at once and numbered (`RectsOrder`)
//! (`frontend/app/view/browser/pane-rect-batcher.ts`), and here:
//!
//! - A pane's rect from an older batch than one already received is dropped.
//! - A batch merges into the window's pending rects (a pane's newer rect
//!   replaces its older one) and one UI task is scheduled for the window if
//!   none is waiting. Batches can't pile up on the UI thread.
//! - That task takes everything pending and applies it: on Windows every
//!   pane's wrapper moves in one `BeginDeferWindowPos`/`EndDeferWindowPos`
//!   per parent window, so they move together; elsewhere each pane goes
//!   through its usual Views resize, all in the one task.
//! - The IPC call returns once its rects have been applied, or after
//!   `APPLY_WAIT` if the UI thread is busy, so the page's next batch follows
//!   the host's pace without ever waiting on a stuck UI thread. A batch that
//!   came over the process-message lane (`ipc_query.rs`) is answered by the
//!   UI task that applies it: its reply reaches the page through the UI
//!   thread anyway, so a cap would gain it nothing.
//! - A pane is moved only while it is live and still in the window that sent
//!   the batch: a rect from a window a pane was torn off from is dropped.
//! - The host also moves shown panes itself when their window resizes, ahead
//!   of the page (`LayoutBase`): macOS `follow_window_resize`, Linux
//!   `window_resized`. Not Windows Views panes: they wait for the page's rects
//!   (`window_resized` says why). Windows child-window panes do the same wait:
//!   `follow_window_resize` (from the window's `WM_SIZE`) holds each at the
//!   page's last rect, inside the window, until the page lays out for the new
//!   size (`HOLD_MAX` bounds the wait), so a view never floats off its frame.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::Duration;

use cef::wrapper::message_router::BrowserSideCallback;
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
pub(crate) struct LayoutBase {
    viewport: (i32, i32),
    /// x, y, width, height of the tile layout; the whole viewport if unknown.
    layout: (i32, i32, i32, i32),
}

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
    #[cfg_attr(target_os = "windows", allow(dead_code))]
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
static LAYOUT_BASES: LazyLock<Mutex<HashMap<String, LayoutBase>>> = LazyLock::new(Default::default);

/// Remember what pane `label`'s rect was laid out against (`None`: a rect
/// with no base, which is used as sent). UI thread, as the rect is applied.
pub(crate) fn note_layout_base(label: &str, base: Option<LayoutBase>) {
    let mut bases = LAYOUT_BASES.lock();
    match base {
        Some(b) => bases.insert(label.to_string(), b),
        None => bases.remove(label),
    };
}

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
///
/// Windows Views panes are not moved this way. There the window's page can be
/// several frames behind the window in a fast resize (90 to 110 ms a frame
/// when it was measured), longer than the window's compositor waits for it
/// (`pane_backend::append_surface_deadline`). A pane placed for the new size
/// then showed outside the border the page was still drawing at the old one.
/// Moved only by the page's own rects, a pane and its border come from the
/// same layout.
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
        crate::browser_pane::creation_views::place_overlay(&controller, Some(window), &to);
        // An overlay's clip is not adjusted when the overlay is resized: with a menu over the pane
        // its cut would be of the old size until the page's next rects (#7063).
        let scale = crate::browser_pane::creation_views::views_scale(Some(window));
        super::views_clip::apply_placed(state, &label, window_label, &controller, (x, y, w, h), (to.width, to.height), scale);
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

/// How long after a window's last size change the page's rects count as
/// settled: they were laid out for the size the window has now.
#[cfg(target_os = "windows")]
const SETTLE: Duration = Duration::from_millis(250);

/// A page's viewport is `innerWidth * devicePixelRatio` rounded, so at a
/// fractional display scale it can differ from the window's client area by a
/// pixel or two with nothing resizing. That is noise, not a stale layout.
#[cfg(any(target_os = "windows", test))]
const ROUNDING_PX: i32 = 2;

/// The most the client area can exceed the page's viewport by with the page
/// caught up (rounding at a fractional scale). A page further off than this
/// is behind the window, however long ago the window last changed.
#[cfg(any(target_os = "windows", test))]
const MAX_OFFSET_PX: i32 = 8;

/// The client area minus the viewport, if a rect laid out against `viewport`
/// shows the page caught up with a `client` that has been still (`settled`).
#[cfg(any(target_os = "windows", test))]
fn learned_offset(settled: bool, viewport: (i32, i32), client: (i32, i32)) -> Option<(i32, i32)> {
    let offset = (client.0 - viewport.0, client.1 - viewport.1);
    (settled && offset.0.abs() <= MAX_OFFSET_PX && offset.1.abs() <= MAX_OFFSET_PX).then_some(offset)
}

/// Where the page's `rect` goes while the window's client area is `client`.
/// `offset` is what the client area exceeds the page's viewport by when the
/// two agree (`WindowSize`).
#[cfg(any(target_os = "windows", test))]
fn follow_rect(base: &LayoutBase, rect: &Rect, client: (i32, i32), offset: (i32, i32)) -> Rect {
    let (width, height) = (client.0 - offset.0, client.1 - offset.1);
    if (width - base.viewport.0).abs() <= ROUNDING_PX && (height - base.viewport.1).abs() <= ROUNDING_PX {
        return Rect { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    }
    base.predict(rect, width, height)
}

/// How long a window's panes are held at the page's last rects while the page
/// sends nothing, after the window changes size. A page that is stuck or gone
/// must not leave the panes frozen: they are stretched to the window as the
/// Linux and macOS hosts do. A busy page answers in 100 to 400 ms; this is well
/// past that.
#[cfg(any(target_os = "windows", test))]
const HOLD_MAX: Duration = Duration::from_millis(1000);

/// One window's hold on its panes (Windows): from the window's size changing
/// until the page has laid out for it.
///
/// The wait is for the page to go quiet, not for the first change to age: a page
/// that keeps sending rects, even for sizes the window has left (a long drag),
/// is alive and is followed through its rects for as long as the drag lasts.
/// Once it has been silent for `HOLD_MAX` the window is `stretching`, and stays so
/// until the page has caught up with the window (`caught_up`): the panes are
/// stretched on every size change, and the page's late rects for older sizes are
/// stretched too, so a pane never flips between the two.
#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Default)]
struct Hold {
    /// When the window's size first changed with the page not yet caught up.
    unanswered_since: Option<std::time::Instant>,
    /// When the page's rects for the window were last applied.
    last_rect: Option<std::time::Instant>,
    /// The page has been silent for `HOLD_MAX`: panes are stretched, not held.
    stretching: bool,
}

#[cfg(any(target_os = "windows", test))]
impl Hold {
    /// The window's size changed at `now`. Whether panes are stretched now.
    fn size_changed(&mut self, now: std::time::Instant) -> bool {
        self.unanswered_since.get_or_insert(now);
        self.check(now)
    }

    /// The page's rects for the window are applied at `now`. Whether panes are
    /// stretched now (decided before this rect, which only keeps the page alive).
    fn rect_applied(&mut self, now: std::time::Instant) -> bool {
        let stretching = self.check(now);
        self.last_rect = Some(now);
        stretching
    }

    /// The page has laid out for the window's size: the next change starts a new wait.
    fn caught_up(&mut self) {
        self.unanswered_since = None;
        self.stretching = false;
    }

    /// How long until the page counts as silent, if a size change is unanswered
    /// and the panes are still held.
    fn left(&self, now: std::time::Instant) -> Option<Duration> {
        let since = self.unanswered_since.filter(|_| !self.stretching)?;
        let last = self.last_rect.filter(|t| *t > since).unwrap_or(since);
        Some(HOLD_MAX.saturating_sub(now.saturating_duration_since(last)))
    }

    /// Start stretching if the page has been silent for `HOLD_MAX`. Whether stretching.
    fn check(&mut self, now: std::time::Instant) -> bool {
        if self.left(now).is_some_and(|left| left.is_zero()) {
            self.stretching = true;
        }
        self.stretching
    }
}

/// Whether a rect laid out against `viewport` is for the window's size now
/// (`client`), within the rounding of a fractional display scale: the page has
/// answered the window's last size change.
#[cfg(any(target_os = "windows", test))]
fn page_caught_up(viewport: (i32, i32), client: (i32, i32)) -> bool {
    (client.0 - viewport.0).abs() <= MAX_OFFSET_PX && (client.1 - viewport.1).abs() <= MAX_OFFSET_PX
}

/// `rect` cut to the window's client area `(0, 0, client)`: a pane is never
/// drawn outside the window, which is what a pane held at its old rect while
/// the window shrinks would be. A rect wholly outside becomes empty, at the
/// nearest edge.
#[cfg(any(target_os = "windows", test))]
fn clip_to_client(rect: &Rect, client: (i32, i32)) -> Rect {
    let left = rect.x.clamp(0, client.0);
    let top = rect.y.clamp(0, client.1);
    let right = (rect.x + rect.width).clamp(left, client.0);
    let bottom = (rect.y + rect.height).clamp(top, client.1);
    Rect { x: left, y: top, width: right - left, height: bottom - top }
}

/// Where each of `window_label`'s shown panes goes while the window's client
/// area is `client` and the page has not laid out for it: its last rect, clipped
/// to the window, by label. A grown window leaves every rect as it is. A pane
/// with no page rect yet is not in `rects` and is not planned.
#[cfg(any(target_os = "windows", test))]
fn plan_hold(rects: &PageRects, window_label: &str, client: (i32, i32)) -> Vec<(String, Rect)> {
    let mut plan: Vec<(String, Rect)> = rects
        .iter()
        .filter(|(_, (w, (_, _, width, height)))| w == window_label && *width > 0 && *height > 0)
        .map(|(label, (_, (x, y, width, height)))| {
            (label.clone(), clip_to_client(&Rect { x: *x, y: *y, width: *width, height: *height }, client))
        })
        .collect();
    plan.sort_by(|a, b| a.0.cmp(&b.0));
    plan
}

/// Each pane's rect as the page last sent it (Windows), by pane label, with the
/// window it sits in: what `follow_window_resize` stretches. macOS and Linux
/// read `browser_pane_physical_rects`, which is `cfg(not(windows))`.
#[cfg(any(target_os = "windows", test))]
type PageRects = HashMap<String, (String, (i32, i32, i32, i32))>;

#[cfg(target_os = "windows")]
static PAGE_RECTS: LazyLock<Mutex<PageRects>> = LazyLock::new(Default::default);

/// Where each of `window_label`'s shown panes that has a layout base goes in a
/// client area of `client`, by label.
#[cfg(any(target_os = "windows", test))]
fn plan_follow(
    rects: &PageRects,
    window_label: &str,
    base_of: impl Fn(&str) -> Option<LayoutBase>,
    client: (i32, i32),
    offset: (i32, i32),
) -> Vec<(String, Rect)> {
    let mut plan: Vec<(String, Rect)> = rects
        .iter()
        .filter(|(_, (w, (_, _, width, height)))| w == window_label && *width > 0 && *height > 0)
        .filter_map(|(label, (_, (x, y, width, height)))| {
            let base = base_of(label)?;
            let rect = Rect { x: *x, y: *y, width: *width, height: *height };
            Some((label.clone(), follow_rect(&base, &rect, client, offset)))
        })
        .collect();
    plan.sort_by(|a, b| a.0.cmp(&b.0));
    plan
}

/// What the host knows about a window's size (Windows), by window label.
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Default)]
struct WindowSize {
    /// When the window's size last changed.
    changed: Option<std::time::Instant>,
    /// The client area minus the page's viewport, as of the last time the page
    /// had caught up with the window.
    offset: (i32, i32),
    /// Whether its panes are held at the page's rects or stretched (`Hold`).
    hold: Hold,
    /// A `HoldTask` is waiting to stretch the panes if the page goes silent.
    timer_armed: bool,
}

#[cfg(target_os = "windows")]
static WINDOW_SIZES: LazyLock<Mutex<HashMap<String, WindowSize>>> = LazyLock::new(Default::default);

/// A pane wrappers' parent window's client area in physical pixels, if it has one.
#[cfg(target_os = "windows")]
fn client_size(parent: isize) -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { GetClientRect(parent as *mut std::ffi::c_void, &mut r) };
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    (ok != 0 && w > 0 && h > 0).then_some((w, h))
}

/// The window `window_label` changed size. Its shown panes stay where the page
/// last put them, held inside the window (`clip_to_client`), so each view keeps
/// agreeing with its frame while the page lays out for the new size; the page's
/// own rects then move them (`apply`). A page that has been silent for
/// `HOLD_MAX` (`Hold`) is stretched past instead, as the Linux and macOS hosts do
/// from the first change: a `HoldTask` makes sure that happens even when the
/// window has stopped changing. UI thread only (the window's `WM_SIZE`, after
/// CEF has handled it).
///
/// Only the panes that have a page rect are held: a pane with no rect yet has
/// nowhere to be held at and is not touched. Floating panes and popups are
/// windows of their own and not part of this.
#[cfg(target_os = "windows")]
pub(crate) fn follow_window_resize(window_label: &str) {
    let now = std::time::Instant::now();
    let (offset, stretch, arm) = {
        let mut sizes = WINDOW_SIZES.lock();
        let size = sizes.entry(window_label.to_string()).or_default();
        size.changed = Some(now);
        let stretch = size.hold.size_changed(now);
        let arm = !std::mem::replace(&mut size.timer_armed, true);
        (size.offset, stretch, arm)
    };
    if arm {
        hold_task::arm(window_label, HOLD_MAX);
    }
    move_held_panes(window_label, offset, stretch);
}

/// A `HoldTask` ran: stretch the window's panes if the page has been silent for
/// `HOLD_MAX` since the last size change or rect, else wait for what is left.
#[cfg(target_os = "windows")]
fn hold_timeout(window_label: &str) {
    let now = std::time::Instant::now();
    let (offset, wait) = {
        let mut sizes = WINDOW_SIZES.lock();
        let Some(size) = sizes.get_mut(window_label) else { return };
        size.timer_armed = false;
        // `None`: the page has answered, or the panes are already stretched.
        let Some(left) = size.hold.left(now) else { return };
        if !left.is_zero() {
            size.timer_armed = true;
            (size.offset, Some(left))
        } else {
            size.hold.check(now);
            (size.offset, None)
        }
    };
    match wait {
        Some(left) => hold_task::arm(window_label, left),
        None => move_held_panes(window_label, offset, true),
    }
}

/// Move the shown panes of `window_label` that have a page rect: to that rect
/// clipped to the window, or with `stretch` to where the page will put it.
#[cfg(target_os = "windows")]
fn move_held_panes(window_label: &str, offset: (i32, i32), stretch: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetParent, SWP_NOACTIVATE, SWP_NOZORDER};

    let started = std::time::Instant::now();
    let rects: PageRects =
        PAGE_RECTS.lock().iter().filter(|(_, (w, _))| w == window_label).map(|(l, v)| (l.clone(), v.clone())).collect();
    let mut by_parent: Vec<(isize, Vec<(isize, Rect)>)> = Vec::new();
    let mut moved = 0;
    // One pane at a time: each wrapper has its own parent to measure.
    for label in rects.keys() {
        // A pane whose wrapper is gone (closed, hibernated) is forgotten here.
        let Some(wrapper) = crate::browser_pane::wrapper::peek_wrapper_hwnd(label) else {
            PAGE_RECTS.lock().remove(label);
            continue;
        };
        let parent = unsafe { GetParent(wrapper as *mut std::ffi::c_void) } as isize;
        let Some(client) = client_size(parent) else { continue };
        let one: PageRects = rects.iter().filter(|(l, _)| *l == label).map(|(l, v)| (l.clone(), v.clone())).collect();
        let planned = if stretch {
            plan_follow(&one, window_label, layout_base, client, offset)
        } else {
            plan_hold(&one, window_label, client)
        };
        let Some((_, to)) = planned.into_iter().next() else { continue };
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, v)) => v.push((wrapper, to)),
            None => by_parent.push((parent, vec![(wrapper, to)])),
        }
        moved += 1;
    }
    for (_, wrappers) in &by_parent {
        defer_move(wrappers, SWP_NOACTIVATE | SWP_NOZORDER);
    }
    if moved > 0 {
        tracing::debug!(
            window = window_label,
            panes = moved,
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "[pane-bounds] followed"
        );
    }
}

/// Which batch of a page's rects a request is: the page's start and a count
/// within its life. The page sends a second batch before the host has answered
/// the first, and two requests can arrive in either order; a pane's rect from
/// an older batch must not replace the newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RectsOrder {
    epoch: u64,
    seq: u64,
}

impl RectsOrder {
    /// `{ epoch, seq }` from a batch; `None` from a page that doesn't send them.
    pub(crate) fn parse(args: &serde_json::Value) -> Option<Self> {
        Some(Self { epoch: args.get("epoch")?.as_u64()?, seq: args.get("seq")?.as_u64()? })
    }
}

/// The newest batch each pane in each window has had a rect from, by window
/// and block. Per window: a pane torn off into a window whose page started
/// earlier is a new sequence, not a stale one.
type NewestBatch = HashMap<(String, String), RectsOrder>;

static NEWEST_BATCH: LazyLock<Mutex<NewestBatch>> = LazyLock::new(Default::default);

/// `rects` without the panes a newer batch has already sent a rect for.
fn drop_superseded(
    newest: &mut NewestBatch,
    window: &str,
    rects: Vec<PaneBounds>,
    order: Option<RectsOrder>,
) -> Vec<PaneBounds> {
    let Some(order) = order else { return rects };
    rects
        .into_iter()
        .filter(|r| {
            let seen = newest.entry((window.to_string(), r.block_id.clone())).or_insert(order);
            if order < *seen {
                return false;
            }
            *seen = order;
            true
        })
        .collect()
}

/// Who waits for a batch to be applied: the HTTP call (`set_rects`), or the
/// page's query over the process-message lane (`ipc_query.rs`).
pub(crate) enum Waiter {
    Channel(oneshot::Sender<()>),
    Query(Arc<StdMutex<dyn BrowserSideCallback>>),
}

impl Waiter {
    /// The batch is applied (or dropped): tell whoever waits.
    fn answer(self) {
        match self {
            Waiter::Channel(tx) => {
                let _ = tx.send(());
            }
            Waiter::Query(callback) => {
                if let Ok(cb) = callback.lock() {
                    cb.success_str("true");
                }
            }
        }
    }
}

/// A window's rects waiting for the UI thread, and who is waiting on them.
#[derive(Default)]
struct Pending {
    /// In first-arrival order; a newer rect for a pane replaces its entry in place.
    /// Each with what its batch was laid out against; installed with the
    /// rect, so a window resize never reads a rect against another's base.
    rects: Vec<(PaneBounds, Option<LayoutBase>)>,
    waiters: Vec<Waiter>,
    /// A UI task for this window is posted and hasn't run yet.
    scheduled: bool,
}

#[derive(Default)]
struct PendingByWindow(HashMap<String, Pending>);

impl PendingByWindow {
    /// Merge `rects` into `window`'s pending set, with who waits for them.
    /// Returns whether the caller must post a task.
    fn enqueue(&mut self, window: &str, rects: Vec<PaneBounds>, base: Option<LayoutBase>, waiter: Waiter) -> bool {
        let p = self.0.entry(window.to_string()).or_default();
        for r in rects {
            match p.rects.iter_mut().find(|(e, _)| e.block_id == r.block_id) {
                Some(e) => *e = (r, base),
                None => p.rects.push((r, base)),
            }
        }
        p.waiters.push(waiter);
        let post = !p.scheduled;
        p.scheduled = true;
        post
    }

    /// Everything pending for `window`; a later batch schedules a new task.
    fn take(&mut self, window: &str) -> Pending {
        self.0.remove(window).unwrap_or_default()
    }
}

static PENDING: LazyLock<Mutex<PendingByWindow>> = LazyLock::new(Default::default);

/// Queue `window_label`'s batch for the UI thread and make sure a task will
/// apply it; `waiter` is answered when it has been, or at once when a newer
/// batch has already replaced every rect in it.
pub(crate) fn queue(
    state: &Arc<AppState>,
    window_label: &str,
    rects: Vec<PaneBounds>,
    base: Option<LayoutBase>,
    order: Option<RectsOrder>,
    waiter: Waiter,
) {
    // The order check and the merge into the pending rects are one step under PENDING: a
    // batch that is checked first is merged first, so an older one can't overwrite a newer
    // rect between the two. (Always PENDING, then NEWEST_BATCH.)
    let post = {
        let mut pending = PENDING.lock();
        let rects = drop_superseded(&mut NEWEST_BATCH.lock(), window_label, rects, order);
        if rects.is_empty() {
            drop(pending);
            waiter.answer();
            return;
        }
        pending.enqueue(window_label, rects, base, waiter)
    };
    if post {
        let mut task = ApplyPaneBoundsTask::new(state.clone(), window_label.to_string());
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            // No UI thread to run it (shutting down): drop what's pending
            // so the window isn't left marked scheduled with no task, and
            // answer every waiter now.
            tracing::warn!(window = window_label, "[pane-bounds] post_task failed; dropping pending rects");
            for w in PENDING.lock().take(window_label).waiters {
                w.answer();
            }
        }
    }
}

impl BrowserPaneManager {
    /// Move `window_label`'s panes to `rects` in one UI-thread step. Resolves
    /// when they have moved, or after `APPLY_WAIT`.
    pub async fn set_rects(
        &self,
        state: &Arc<AppState>,
        window_label: &str,
        rects: Vec<PaneBounds>,
        base: Option<LayoutBase>,
        order: Option<RectsOrder>,
    ) {
        let (tx, applied) = oneshot::channel();
        queue(state, window_label, rects, base, order, Waiter::Channel(tx));
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
                w.answer();
            }
        }
    }
}

/// Apply on the UI thread: only live panes that still belong to `window_label`.
/// Each pane's base is installed with its rect (`note_layout_base`).
fn apply(state: &Arc<AppState>, window_label: &str, rects: &[(PaneBounds, Option<LayoutBase>)]) {
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
    let views = crate::pane_backend::window_is_views(state, window_label);
    #[cfg(target_os = "windows")]
    if !views {
        apply_windows(state, window_label, &live);
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
    // owner gives the app the keyboard if that pane had it. macOS reports a
    // shown pane later, once its overlay window is on screen and can take key
    // status (SetPaneBoundsViewsTask, which this resize posts).
    if views {
        for (r, _) in &live {
            let collapsed = r.width <= 0 || r.height <= 0;
            if collapsed || cfg!(not(target_os = "macos")) {
                crate::input_owner::views::pane_laid_out(state, window_label, &r.block_id, collapsed);
            }
        }
    }
    // Linux: move every pane, then lay the window out once, not once per pane.
    // Windows Views panes need no layout: setting an overlay's bounds moves it.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if views {
        let mut moved = false;
        for (r, label) in &live {
            moved |= crate::browser_pane::creation_views::resize_browser_pane_view_inner(state, label, r.rect(), false)
                .is_some();
        }
        if moved && cfg!(target_os = "linux") {
            if let Some(window) = state.windows.lock().get(window_label).cloned() {
                window.layout();
            }
        }
    }
    note_hidden(state, &live);
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

/// The browsers this host has told CEF are hidden, by browser id (a pane's
/// browser recreated after hibernation has a new id, so it starts shown).
static HIDDEN: LazyLock<Mutex<HashSet<i32>>> = LazyLock::new(Default::default);

/// A pane laid out at 0×0 is on a tab or window tab that isn't shown: tell CEF
/// it is hidden, so its page gets hidden-page timer throttling and stops
/// producing frames, and shown again when it gets a real rect. A collapsed
/// pane's native window is still shown, so without this Chromium keeps the
/// page visible. Needs the AgentMux CEF fork's windowed `WasHidden`; an older
/// libcef ignores the call. Every path that shows a pane lays it out here, the
/// moves above first, so the first frame after showing has a place to land.
fn note_hidden(state: &AppState, live: &[(&PaneBounds, String)]) {
    let mut hidden = HIDDEN.lock();
    for (r, label) in live {
        let Some(browser) = state.get_browser(label) else { continue };
        let collapsed = r.width <= 0 || r.height <= 0;
        let id = browser.identifier();
        let changed = if collapsed { hidden.insert(id) } else { hidden.remove(&id) };
        if !changed {
            continue;
        }
        if let Some(host) = browser.host() {
            host.was_hidden(collapsed as i32);
        }
    }
}

/// A browser closed (hibernation closes hidden panes): drop its entry.
pub(crate) fn forget_hidden(browser_id: i32) {
    HIDDEN.lock().remove(&browser_id);
}

/// Every pane's wrapper HWND in one deferred move per parent, with the same
/// flags `wrapper::resize_wrapper` uses (`SWP_NOACTIVATE`, insert-after null =
/// top, so a moved pane is raised as before). Each wrapper's own `WM_SIZE`
/// then sizes CEF's HWND inside it.
///
/// The page's rect is placed as sent, clipped to the window (`clip_to_client`):
/// it is the layout the page's frame is drawn in, so a view and its frame agree
/// whatever size the rect was laid out for. It is also kept for the next resize
/// (`follow_window_resize`). A rect for the window's size now ends the hold. Once
/// the page has been silent for `HOLD_MAX` (`Hold::stretching`) its late rects
/// for older sizes are stretched to the window instead, as the panes are on each
/// size change, until the page has caught up.
#[cfg(target_os = "windows")]
fn apply_windows(state: &Arc<AppState>, window_label: &str, live: &[(&PaneBounds, String)]) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetParent, SWP_NOACTIVATE};

    let started = std::time::Instant::now();
    let mut by_parent: Vec<(isize, Vec<(isize, Rect)>)> = Vec::new();
    let mut moved: Vec<&str> = Vec::new();
    {
        let mut kept = PAGE_RECTS.lock();
        for (r, label) in live {
            kept.insert(label.clone(), (window_label.to_string(), (r.x, r.y, r.width, r.height)));
        }
    }
    // The page is alive (it keeps the hold from timing out). Whether its silence had already
    // ended the hold: then its late rects for older sizes are stretched too, as before, so a
    // pane does not flip between the page's rect and the stretched one.
    let stretching = WINDOW_SIZES.lock().entry(window_label.to_string()).or_default().hold.rect_applied(started);
    let mut caught_up = false;
    for (r, label) in live {
        let Some(wrapper) = crate::browser_pane::wrapper::peek_wrapper_hwnd(label) else { continue };
        let parent = unsafe { GetParent(wrapper as *mut std::ffi::c_void) } as isize;
        let rect = match (r.width > 0 && r.height > 0, client_size(parent)) {
            (true, Some(client)) => {
                let base = layout_base(label);
                let offset = base.map(|b| window_offset(window_label, &b, client));
                if let Some(b) = base {
                    caught_up |= page_caught_up(b.viewport, client);
                }
                match (stretching, base, offset) {
                    (true, Some(b), Some(o)) => follow_rect(&b, &r.rect(), client, o),
                    // The page's rect is its own layout, which its frame is drawn in: placed as
                    // sent, and never past the window's edge.
                    _ => clip_to_client(&r.rect(), client),
                }
            }
            _ => r.rect(),
        };
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, v)) => v.push((wrapper, rect)),
            None => by_parent.push((parent, vec![(wrapper, rect)])),
        }
        moved.push(label);
    }
    if caught_up {
        // The page has laid out for the window's size: the next size change starts a new wait.
        if let Some(size) = WINDOW_SIZES.lock().get_mut(window_label) {
            size.hold.caught_up();
        }
    }

    for (_, wrappers) in &by_parent {
        defer_move(wrappers, SWP_NOACTIVATE);
    }

    tracing::debug!(panes = moved.len(), ms = started.elapsed().as_secs_f64() * 1000.0, "[pane-bounds] moved");
    for label in moved {
        if let Some(host) = state.get_browser(label).and_then(|b| b.host()) {
            host.notify_move_or_resize_started();
        }
    }
}

/// The offset between `window_label`'s client area and its page's viewport. It
/// is learned from a rect laid out for a size near the client area's, once the
/// window hasn't changed size for `SETTLE`. A busy page's late rect for an
/// older size is far from the client area and teaches nothing.
#[cfg(target_os = "windows")]
fn window_offset(window_label: &str, base: &LayoutBase, client: (i32, i32)) -> (i32, i32) {
    let mut sizes = WINDOW_SIZES.lock();
    let size = sizes.entry(window_label.to_string()).or_default();
    let settled = size.changed.is_none_or(|t| t.elapsed() > SETTLE);
    if let Some(offset) = learned_offset(settled, base.viewport, client) {
        size.offset = offset;
    }
    size.offset
}

/// Move wrappers of one parent together with one `DeferWindowPos` batch.
#[cfg(target_os = "windows")]
fn defer_move(wrappers: &[(isize, Rect)], flags: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos};

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
                flags,
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

/// The UI-thread task that ends a hold nobody answered (`hold_timeout`).
#[cfg(target_os = "windows")]
mod hold_task {
    use std::time::Duration;

    use cef::*;

    wrap_task! {
        struct HoldTask {
            label: String,
        }

        impl Task {
            fn execute(&self) {
                super::hold_timeout(&self.label);
            }
        }
    }

    /// Run `hold_timeout` for `label` in `after` (at least 1 ms).
    pub(super) fn arm(label: &str, after: Duration) {
        let mut task = HoldTask::new(label.to_string());
        post_delayed_task(ThreadId::UI, Some(&mut task), after.as_millis().max(1) as i64);
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

    /// A waiter nobody listens to.
    fn w() -> Waiter {
        Waiter::Channel(oneshot::channel().0)
    }

    #[test]
    fn posts_one_task_per_window_until_it_runs() {
        let mut p = PendingByWindow::default();
        let post_a = p.enqueue("main", vec![b("x", 1)], None, w());
        let post_b = p.enqueue("main", vec![b("y", 2)], None, w());
        let post_c = p.enqueue("floating-1", vec![b("z", 3)], None, w());
        assert!(post_a);
        assert!(!post_b);
        assert!(post_c);
        p.take("main");
        let post_d = p.enqueue("main", vec![b("x", 4)], None, w());
        assert!(post_d);
    }

    #[test]
    fn a_newer_rect_replaces_a_panes_pending_one_in_place() {
        let mut p = PendingByWindow::default();
        p.enqueue("main", vec![b("x", 1), b("y", 2)], None, w());
        p.enqueue("main", vec![b("x", 9)], None, w());
        let taken = p.take("main");
        assert_eq!(taken.rects, vec![(b("x", 9), None), (b("y", 2), None)]);
        assert_eq!(taken.waiters.len(), 2);
    }

    #[test]
    fn answering_a_channel_waiter_fires_its_receiver() {
        let (tx, mut rx) = oneshot::channel();
        Waiter::Channel(tx).answer();
        assert_eq!(rx.try_recv(), Ok(()));
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
        p.enqueue("main", vec![b("x", 1), b("y", 1)], Some(base()), w());
        p.enqueue("main", vec![b("y", 2)], newer, w());
        p.enqueue("main", vec![b("z", 3)], None, w());
        assert_eq!(
            p.take("main").rects,
            vec![(b("x", 1), Some(base())), (b("y", 2), newer), (b("z", 3), None)]
        );
    }

    fn page_rects() -> PageRects {
        let mut m = PageRects::new();
        m.insert("right".into(), ("main".into(), (500, 100, 500, 300)));
        m.insert("left".into(), ("main".into(), (0, 100, 500, 300)));
        m.insert("hidden".into(), ("main".into(), (0, 0, 0, 0)));
        m.insert("elsewhere".into(), ("floating-1".into(), (500, 100, 500, 300)));
        m
    }

    fn plan(client: (i32, i32), offset: (i32, i32)) -> Vec<(String, (i32, i32, i32, i32))> {
        plan_follow(&page_rects(), "main", |_| Some(base()), client, offset)
            .into_iter()
            .map(|(l, r)| (l, t(r)))
            .collect()
    }

    #[test]
    fn a_window_resize_moves_its_shown_panes_and_leaves_the_rest() {
        assert_eq!(
            plan((800, 800), (0, 0)),
            vec![("left".to_string(), (0, 100, 400, 300)), ("right".to_string(), (400, 100, 400, 300))]
        );
    }

    #[test]
    fn a_pane_without_a_layout_base_is_left_where_it_is() {
        let planned = plan_follow(&page_rects(), "main", |l| (l == "left").then(base), (800, 800), (0, 0));
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].0, "left");
    }

    #[test]
    fn the_pages_own_rounding_is_not_a_resize() {
        // At a fractional display scale the viewport is a pixel or two off the client area.
        let r = Rect { x: 500, y: 100, width: 500, height: 300 };
        let same = t(Rect { x: r.x, y: r.y, width: r.width, height: r.height });
        assert_eq!(t(follow_rect(&base(), &r, (1001, 799), (0, 0))), same);
        // And a client area that is always 3 px wider than the page's viewport is the offset.
        assert_eq!(t(follow_rect(&base(), &r, (1003, 800), (3, 0))), same);
        assert_eq!(t(follow_rect(&base(), &r, (803, 800), (3, 0))), (400, 100, 400, 300));
    }

    fn hold(client: (i32, i32)) -> Vec<(String, (i32, i32, i32, i32))> {
        plan_hold(&page_rects(), "main", client).into_iter().map(|(l, r)| (l, t(r))).collect()
    }

    #[test]
    fn a_shrinking_window_holds_every_pane_at_the_pages_rect_not_stretched() {
        // The window is 40 px narrower than the page's layout: the left pane is untouched, and the
        // right one (500..1000) is cut at the window's edge, not moved or scaled with it.
        assert_eq!(
            hold((960, 800)),
            vec![("left".to_string(), (0, 100, 500, 300)), ("right".to_string(), (500, 100, 460, 300))]
        );
    }

    #[test]
    fn a_pane_is_never_drawn_outside_the_window() {
        let rects = [
            (500, 100, 500, 300),
            (-20, -10, 300, 200),
            (900, 700, 400, 400),
            (0, 0, 5000, 5000),
            (3000, 3000, 10, 10),
            (100, 100, 1, 1),
        ];
        for client in [(960, 800), (300, 200), (1, 1), (2000, 1500), (0, 0)] {
            for (x, y, w, h) in rects {
                let r = clip_to_client(&Rect { x, y, width: w, height: h }, client);
                let got = t(r);
                assert!(got.0 >= 0 && got.1 >= 0, "{got:?} in {client:?}");
                assert!(got.2 >= 0 && got.3 >= 0, "{got:?} in {client:?}");
                assert!(got.0 + got.2 <= client.0 && got.1 + got.3 <= client.1, "{got:?} in {client:?}");
                // A rect that was inside the window is not changed.
                if x >= 0 && y >= 0 && x + w <= client.0 && y + h <= client.1 {
                    assert_eq!(got, (x, y, w, h));
                }
            }
        }
    }

    #[test]
    fn a_pane_the_window_has_left_behind_is_empty_at_its_edge() {
        let r = clip_to_client(&Rect { x: 900, y: 100, width: 100, height: 300 }, (800, 800));
        assert_eq!(t(r), (800, 100, 0, 300));
    }

    #[test]
    fn a_growing_window_keeps_each_pane_at_its_old_size_until_the_page_lays_out() {
        assert_eq!(
            hold((1400, 1000)),
            vec![("left".to_string(), (0, 100, 500, 300)), ("right".to_string(), (500, 100, 500, 300))]
        );
    }

    #[test]
    fn a_pane_with_no_rect_yet_and_a_collapsed_or_other_window_s_pane_are_not_moved() {
        // `page_rects` has no entry for a pane that never got a rect ("new"): there is nothing to hold.
        let planned = hold((960, 800));
        assert!(planned.iter().all(|(l, _)| l != "new" && l != "hidden" && l != "elsewhere"));
        assert_eq!(planned.len(), 2);
    }

    #[test]
    fn a_late_rect_for_an_older_size_is_placed_as_the_page_sent_it() {
        // The page laid out for 1000 wide; the window is 800 now. Its frame is drawn at 500..1000
        // until it catches up, so the view is too (cut at the window), where stretching would
        // have put it at 400..800.
        let r = Rect { x: 500, y: 100, width: 500, height: 300 };
        assert_eq!(t(clip_to_client(&r, (800, 800))), (500, 100, 300, 300));
        assert_eq!(t(follow_rect(&base(), &r, (800, 800), (0, 0))), (400, 100, 400, 300));
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_page_that_stays_silent_ends_the_hold_after_a_stated_time_and_not_before() {
        let t0 = std::time::Instant::now();
        let mut h = Hold::default();
        assert!(!h.size_changed(t0));
        assert_eq!(h.left(t0), Some(HOLD_MAX));
        assert!(!h.check(t0 + HOLD_MAX - ms(1)));
        assert!(h.check(t0 + HOLD_MAX));
        // Stretching stays on through the window's next changes.
        assert!(h.size_changed(t0 + HOLD_MAX + ms(50)));
        assert_eq!(h.left(t0 + HOLD_MAX + ms(50)), None);
    }

    #[test]
    fn a_page_that_keeps_sending_rects_through_a_long_drag_keeps_its_panes_held() {
        // The drag lasts 5 s and the page's rects (for older sizes) arrive every 250 ms.
        let t0 = std::time::Instant::now();
        let mut h = Hold::default();
        for step in 0..20u64 {
            let t = t0 + ms(step * 250);
            assert!(!h.size_changed(t), "size change at {step}");
            assert!(!h.rect_applied(t + ms(100)), "rect at {step}");
        }
        // It goes quiet: the hold lasts HOLD_MAX from its last rect, not from the drag's start.
        let last = t0 + ms(19 * 250 + 100);
        assert_eq!(h.left(last), Some(HOLD_MAX));
        assert!(!h.check(last + HOLD_MAX - ms(1)));
        assert!(h.check(last + HOLD_MAX));
    }

    #[test]
    fn once_stretching_a_late_rect_is_stretched_too_until_the_page_has_caught_up() {
        let t0 = std::time::Instant::now();
        let mut h = Hold::default();
        h.size_changed(t0);
        // The page's first sign of life is after HOLD_MAX, and for an older size.
        assert!(h.rect_applied(t0 + HOLD_MAX + ms(200)));
        // Its next late rect, and the next size change, are stretched as well: no flip back.
        assert!(h.rect_applied(t0 + HOLD_MAX + ms(300)));
        assert!(h.size_changed(t0 + HOLD_MAX + ms(310)));
        // The page lays out for the window's size: held again for the next change.
        h.caught_up();
        assert!(!h.size_changed(t0 + HOLD_MAX + ms(400)));
        assert!(!h.rect_applied(t0 + HOLD_MAX + ms(450)));
    }

    #[test]
    fn a_window_the_page_has_answered_has_no_hold_to_time_out() {
        let t0 = std::time::Instant::now();
        let mut h = Hold::default();
        assert_eq!(h.left(t0), None);
        h.size_changed(t0);
        h.caught_up();
        assert_eq!(h.left(t0 + HOLD_MAX * 2), None);
        assert!(!h.check(t0 + HOLD_MAX * 2));
    }

    #[test]
    fn the_page_has_answered_once_its_rects_are_for_the_window_s_size() {
        assert!(page_caught_up((1000, 800), (1000, 800)));
        // A fractional display scale: a pixel or two either way.
        assert!(page_caught_up((1000, 800), (1002, 799)));
        // A busy page's rect for a size 200 px behind, or a different height only.
        assert!(!page_caught_up((800, 800), (1000, 800)));
        assert!(!page_caught_up((1000, 700), (1000, 800)));
    }

    fn order(epoch: u64, seq: u64) -> Option<RectsOrder> {
        Some(RectsOrder { epoch, seq })
    }

    fn ids(rects: &[PaneBounds]) -> Vec<&str> {
        rects.iter().map(|r| r.block_id.as_str()).collect()
    }

    #[test]
    fn a_batch_that_arrives_late_loses_only_the_panes_a_newer_one_replaced() {
        let mut newest = NewestBatch::new();
        let second = drop_superseded(&mut newest, "main", vec![b("x", 2)], order(7, 2));
        assert_eq!(ids(&second), ["x"]);
        // The first batch (a: x and y) arrives after the second: y was only in it.
        let first = drop_superseded(&mut newest, "main", vec![b("x", 1), b("y", 1)], order(7, 1));
        assert_eq!(ids(&first), ["y"]);
    }

    #[test]
    fn a_reloaded_page_and_a_page_that_sends_no_order_are_never_superseded() {
        let mut newest = NewestBatch::new();
        drop_superseded(&mut newest, "main", vec![b("x", 1)], order(7, 50));
        assert_eq!(ids(&drop_superseded(&mut newest, "main", vec![b("x", 2)], order(8, 1))), ["x"]);
        assert_eq!(ids(&drop_superseded(&mut newest, "main", vec![b("x", 3)], None)), ["x"]);
    }

    #[test]
    fn a_pane_in_another_window_has_its_own_sequence() {
        let mut newest = NewestBatch::new();
        drop_superseded(&mut newest, "floating-1", vec![b("x", 1)], order(9, 40));
        // The pane is torn off into a window whose page started earlier.
        assert_eq!(ids(&drop_superseded(&mut newest, "main", vec![b("x", 2)], order(7, 3))), ["x"]);
    }

    #[test]
    fn the_order_comes_from_the_batch_when_it_has_one() {
        let args = serde_json::json!({ "epoch": 1_793_000_000_000u64, "seq": 4 });
        assert_eq!(RectsOrder::parse(&args), order(1_793_000_000_000, 4));
        assert_eq!(RectsOrder::parse(&serde_json::json!({ "seq": 4 })), None);
    }

    #[test]
    fn a_late_rect_for_an_older_size_teaches_no_offset() {
        // Still for a while, and the page is caught up apart from rounding: learned.
        assert_eq!(learned_offset(true, (1000, 800), (1002, 799)), Some((2, -1)));
        // The page is slow: its rect is for a size 200 px behind the window.
        assert_eq!(learned_offset(true, (800, 800), (1000, 800)), None);
        // Mid-drag, whatever the rect says.
        assert_eq!(learned_offset(false, (1000, 800), (1001, 800)), None);
    }

    #[test]
    fn taking_wakes_every_waiter() {
        let mut p = PendingByWindow::default();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        p.enqueue("main", vec![b("x", 1)], None, Waiter::Channel(tx1));
        p.enqueue("main", vec![b("y", 2)], None, Waiter::Channel(tx2));
        for w in p.take("main").waiters {
            w.answer();
        }
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }
}
