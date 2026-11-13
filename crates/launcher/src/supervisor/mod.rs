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
/// `--disable-gpu` (issue #3666).
///
/// A manual kill of a hung host also exits with code 1 (`TerminateProcess`,
/// `taskkill /F`), so the code alone pairs two unrelated kills and the degraded
/// relaunch sticks for the rest of the launcher's life. Two things pair an exit
/// instead:
///   - a Windows fault code (an NTSTATUS exception, negative as `i32`): a real
///     crash that repeats is deterministic however long the host ran between;
///   - the same code as an exit within [`HOST_RESTART_WINDOW`] of the previous
///     one: a crash soon after a relaunch, which is the case software rendering
///     actually helps. A host that ran for minutes and was then killed is
///     neither, so it degrades nothing.
pub(crate) fn repeats_crash(last: Option<(i32, std::time::Instant)>, code: i32, now: std::time::Instant) -> bool {
    match last {
        Some((last_code, at)) => last_code == code && (is_fault_code(code) || now.saturating_duration_since(at) < HOST_RESTART_WINDOW),
        None => false,
    }
}

/// Whether a host exit is the one that says "the browser sandbox is blocked by system policy and nobody can answer
/// the dialog" (`agentmux_common::process::HOST_EXIT_SANDBOX_UNAVAILABLE`, set by `agentmux-cef` for a start made
/// from an agent's environment). The same stop would come again at every relaunch, so the supervisor ends with
/// that code at once instead of spending its crash budget on it.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn is_sandbox_unavailable_exit(code: i32) -> bool {
    code == agentmux_common::process::HOST_EXIT_SANDBOX_UNAVAILABLE
}

/// The log line for a host exit that ends the launcher without a relaunch: a clean exit, or the stop for a blocked
/// sandbox (#610), which says why where the person or agent reading this log looks.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn final_host_exit_line(code: i32) -> String {
    if is_sandbox_unavailable_exit(code) {
        format!(
            "CEF host stopped at start-up (code {code}): the browser sandbox is blocked by system policy and this start came \
             from an agent's environment, so nobody can answer the dialog. Not relaunching. The host's stderr says why; \
             AGENTMUX_UNSAFE_NOSANDBOX=1 is the explicit opt-out (docs/linux.md)."
        )
    } else {
        format!("CEF host exited cleanly (code {code}) — shutting down")
    }
}

/// `STATUS_CONTROL_C_EXIT`: the console interrupted the process (Ctrl+C or a
/// close) — a shutdown, not a fault, whatever its NTSTATUS-looking value.
const STATUS_CONTROL_C_EXIT: i32 = 0xC000_013A_u32 as i32;

/// Whether an exit code is cast as a Windows fault: an NTSTATUS exception
/// (negative as `i32`), except the console's own control-C exit.
fn is_fault_code(code: i32) -> bool {
    code < 0 && code != STATUS_CONTROL_C_EXIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn the_final_exit_line_says_which_stop_it_is() {
        assert_eq!(final_host_exit_line(0), "CEF host exited cleanly (code 0) — shutting down");
        let l = final_host_exit_line(agentmux_common::process::HOST_EXIT_SANDBOX_UNAVAILABLE);
        assert!(l.contains("code 78") && l.contains("Not relaunching") && l.contains("AGENTMUX_UNSAFE_NOSANDBOX=1"), "{l}");
    }

    #[test]
    fn only_the_sandbox_unavailable_code_ends_the_launcher_without_a_relaunch() {
        assert!(is_sandbox_unavailable_exit(agentmux_common::process::HOST_EXIT_SANDBOX_UNAVAILABLE));
        for ordinary in [0, 1, 2, -1, 101] {
            assert!(!is_sandbox_unavailable_exit(ordinary), "{ordinary} keeps the crash-budget relaunch");
        }
    }

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

    #[test]
    fn a_repeated_windows_fault_code_is_deterministic_however_long_between() {
        // An NTSTATUS exception (0xC0000005 as i32) is a real crash, not a
        // TerminateProcess(1) kill: it still steps the ladder down after a
        // long run, as before.
        let access_violation = 0xC0000005u32 as i32;
        let t0 = Instant::now();
        assert!(access_violation < 0, "precondition: the exception code is negative as i32");
        assert!(repeats_crash(Some((access_violation, t0)), access_violation, t0 + Duration::from_secs(8 * 60)));
        // A console interrupt is a shutdown, not a fault, even though its
        // NTSTATUS value looks like one.
        assert!(!repeats_crash(Some((STATUS_CONTROL_C_EXIT, t0)), STATUS_CONTROL_C_EXIT, t0 + Duration::from_secs(8 * 60)));
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
