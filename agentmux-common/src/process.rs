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
//! ## The `kill(2)` argument ranges, and why the guards are what they are
//!
//! Callers hand us a `u32`; POSIX `kill(2)` takes a signed `pid_t`, and
//! its argument is **not** simply "a process id". Every range means
//! something different:
//!
//! | `pid` argument | `kill(2)` signals… |
//! |---|---|
//! | `> 0` | exactly that one process |
//! | `== 0` | every process in the **caller's own** process group |
//! | `== -1` | **every process the caller may signal — a system-wide broadcast** |
//! | `< -1` | every process in process group `-pid` |
//!
//! Three successive ReAgent P1s on PR #3033 each caught this module
//! hitting one of the non-literal rows through an innocent-looking `u32`:
//!
//! 1. `u32::MAX - 1` cast with `as` wraps to `-2` → row four, group 2.
//! 2. `0` → row two, the caller's own group (the test runner).
//! 3. `1`, negated for the group call → `-1` → row three, the broadcast.
//!
//! So the guards are derived from the table, not added one hole at a time:
//!
//! - [`checked_pid`] refuses `0` and anything that does not fit `pid_t`,
//!   which is exactly the set of inputs that could reach rows two and
//!   four from the single-PID path. Every `pid_t` that reaches
//!   [`kill_pid`] is strictly positive → row one only.
//! - [`kill_process_group`] additionally requires `pgid >= 2`, because it
//!   negates: `-1` would be row three, and `-0` is `0`, row two. Every
//!   value it passes to `kill(2)` is `<= -2` → row four only, and never
//!   the broadcast.
//!
//! Real PIDs are never 0 or 1 for anything this module should touch
//! (1 is `init`/`systemd`), and never approach `i32::MAX` on any supported
//! platform (Linux `pid_max` caps at 4194304; Windows PIDs are small
//! multiples of 4) — so the guards cost nothing in practice and close the
//! whole class, not just the three instances that were reported.

use std::io;

/// Convert a caller-supplied PID to a signed, **strictly positive** `pid_t`.
/// Refuses `0` (the caller's own process group to `kill(2)`) and anything
/// that would wrap negative (a process group, or the broadcast). Shared by
/// both public functions so the guard can't drift.
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
///
/// `1` is accepted here: to `kill(2)` a positive `1` is exactly one process
/// (row one of the table), not a special value. It only becomes the
/// broadcast when *negated*, which this function never does.
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
        // this is row one of the module-doc table — exactly one process.
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
/// caller can do about a failed kill of an already-dying group.
///
/// A PID of `0`, `1`, or anything out of range for `pid_t` is a **no-op**.
/// `0` and out-of-range are refused by [`checked_pid`]; `1` is refused
/// here specifically because this function negates its argument and
/// `kill(-1, …)` is the system-wide broadcast (row three of the module-doc
/// table), not "process group 1".
pub fn kill_process_group(pid: u32) {
    let Ok(pgid) = checked_pid(pid) else { return };
    if pgid < 2 {
        // pgid == 1 → -pgid == -1 → broadcast to every signalable process.
        // (pgid == 0 is already impossible here; checked_pid refused it.)
        return;
    }
    #[cfg(windows)]
    {
        let _ = taskkill_tree(pgid).status();
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) is a well-defined POSIX syscall; `pgid >= 2` by
        // the guard above, so `-pgid <= -2` — row four of the module-doc
        // table, exactly that one process group. It can never be 0 (the
        // caller's own group) or -1 (the broadcast).
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

    // ── Row two: pid 0 is the caller's own group ────────────────────────

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

    // ── Row three: pgid 1 negated is -1, the broadcast ──────────────────

    /// The third P1: `kill_process_group(1)` would have called
    /// `kill(-1, SIGTERM)` then `kill(-1, SIGKILL)` — every process the
    /// runner may signal. Must be a silent no-op, reached without any
    /// syscall.
    #[test]
    fn kill_process_group_refuses_one() {
        kill_process_group(1);
    }

    // ── Row four via overflow: u32::MAX - 1 wraps to -2 ─────────────────

    #[test]
    fn kill_pid_refuses_a_pid_that_would_wrap_negative() {
        let err = kill_pid(u32::MAX - 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(err.to_string().contains("does not fit"), "{err}");
    }

    #[test]
    fn kill_process_group_refuses_a_pid_that_would_wrap_negative() {
        kill_process_group(u32::MAX - 1);
    }

    // ── Row one: a real, positive, nonexistent pid fails cleanly ────────

    /// `i32::MAX` fits `pid_t`, so the guard must pass it through — and it
    /// cannot be a live process on any supported platform, so the OS
    /// rejects it (`ESRCH` / non-zero `taskkill`). Asserting the error is
    /// NOT the guard's proves the syscall was actually made.
    #[test]
    fn kill_pid_on_a_nonexistent_in_range_pid_is_an_error_not_a_panic() {
        let err = kill_pid(i32::MAX as u32).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::InvalidInput, "guard should NOT have fired: {err}");
    }

    /// The guard's exact contract: strictly positive and fits `pid_t`.
    /// `1` passes `checked_pid` — it is a legitimate single-process target
    /// for `kill_pid`; only `kill_process_group` layers the `>= 2` rule on
    /// top, because only it negates.
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
