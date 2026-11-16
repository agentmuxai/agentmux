// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A test build that never shows a window on a real display.
//!
//! A tester starts an isolated build with `AGENTMUX_TEST_OFFSCREEN=1`. Every window the host
//! creates then goes to [`ORIGIN`], far above every display, and nothing saves that place as the
//! person's window position. Before this, the window opened on screen and a separate script
//! raced to move it, and three times (2026-11-15) a tester's window showed on a shared screen.
//!
//! Only a dev or local build honours it ([`build_kind`](crate::build_kind)): a published release
//! ignores the variable, so one inherited from a test instance's shell cannot hide a person's
//! windows. The decision is the binary's, never the variable's alone.
//!
//! Windows and Linux/X11 place a window anywhere. Wayland cannot place a window at all, so there
//! the flag does nothing and a tester must use X11 (or XWayland) for a test build.
//!
//! macOS cannot park a window: AppKit puts a shown window onto a display. There no window is ever
//! shown at all (`test_offscreen_macos`): the process blocks every way to order one in, refuses to
//! start if it cannot, and stops itself if one is on the on-screen list anyway.

use crate::build_kind::{build_kind, BuildKind};

/// The environment variable a launch sets.
pub const ENV: &str = "AGENTMUX_TEST_OFFSCREEN";

/// Where a window goes: far above every display, with its left edge at 0. Not the pool's corner
/// (-32000, -32000): the host reads a window whose left edge is under -20000 as a spare pool window
/// (`OFFSCREEN_POOL_THRESHOLD_X`), counts it as closed, and ends the app when it is the only one.
pub const ORIGIN: (i32, i32) = (0, -32000);

/// Whether a build of this `kind` started with `value` in [`ENV`] runs off-screen.
pub fn requested_by(kind: BuildKind, value: Option<&str>) -> bool {
    kind != BuildKind::Release && matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// True when this process creates every window off-screen. Read once.
pub fn active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| requested_by(build_kind(), std::env::var(ENV).ok().as_deref()))
}

/// A spare pool window waits parked with its left edge under this (the host's
/// `OFFSCREEN_POOL_THRESHOLD_X`): it is already off-screen and stays there until it is promoted.
pub const PARKED_BELOW_X: i32 = -20000;

/// Where a window goes: `pos` unless `off_screen`; and then [`ORIGIN`], except a parked pool window.
fn placed(off_screen: bool, pos: (i32, i32)) -> (i32, i32) {
    if off_screen && pos.0 >= PARKED_BELOW_X {
        ORIGIN
    } else {
        pos
    }
}

/// `(x, y)` for a window that is off-screen, or `pos` when this process is not.
pub fn place(pos: (i32, i32)) -> (i32, i32) {
    placed(active(), pos)
}

