// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the main window was when AgentMux last ran, so it opens there again
//! after a quit, a crash or an upgrade (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`
//! §6.1: every other window is reopened where it was from the quit record; the
//! main window was always opened centred at 70% of its monitor).
//!
//! The rect and whether the window was maximized are written beside the
//! channel's settings (`AGENTMUX_CONFIG_DIR/main-window.json`), not into a
//! version's data folder, so an upgrade, which starts a new version's data, keeps
//! it. It is the host's own file: the main window is placed before srv has said
//! anything, and a hard kill still leaves the last place it was at.
//!
//! On Windows positions are physical pixels, as `GetWindowRect` reports them; on
//! macOS and Linux they are points (DIP), as CEF Views reports them in screen
//! coordinates with the origin at the top-left of the primary display. [`fit`] puts
//! a saved rect back inside a monitor's work area: a monitor that has been unplugged
//! since, or a resolution that shrank, never leaves the window off screen.
//! Windows, macOS and Linux record the position.
//!
//! **Linux on Wayland** (the native-Wayland backend, `--ozone-platform=wayland`)
//! cannot place a window: the compositor decides. There only the size and whether
//! it was maximized are restored ([`fit_size_only`]); the position is saved
//! but not used. On X11, including XWayland, the position is restored as well.

// Only Windows, macOS and Linux record and restore the place; the pure parts are tested everywhere.
#![cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "linux")), allow(dead_code))]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where the state lives, under the channel's config folder.
const FILE_NAME: &str = "main-window.json";
/// The smallest a saved window may be: the window's own floor is 380 wide.
const MIN_WIDTH: i32 = 380;
const MIN_HEIGHT: i32 = 200;
/// A rect this far off is a corrupt file, not a window.
const LIMIT: i32 = 100_000;

/// The main window's last normal place, and whether it was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainWindowState {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Maximized when last seen; the rect is then where it goes when restored.
    #[serde(default)]
    pub maximized: bool,
}

/// A work area, `(x, y, width, height)` in physical pixels.
pub type WorkArea = (i32, i32, i32, i32);

/// The state in `text`, or `None` for anything that isn't a real window rect.
pub fn parse(text: &str) -> Option<MainWindowState> {
    let s: MainWindowState = serde_json::from_str(text).ok()?;
    let sane = |v: i32| (-LIMIT..=LIMIT).contains(&v);
    (sane(s.x) && sane(s.y) && s.width >= MIN_WIDTH && s.height >= MIN_HEIGHT && s.width <= LIMIT && s.height <= LIMIT)
        .then_some(s)
}

/// Where the saved rect goes now, `(x, y, width, height)` in physical pixels:
/// inside the work area of the monitor its centre is on, shrunk to fit it. `None`
/// when that spot is on no monitor now (one was unplugged), so the window opens
/// where a new one goes. `work_area_at` answers for a physical point, `None` off
/// every monitor.
pub fn fit(state: &MainWindowState, work_area_at: impl Fn(i32, i32) -> Option<WorkArea>) -> Option<(i32, i32, i32, i32)> {
    let (cx, cy) = (state.x + state.width / 2, state.y + state.height / 2);
    let (wx, wy, ww, wh) = work_area_at(cx, cy)?;
    if ww <= 0 || wh <= 0 {
        return None;
    }
    let width = state.width.min(ww);
    let height = state.height.min(wh);
    let x = state.x.clamp(wx, wx + ww - width);
    let y = state.y.clamp(wy, wy + wh - height);
    Some((x, y, width, height))
}

/// Where the saved rect goes when the position can't be chosen (native Wayland): the
/// saved size, shrunk to fit `work`, centred in it. `(x, y, width, height)`; the
/// compositor ignores the position, it is only a well-formed one.
#[cfg(any(target_os = "linux", test))]
pub fn fit_size_only(state: &MainWindowState, work: WorkArea) -> Option<(i32, i32, i32, i32)> {
    let (wx, wy, ww, wh) = work;
    if ww <= 0 || wh <= 0 {
        return None;
    }
    let width = state.width.min(ww);
    let height = state.height.min(wh);
    Some((wx + (ww - width) / 2, wy + (wh - height) / 2, width, height))
}

/// Whether windows can be put where we say on the ozone platform this process runs on
/// (`None`: nothing was appended, Chromium's X11 default). Native Wayland can't.
#[cfg(any(target_os = "linux", test))]
pub fn places_windows(ozone_platform: Option<&str>) -> bool {
    ozone_platform != Some("wayland")
}

