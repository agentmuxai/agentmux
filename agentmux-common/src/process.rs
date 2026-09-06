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

use std::io;

/// Terminate one process by PID — `SIGTERM` on Unix; `taskkill /F /T /PID`
/// on Windows (which also takes the tree, since Windows has no
/// process-group signal).
///
/// Returns `Err` if the signal/command could not be delivered; a process
/// that has already exited surfaces as an error too (Unix `ESRCH`, Windows
/// non-zero `taskkill` exit) — callers treating "already gone" as success
/// should ignore the result, which is what every prior copy did.
pub fn kill_pid(pid: u32) -> io::Result<()> {
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
        // SAFETY: kill(2) is a well-defined POSIX syscall on any pid value.
        let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
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
pub fn kill_process_group(pid: u32) {
    #[cfg(windows)]
    {
        let _ = taskkill_tree(pid).status();
    }
    #[cfg(unix)]
    {
        let pgid = pid as libc::pid_t;
        // SAFETY: kill(2) is a well-defined POSIX syscall; a negative pid
        // targets the process group.
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    }
}

/// `taskkill /F /T /PID <pid>` with stdio detached and the console flash
/// suppressed — the one Windows kill command, built once.
#[cfg(windows)]
fn taskkill_tree(pid: u32) -> std::process::Command {
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

    /// Killing a PID that cannot exist must fail cleanly, not panic — the
    /// helpers run in teardown paths where a panic would mask the real
    /// error. PID 0 is never a valid target on either platform (Unix:
    /// `kill(0, …)` would hit the *caller's* group, which the helper never
    /// does because it targets `pid` itself for `kill_pid`; we use a
    /// clearly-dead high PID instead).
    #[test]
    fn kill_pid_on_dead_pid_is_an_error_not_a_panic() {
        // u32::MAX - 1 is not a live PID on any supported platform.
        let r = kill_pid(u32::MAX - 1);
        assert!(r.is_err());
    }

    #[test]
    fn kill_process_group_on_dead_pid_does_not_panic() {
        kill_process_group(u32::MAX - 1);
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
