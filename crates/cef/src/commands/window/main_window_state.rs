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
//! Positions are physical pixels, as `GetWindowRect` reports them. [`fit`] puts a
//! saved rect back inside a monitor's work area: a monitor that has been
//! unplugged since, or a resolution that shrank, never leaves the window
//! off screen. Only Windows records the position today (the position stream is a
//! Windows one); elsewhere the main window opens as it always did.

// Only Windows records and restores the place; the pure parts are tested everywhere.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

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

#[cfg(target_os = "windows")]
mod win {
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

    /// The work area of the monitor at a physical point, `None` when no monitor has it.
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

    /// Where the main window opens, `(x, y, width, height)` in DIP as CEF Views
    /// wants it, or `None` for the default (no saved place, or its monitor is gone).
    pub fn initial_bounds() -> Option<(i32, i32, i32, i32)> {
        let state = load()?;
        let (x, y, w, h) = fit(&state, work_area_at)?;
        *NORMAL.lock().unwrap() = Some(MainWindowState { x, y, width: w, height: h, maximized: false });
        START_MAXIMIZED.store(state.maximized, Ordering::Relaxed);
        // A new window is placed in DIP; the rect is physical on the monitor it is on.
        let scale = crate::app::dpi_scale_at(x, y);
        let dip = |v: i32| (v as f32 / scale).round() as i32;
        tracing::info!(x, y, w, h, scale, maximized = state.maximized, "[window] opening the main window where it was");
        Some((dip(x), dip(y), dip(w), dip(h)))
    }

    /// Whether the main window should open maximized; true once, for the delegate.
    pub fn take_start_maximized() -> bool {
        START_MAXIMIZED.swap(false, Ordering::Relaxed)
    }

    /// The main window moved or was resized: `rect` is its `GetWindowRect`, physical
    /// pixels. A maximized window keeps the last normal rect and records that it is
    /// maximized. Written once the window has been still for a moment.
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

#[cfg(target_os = "windows")]
pub(crate) use win::{initial_bounds, note_position, take_start_maximized};

/// Where the main window opens: not recorded off Windows.
#[cfg(not(target_os = "windows"))]
pub(crate) fn initial_bounds() -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(not(target_os = "windows"))]
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