/// A window rect `(x, y, width, height)` with its origin moved off-screen when this process is.
pub fn place_rect((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (x, y) = place((x, y));
    (x, y, w, h)
}

/// The rect a window is created at when this process is off-screen, `None` when it is not: CEF
/// places a window whose initial bounds are empty at (0, 0), so the window must say where it starts.
/// `bounds` are the ones the window was asked for, if any (a spare pool window keeps its parking place).
pub fn initial_rect(bounds: Option<(i32, i32, i32, i32)>) -> Option<(i32, i32, i32, i32)> {
    active().then(|| place_rect(bounds.unwrap_or((0, 0, 1200, 800))))
}

/// Why a process must not start: it was asked to stay off-screen and cannot. On macOS AppKit moves a
/// window onto a display when it is first shown and clamps every later move, so the only way is that
/// no window is ever ordered in; `test_offscreen_macos` makes that so,
/// and a process where it could not be made so refuses to start rather than show a window.
#[cfg(target_os = "macos")]
fn engage(process: &str) -> Result<(), String> {
    crate::test_offscreen_macos::engage(process).map_err(|why| {
        format!("{ENV}=1 needs every window kept from being shown on macOS, and that could not be set up ({why}). Nothing was started.")
    })
}

#[cfg(not(target_os = "macos"))]
fn engage(_process: &str) -> Result<(), String> {
    Ok(())
}

/// `NSApplicationActivationPolicy` raw values.
const ACTIVATION_REGULAR: isize = 0;
const ACTIVATION_ACCESSORY: isize = 1;

fn activation_policy(off_screen: bool) -> isize {
    if off_screen {
        ACTIVATION_ACCESSORY
    } else {
        ACTIVATION_REGULAR
    }
}

/// The macOS activation policy the host sets: a normal Dock app, or an accessory app in test window
/// mode (no Dock tile, no menu bar, never activated by itself), so a test run leaves no icon in the
/// person's Dock and cannot take the keyboard from their frontmost app.
pub fn macos_activation_policy() -> isize {
    activation_policy(active())
}

/// The start of a process: when it runs off-screen it first makes sure it can (on macOS, that no
/// window will ever be ordered in), and refuses (one line on stderr, exit status 2, nothing started)
/// when it cannot. It then logs [`start_line`] if there is one.
pub fn begin(process: &str) {
    if active() {
        if let Err(why) = engage(process) {
            eprintln!("{process}: {why}");
            std::process::exit(2);
        }
    }
    if let Some(line) = start_line(process) {
        tracing::info!("{line}");
    }
}

/// One log line for the start of a process that says what the variable did, so a tester can
/// tell a build that ignored it from one that honoured it. `None` when it was not set.
pub fn start_line(process: &str) -> Option<String> {
    let value = std::env::var(ENV).ok().filter(|v| !v.trim().is_empty())?;
    Some(if active() && cfg!(target_os = "macos") {
        format!("{process}: test window mode: no window is ever shown (every way to order one in is blocked, a watchdog stops the process if one is on the screen) and its place is not saved")
    } else if active() {
        format!("{process}: test window mode: every window is created off-screen at ({}, {}) and its place is not saved", ORIGIN.0, ORIGIN.1)
    } else {
        format!("{process}: {ENV}={value} ignored: only a dev or local build runs off-screen")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_or_local_build_honours_it() {
        for kind in [BuildKind::Dev, BuildKind::Local] {
            for v in ["1", "true", "yes", "on", " 1 "] {
                assert!(requested_by(kind, Some(v)), "{kind:?} {v:?}");
            }
        }
    }

    #[test]
    fn a_published_release_never_does() {
        for v in ["1", "true", "yes", "on"] {
            assert!(!requested_by(BuildKind::Release, Some(v)), "{v:?}");
        }
    }

    #[test]
    fn unset_empty_or_other_values_do_not() {
        for v in [None, Some(""), Some("0"), Some("false"), Some("no"), Some("off"), Some("2")] {
            assert!(!requested_by(BuildKind::Dev, v), "{v:?}");
        }
    }

    #[test]
    fn a_process_that_is_not_off_screen_gives_no_initial_rect() {
        // The tests run without the variable set.
        assert_eq!(initial_rect(None), None);
        assert_eq!(initial_rect(Some((5, 6, 7, 8))), None);
        assert_eq!(place_rect((10, 20, 30, 40)), (10, 20, 30, 40));
    }

    #[test]
    fn a_test_build_is_an_accessory_app_and_a_normal_one_is_not() {
        assert_eq!(activation_policy(true), 1);
        assert_eq!(activation_policy(false), 0);
        // The tests run without the variable set.
        assert_eq!(macos_activation_policy(), 0);
    }

    #[test]
    fn a_spare_pool_window_keeps_its_parking_place() {
        // The pool parks at (-32000, -32000), or that scaled by the display (-25600 at 80%).
        for pos in [(-32000, -32000), (-25600, -25600), (-20001, 0)] {
            assert_eq!(placed(true, pos), pos);
        }
        // Everything else goes to the origin, including a window at 0,0 and on a display left of the primary.
        for pos in [(0, 0), (500, 300), (-1920, 0), (-20000, 0), (-19999, 0)] {
            assert_eq!(placed(true, pos), ORIGIN, "{pos:?}");
        }
        assert_eq!(placed(false, (500, 300)), (500, 300));
    }

    #[test]
    fn the_corner_is_far_outside_any_display() {
        // Higher than any arrangement of displays reaches.
        assert!(ORIGIN.1 <= -16000);
        // And not under the host's pool threshold, which reads such a window as a spare one.
        assert!(ORIGIN.0 > -20000);
    }
}
