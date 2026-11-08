// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Platform host-supervision loops. The Windows and Unix supervisors are
//! fully `#[cfg]`-gated in separate files; only the flow spine
//! (`launcher_main`) lives at the crate root.

/// Phase 1 host supervision (spec
/// `docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`): on an
/// abnormal host exit the launcher relaunches the host, but at most
/// `HOST_RESTART_BUDGET` times within `HOST_RESTART_WINDOW` — a crash budget
/// so a deterministic crash cannot spin forever (spec §10-A). Shared by
/// the Windows (`run_windows`) and Unix (`run_unix`) supervisors.
pub(crate) const HOST_RESTART_BUDGET: usize = 3;
pub(crate) const HOST_RESTART_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11 (#942 Phase 2) — srv gets its
/// own crash budget: an unexpected srv exit respawns srv and recycles the
/// host through the existing supervised-restart path (crash-reproject
/// restores the session), at most this many times per window. A wider
/// window than the host's: each srv recycle also costs a host restart, so
/// a crash-looping srv burns both budgets and gives up loudly either way.
pub(crate) const SRV_RESTART_BUDGET: usize = 3;
pub(crate) const SRV_RESTART_WINDOW: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether this abnormal host exit repeats the previous one closely enough to
/// be the same fault twice in a row — what the retry ladder answers with
/// `--disable-gpu` (issue #3666). A manual kill of a hung host also exits with
/// code 1 (`TerminateProcess`, `taskkill /F`), so the code alone pairs two
/// unrelated kills and the degraded relaunch sticks for the rest of the
/// launcher's life. Requiring the previous exit to be within
/// [`HOST_RESTART_WINDOW`] of now makes the step mean a crash soon after a
/// relaunch, which is the case software rendering actually helps; a host that
/// ran for minutes and was then killed degrades nothing.
pub(crate) fn repeats_crash(last: Option<(i32, std::time::Instant)>, code: i32, now: std::time::Instant) -> bool {
    last.is_some_and(|(last_code, at)| last_code == code && now.saturating_duration_since(at) < HOST_RESTART_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_manual_kill_after_a_long_run_is_not_a_repeat_crash() {
        let t0 = Instant::now();
        assert!(!repeats_crash(Some((1, t0)), 1, t0 + Duration::from_secs(8 * 60)));
    }

    #[test]
    fn two_startup_crashes_with_the_same_code_are_a_repeat_crash() {
        let t0 = Instant::now();
        assert!(repeats_crash(Some((1, t0)), 1, t0 + Duration::from_secs(5)));
        assert!(!repeats_crash(Some((1, t0)), 2, t0 + Duration::from_secs(5)), "a different code is a different fault");
        assert!(!repeats_crash(None, 1, t0), "the first crash has nothing to repeat");
    }

    #[test]
    fn the_window_is_the_host_restart_window_and_it_is_exclusive() {
        let t0 = Instant::now();
        assert!(repeats_crash(Some((1, t0)), 1, t0 + HOST_RESTART_WINDOW - Duration::from_millis(1)));
        assert!(!repeats_crash(Some((1, t0)), 1, t0 + HOST_RESTART_WINDOW), "exactly a window later is not a repeat");
    }
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::run_windows;

#[cfg(not(target_os = "windows"))]
mod srv_exit;
#[cfg(not(target_os = "windows"))]
mod unix;
#[cfg(not(target_os = "windows"))]
pub(crate) use unix::run_unix;
