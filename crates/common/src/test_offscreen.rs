// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A test build that never shows a window on a real display.
//!
//! A tester starts an isolated build with `AGENTMUX_TEST_OFFSCREEN=1`. Every window the host
//! creates then goes to [`ORIGIN`], far outside any display, and nothing saves that place as the
//! person's window position. Before this, the window opened on screen and a separate script
//! raced to move it, and three times (2026-11-15) a tester's window showed on a shared screen.
//!
//! Only a dev or local build honours it ([`build_kind`](crate::build_kind)): a published release
//! ignores the variable, so one inherited from a test instance's shell cannot hide a person's
//! windows. The decision is the binary's, never the variable's alone.
//!
//! Windows and Linux/X11 place a window anywhere; macOS places a pool window at the same
//! place already. Wayland cannot place a window at all, so there the flag does nothing and a
//! tester must use X11 (or XWayland) for a test build.

use crate::build_kind::{build_kind, BuildKind};

/// The environment variable a launch sets.
pub const ENV: &str = "AGENTMUX_TEST_OFFSCREEN";

/// Where a window goes, in both axes: the same corner the window pool parks its spare windows in
/// (`POOL_OFFSCREEN_X/Y`), and the one Windows itself uses for a hidden window.
pub const ORIGIN: i32 = -32000;

/// Whether a build of this `kind` started with `value` in [`ENV`] runs off-screen.
pub fn requested_by(kind: BuildKind, value: Option<&str>) -> bool {
    kind != BuildKind::Release && matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// True when this process creates every window off-screen. Read once.
pub fn active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| requested_by(build_kind(), std::env::var(ENV).ok().as_deref()))
}

/// `(x, y)` for a window that is off-screen, or `pos` when this process is not.
pub fn place(pos: (i32, i32)) -> (i32, i32) {
    if active() {
        (ORIGIN, ORIGIN)
    } else {
        pos
    }
}

/// A window rect `(x, y, width, height)` with its origin moved off-screen when this process is.
pub fn place_rect((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (x, y) = place((x, y));
    (x, y, w, h)
}

/// Log [`start_line`] for `process`, if there is one.
pub fn log_start(process: &str) {
    if let Some(line) = start_line(process) {
        tracing::info!("{line}");
    }
}

/// One log line for the start of a process that says what the variable did, so a tester can
/// tell a build that ignored it from one that honoured it. `None` when it was not set.
pub fn start_line(process: &str) -> Option<String> {
    let value = std::env::var(ENV).ok().filter(|v| !v.trim().is_empty())?;
    Some(if active() {
        format!("{process}: test window mode: every window is created off-screen at ({ORIGIN}, {ORIGIN}) and its place is not saved")
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
    fn the_corner_is_far_outside_any_display() {
        // Wider than 8K, so no arrangement of displays reaches it.
        assert!(ORIGIN <= -16000);
    }
}
