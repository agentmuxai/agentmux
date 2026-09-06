// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Best-effort child-process termination, by PID.
//!
//! Before this module, the same two operations were implemented five
//! times across four crates — `agentmux-srv` (twice), `agentmux-cef`,
//! `agentmux-bashwrap` — one of which described itself as a *"Mirror of
//! the cef-side helper"* (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`
//! §2.2). Only one of the five suppressed the console flash that `taskkill`
//! otherwise causes from a GUI-subsystem parent; here every Windows path
//! does.
//!
//! **This is not `process_tracker`.** That module owns *tracked* agent
//! process trees (Job Object / cgroup / pgid) and is the right tool when
//! the child was assigned to a tracker at spawn. These helpers are for the
//! untracked cases — a one-shot login CLI, a shell node, a pager left
//! behind by a hook — where all the caller has is a PID.
//!
//! Every function is scoped strictly to the given PID (and, on Windows,
//! its tree via `/T`) — **never** by image name. Killing by image name is
//! the cross-instance hazard CLAUDE.md bans outright.
//!
//! ## The `pid_t` guard
//!
//! Callers hand us a `u32`; POSIX `kill(2)` takes a signed `pid_t`, and
//! two of its argument ranges do not mean "one process" at all:
//!
//! - **negative** `pid` means *"signal this process group"* — and a raw
//!   `as` cast of any `u32` above `i32::MAX` wraps negative. ReAgent P1 on
//!   PR #3033 caught exactly this in the first version of this module's
//!   own tests: `u32::MAX - 1` wrapped to `-2`, targeting process group 2,
//!   and the group variant negated it to signal PID 2.
//! - **zero** means *"every process in the caller's own process group"*.
//!   The first fix guarded only [`kill_process_group`] against this; a
//!   second ReAgent P1 pointed out [`kill_pid`]`(0)` still reached
//!   `kill(0, SIGTERM)` — a group-wide kill from the single-PID path.
//!
//! So [`checked_pid`] refuses both: anything that does not fit `pid_t`,
//! and `0`. Both public functions go through it, so the guard cannot
//! drift between them, and every `pid_t` that reaches a syscall is
//! **strictly positive** by construction. Real PIDs are never 0 and never
//! approach `i32::MAX` on any supported platform (Linux `pid_max` caps at
//! 4194304; Windows PIDs are small multiples of 4), so the guard costs
//! nothing in practice and closes the hazard entirely.

use std::io;

/// Convert a caller-supplied PID to a signed, **strictly positive** `pid_t`.
/// Refuses `0` (the caller's own process group to `kill(2)`) and anything
/// that would wrap negative (a process group to `kill(2)`). Shared by both
/// public functions so the guard can't drift.
fn checked_pid(pid: u32) -> io::Result<i32> {
    if pid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pid 0 would target the caller's own process group — refusing",
        ));
    }
    i32::try_from(pid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("pid {pid} does not fit in pid_t — refusing (would wrap negative)"),
        )
    })
}

/// Terminate one process by PID — `SIGTERM` on Unix; `taskkill /F /T /PID`
/// on Windows (which also takes the tree, since Windows has no
/// process-group signal).
///
/// Returns `Err` if the PID is `0` or out of range for `pid_t` (see the
/// module doc), or if the signal/command could not be delivered. A process
/// that has already exited surfaces as an error too (Unix `ESRCH`, Windows
/// non-zero `taskkill` exit) — callers treating "already gone" as success
/// should ignore the result, which is what every prior copy did.
pub fn kill_pid(pid: u32) -> io::Result<()> {
    let pid = checked_pid(pid)?;
    #[cfg(windows)]
    {
        let status = taskkill_tree(pid).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("taskkill exit {:?}", status.code())))
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) is a well-defined POSIX syscall; `pid` is a
        // checked, strictly-positive pid_t (never 0, never negative), so
        // this addresses exactly one process — never a group.
        let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Terminate a process **group** led by `pid`, with escalation: `SIGTERM`