/// Whether `rect` (`left, top, right, bottom`) fills `work` (give or take a point
/// or two): how a macOS window that was zoomed looks, before CEF reports it as
/// maximized (NSWindow's zoom sets the frame to the display's visible frame), and
/// how a Linux window the window manager maximized looks before CEF says so.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub fn fills_work_area(rect: &agentmux_common::ipc::Rect, work: WorkArea) -> bool {
    const SLACK: i32 = 2;
    let (wx, wy, ww, wh) = work;
    ww > 0
        && wh > 0
        && (rect.left - wx).abs() <= SLACK
        && (rect.top - wy).abs() <= SLACK
        && (rect.right - (wx + ww)).abs() <= SLACK
        && (rect.bottom - (wy + wh)).abs() <= SLACK
}

/// The state file, in the channel's config folder.
fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("AGENTMUX_CONFIG_DIR")
        .or_else(|| std::env::var_os("AGENTMUX_CONFIG_HOME"))
        .filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join(FILE_NAME))
}

/// The saved state, if there is a valid one.
pub fn load() -> Option<MainWindowState> {
    parse(&std::fs::read_to_string(path()?).ok()?)
}

/// Keep `state`, best effort: written beside the target and renamed over it, so a
/// kill mid-write leaves the old file.
pub fn save(state: &MainWindowState) {
    let Some(path) = path() else { return };
    let Ok(text) = serde_json::to_string(state) else { return };
    let tmp = path.with_extension("json.tmp");
    let result = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        tracing::warn!(error = %e, "[window] couldn't keep the main window's place");
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod live {
    use super::{fit, load, save, MainWindowState, WorkArea};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::commands::window::trailing_writer::TrailingWriter;

    /// How long the position must stay still before it is written (a drag is one write).
    const WRITE_DEBOUNCE_MS: u64 = 1500;
    static WRITER: TrailingWriter = TrailingWriter::new("main-window", Duration::from_millis(WRITE_DEBOUNCE_MS));
    /// The last normal (not maximized) rect seen, for a maximized window to keep.
    static NORMAL: Mutex<Option<MainWindowState>> = Mutex::new(None);
    /// The window opens maximized: read by its delegate once it is created.
    static START_MAXIMIZED: AtomicBool = AtomicBool::new(false);

    /// The work area of the monitor at a point in the saved rect's units, `None`
    /// when no monitor has it.
    #[cfg(target_os = "windows")]
    fn work_area_at(x: i32, y: i32) -> Option<WorkArea> {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONULL};
        unsafe {
            let hmon = MonitorFromPoint(windows_sys::Win32::Foundation::POINT { x, y }, MONITOR_DEFAULTTONULL);
            if hmon.is_null() {
                return None;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut info) == 0 {
                return None;
            }
            let rc = info.rcWork;
            Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn work_area_at(x: i32, y: i32) -> Option<WorkArea> {
        crate::app::display_work_area_at(x, y)
    }

    /// DIP per saved unit at `(x, y)`: the saved rect is physical on Windows and
    /// already in DIP (points) on macOS and Linux.
    #[cfg(target_os = "windows")]
    fn dip_scale(x: i32, y: i32) -> f32 {
        crate::app::dpi_scale_at(x, y)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn dip_scale(_x: i32, _y: i32) -> f32 {
        1.0
    }

    /// Whether the main window's `rect` (points) fills the work area of the display
    /// it is on. CEF's `is_maximized()` lags the frame while NSWindow's zoom settles
    /// (it still says no after a zoom and still says yes after a restore), so a
    /// zoomed frame is recognised by its size. Linux window managers are the same.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn fills_its_display(rect: &agentmux_common::ipc::Rect) -> bool {
        let (cx, cy) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        work_area_at(cx, cy).is_some_and(|work| super::fills_work_area(rect, work))
    }

    /// Where the saved rect goes now. Native Wayland can't place a window, so there
    /// it is the saved size centred on the primary display; everywhere else, [`fit`].
    #[cfg(target_os = "linux")]
    fn place(state: &MainWindowState) -> Option<(i32, i32, i32, i32)> {
        if super::places_windows(crate::app::window_settings::SELECTED_OZONE_PLATFORM.get().map(String::as_str)) {
            fit(state, work_area_at)
        } else {
            super::fit_size_only(state, crate::app::primary_work_area()?)
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn place(state: &MainWindowState) -> Option<(i32, i32, i32, i32)> {
        fit(state, work_area_at)
    }

    /// Where the main window opens, `(x, y, width, height)` in DIP as CEF Views
    /// wants it, or `None` for the default (no saved place, or its monitor is gone).
    pub fn initial_bounds() -> Option<(i32, i32, i32, i32)> {
        let state = load()?;
        let (x, y, w, h) = place(&state)?;
        *NORMAL.lock().unwrap() = Some(MainWindowState { x, y, width: w, height: h, maximized: false });
        START_MAXIMIZED.store(state.maximized, Ordering::Relaxed);
        // A new window is placed in DIP; on Windows the rect is physical on the monitor it is on.
        let scale = dip_scale(x, y);
        let dip = |v: i32| (v as f32 / scale).round() as i32;
        tracing::info!(x, y, w, h, scale, maximized = state.maximized, "[window] opening the main window where it was");
        Some((dip(x), dip(y), dip(w), dip(h)))
    }

    /// Whether the main window should open maximized; true once, for the delegate.
    pub fn take_start_maximized() -> bool {
        START_MAXIMIZED.swap(false, Ordering::Relaxed)
    }

    /// The main window moved or was resized: `rect` is its screen rect (physical
    /// pixels on Windows, `GetWindowRect`; points on macOS). A maximized window
    /// keeps the last normal rect and records that it is maximized. Written once
    /// the window has been still for a moment.
    pub fn note_position(maximized: bool, rect: agentmux_common::ipc::Rect) {
        let mut normal = NORMAL.lock().unwrap();
        let next = if maximized {
            // Nothing normal seen yet this run (it started maximized without a saved
            // rect): there is no restore place to keep.
            match *normal {
                Some(n) => MainWindowState { maximized: true, ..n },
                None => return,
            }
        } else {
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            if width <= 0 || height <= 0 {
                return;
            }
            MainWindowState { x: rect.left, y: rect.top, width, height, maximized: false }
        };
        if !maximized {
            *normal = Some(next);
        }
        drop(normal);
        WRITER.schedule("main", move || save(&next));
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub(crate) use live::{initial_bounds, note_position, take_start_maximized};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use live::fills_its_display;

/// Where the main window opens: not recorded off Windows, macOS and Linux.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub(crate) fn initial_bounds() -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub(crate) fn take_start_maximized() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: WorkArea = (0, 0, 1920, 1040);
    const LEFT: WorkArea = (-1920, 0, 1920, 1040);

    fn state(x: i32, y: i32, width: i32, height: i32) -> MainWindowState {
        MainWindowState { x, y, width, height, maximized: false }
    }
    /// Monitors: `LEFT` and `ONE` side by side; nowhere else.
    fn two(x: i32, y: i32) -> Option<WorkArea> {
        [LEFT, ONE].into_iter().find(|&(wx, wy, ww, wh)| x >= wx && x < wx + ww && y >= wy && y < wy + wh)
    }
    fn one(x: i32, y: i32) -> Option<WorkArea> {
        two(x, y).filter(|w| *w == ONE)
    }

    #[test]
    fn a_saved_rect_inside_a_monitor_comes_back_as_it_was() {
        assert_eq!(fit(&state(100, 80, 1200, 800), two), Some((100, 80, 1200, 800)));
        // On the left monitor, a negative x.
        assert_eq!(fit(&state(-1800, 50, 1200, 800), two), Some((-1800, 50, 1200, 800)));
    }

    #[test]
    fn a_monitor_that_is_gone_means_the_default_place() {
        // Saved on the left monitor, which is unplugged now.
        assert_eq!(fit(&state(-1800, 50, 1200, 800), one), None);
    }

    #[test]
    fn a_rect_hanging_off_its_monitor_is_pulled_back_in() {
        // Centre on the monitor, but the right part off it and the bottom below it.
        assert_eq!(fit(&state(1200, 600, 1200, 800), two), Some((720, 240, 1200, 800)));
        // Title bar above the work area.
        assert_eq!(fit(&state(100, -40, 1200, 800), two).map(|r| r.1), Some(0));
    }

    #[test]
    fn a_rect_bigger_than_the_monitor_shrinks_to_its_work_area() {
        // The monitor got smaller since: 3000x2000 on a 1920x1040 work area.
        assert_eq!(fit(&state(0, 0, 3000, 2000), two), Some((0, 0, 1920, 1040)));
    }

    #[test]
    fn native_wayland_restores_the_size_and_not_the_position() {
        // The compositor places windows there: the platform says so, and the saved size is
        // centred on the primary display whatever x and y were saved.
        assert!(!places_windows(Some("wayland")));
        assert!(places_windows(Some("x11")));
        assert!(places_windows(None));
        let primary: WorkArea = (0, 30, 1920, 1050);
        let at = |x, y, w, h| MainWindowState { x, y, width: w, height: h, maximized: false };
        assert_eq!(fit_size_only(&at(5000, -400, 1000, 700), primary), Some((460, 205, 1000, 700)));
        assert_eq!(fit_size_only(&at(0, 0, 1000, 700), primary), Some((460, 205, 1000, 700)));
        // A size the display can't hold shrinks to it; a display with no area gives nothing.
        assert_eq!(fit_size_only(&at(10, 10, 4000, 3000), primary), Some((0, 30, 1920, 1050)));
        assert_eq!(fit_size_only(&at(10, 10, 1000, 700), (0, 0, 0, 0)), None);
    }

    #[test]
    fn a_saved_window_on_a_display_that_is_gone_opens_where_a_new_one_goes() {
        let state = at_default();
        // Saved on a second display that has been unplugged: no work area for its centre.
        let only_primary = |x: i32, y: i32| (0..1920).contains(&x).then_some(()).and((0..1080).contains(&y).then_some((0, 0, 1920, 1080)));
        let gone = MainWindowState { x: 2000, y: 100, width: 900, height: 600, maximized: true };
        assert_eq!(fit(&gone, only_primary), None);
        // Still on the primary: kept, and clamped inside it when it ran off an edge.
        assert_eq!(fit(&state, only_primary), Some((state.x, state.y, state.width, state.height)));
        let partly_off = MainWindowState { x: 1200, y: 700, width: 900, height: 600, maximized: false };
        assert_eq!(fit(&partly_off, only_primary), Some((1020, 480, 900, 600)));
    }

    fn at_default() -> MainWindowState {
        MainWindowState { x: 100, y: 80, width: 1000, height: 700, maximized: false }
    }

    #[test]
    fn a_corrupt_or_missing_state_file_opens_the_default() {
        for text in ["", "not json", "{}", "{\"x\":1}", "{\"x\":1,\"y\":1,\"width\":5,\"height\":5}", "{\"x\":1,\"y\":1,\"width\":900,\"height\":600,\"maximized\":\"yes\"}"] {
            assert_eq!(parse(text), None, "{text:?}");
        }
        assert!(parse("{\"x\":1,\"y\":1,\"width\":900,\"height\":600}").is_some());
    }

    #[test]
    fn a_rect_filling_the_work_area_is_a_zoomed_window() {
        use agentmux_common::ipc::Rect;
        let work: WorkArea = (51, 40, 1659, 1072);
        let r = |l, t, rr, b| Rect { left: l, top: t, right: rr, bottom: b };
        assert!(fills_work_area(&r(51, 40, 1710, 1112), work));
        // A point or two off still counts; a real margin does not.
        assert!(fills_work_area(&r(52, 41, 1709, 1111), work));
        assert!(!fills_work_area(&r(150, 120, 1050, 740), work));
        assert!(!fills_work_area(&r(51, 40, 1710, 1000), work));
        assert!(!fills_work_area(&r(0, 0, 10, 10), (0, 0, 0, 0)));
    }

    #[test]
    fn only_a_real_rect_parses() {
        assert_eq!(
            parse(r#"{"x":10,"y":20,"width":1200,"height":800,"maximized":true}"#),
            Some(MainWindowState { x: 10, y: 20, width: 1200, height: 800, maximized: true })
        );
        // `maximized` is optional.
        assert_eq!(parse(r#"{"x":-1800,"y":0,"width":1200,"height":800}"#).map(|s| s.maximized), Some(false));
        // A window that was never placed, too small, or wildly off: nothing to restore.
        assert_eq!(parse(r#"{"x":0,"y":0,"width":0,"height":0}"#), None);
        assert_eq!(parse(r#"{"x":0,"y":0,"width":100,"height":100}"#), None);
        assert_eq!(parse(r#"{"x":99999999,"y":0,"width":1200,"height":800}"#), None);
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"x":1}"#), None);
    }

    #[test]
    fn the_state_round_trips() {
        let s = MainWindowState { x: -5, y: 7, width: 1000, height: 700, maximized: true };
        assert_eq!(parse(&serde_json::to_string(&s).unwrap()), Some(s));
    }
}