/// to the group, a short grace, then `SIGKILL` to the group. On Windows
/// the same `taskkill /F /T /PID` as [`kill_pid`] — there is no group
/// signal, and `/T` already takes the tree.
///
/// Unix callers must have spawned the child as a group leader
/// (`process_group(0)` / `setsid`) for the negative-pid signal to reach
/// its descendants; that is the contract every prior copy relied on.
/// Best-effort: nothing is returned because there is nothing useful a
/// caller can do about a failed kill of an already-dying group. A PID
/// that is `0` or out of range for `pid_t` is a **no-op** — never a signal
/// to the wrong group (see the module doc).
pub fn kill_process_group(pid: u32) {
    // checked_pid already refuses 0 and anything that would wrap, so
    // `pgid` is strictly positive here — `-pgid` can never be 0 (the
    // caller's own group) or -1 (every process).
    let Ok(pgid) = checked_pid(pid) else { return };
    #[cfg(windows)]
    {
        let _ = taskkill_tree(pgid).status();
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) is a well-defined POSIX syscall; `pgid` is a
        // checked, strictly-positive pid_t, so `-pgid` addresses exactly
        // that one group.
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    }
}

/// `taskkill /F /T /PID <pid>` with stdio detached and the console flash
/// suppressed — the one Windows kill command, built once.
#[cfg(windows)]
fn taskkill_tree(pid: i32) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(crate::win32::CREATE_NO_WINDOW);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pid 0 means "the caller's own process group" to `kill(2)`. Both
    /// entry points must refuse it before any syscall — this is the
    /// difference between a no-op and killing the test runner.
    #[test]
    fn kill_pid_refuses_zero() {
        let err = kill_pid(0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(err.to_string().contains("own process group"), "{err}");
    }

    #[test]
    fn kill_process_group_refuses_zero() {
        kill_process_group(0);
    }

    /// The overflow case ReAgent caught: `u32::MAX - 1` cast to `i32` is
    /// `-2`, which `kill(2)` reads as "process group 2". The guard must
    /// reject it *before* any syscall, on every platform.
    #[test]
    fn kill_pid_refuses_a_pid_that_would_wrap_negative() {
        let err = kill_pid(u32::MAX - 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(err.to_string().contains("does not fit"), "{err}");
    }

    /// Same input through the group variant: wrapping to `-2` and then
    /// negating would have signalled PID 2. Must be a silent no-op.
    #[test]
    fn kill_process_group_refuses_a_pid_that_would_wrap_negative() {
        kill_process_group(u32::MAX - 1);
    }

    /// The boundary itself: `i32::MAX` fits `pid_t`, so the guard must
    /// pass it through — and it cannot be a live process on any supported
    /// platform (Linux pid_max ≤ 4194304; Windows PIDs are small multiples
    /// of 4), so the OS rejects it cleanly (`ESRCH` / non-zero `taskkill`).
    /// This is the real "nonexistent PID fails cleanly, not by panic" test.
    #[test]
    fn kill_pid_on_a_nonexistent_in_range_pid_is_an_error_not_a_panic() {
        let err = kill_pid(i32::MAX as u32).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::InvalidInput, "guard should NOT have fired: {err}");
    }

    /// The guard's exact contract: strictly positive and fits `pid_t`.
    #[test]
    fn checked_pid_boundaries() {
        assert!(checked_pid(0).is_err(), "0 is the caller's own group");
        assert_eq!(checked_pid(1).unwrap(), 1);
        assert_eq!(checked_pid(i32::MAX as u32).unwrap(), i32::MAX);
        assert!(checked_pid(i32::MAX as u32 + 1).is_err());
        assert!(checked_pid(u32::MAX).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn taskkill_command_targets_pid_never_image_name() {
        let cmd = taskkill_tree(4242);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["/F", "/T", "/PID", "4242"]);
        assert!(!args.iter().any(|a| a.eq_ignore_ascii_case("/IM")));
    }
}
