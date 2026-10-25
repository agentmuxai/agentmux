// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeping other processes of the same user out of srv's and the host's memory.
//!
//! The host's IPC token and the launcher's secrets live in these processes'
//! memory. An ordinary process of the same user could otherwise read them with
//! `ptrace`, `process_vm_readv` or `/proc/<pid>/mem` (Linux), or by attaching a
//! debugger (macOS). [`deny_memory_access`] turns that off for the calling
//! process: on Linux the process is marked not dumpable (`/proc/<pid>/mem`,
//! `environ`, `maps`, `fd/` and ptrace need `CAP_SYS_PTRACE` from then on; the
//! flag resets by itself in anything the process `exec`s, so the agents srv
//! starts are unaffected); on macOS `PT_DENY_ATTACH` refuses debugger attach.
//!
//! The host calls it once CEF is initialized, not at startup: Chromium's user-namespace
//! sandbox launches the zygotes from the browser process during `CefInitialize`, and a
//! non-dumpable browser process cannot (`No usable sandbox!`). Every helper is started
//! from those zygotes or by `exec`, so none of them inherits the flag.
//!
//! Left alone, on purpose: a headless srv (it runs without a launcher, and the
//! launcher's check for an orphaned one reads its environment), a dev build
//! (people attach debuggers to those), and any process started with
//! `AGENTMUX_ALLOW_PTRACE=1`.

/// Set to anything non-empty to leave a process attachable (a debugger, `perf`).
pub const ALLOW_PTRACE_ENV: &str = "AGENTMUX_ALLOW_PTRACE";

/// Why [`deny_memory_access`] left the process as it was, if it did.
#[derive(Debug, PartialEq, Eq)]
pub enum Skipped {
    OptedOut,
    DevBuild,
    Headless,
}

/// What to do for a process, from the facts about it. Pure, so the policy is tested.
pub fn decide(opt_out: bool, dev_build: bool, headless: bool) -> Result<(), Skipped> {
    if opt_out {
        Err(Skipped::OptedOut)
    } else if dev_build {
        Err(Skipped::DevBuild)
    } else if headless {
        Err(Skipped::Headless)
    } else {
        Ok(())
    }
}

/// Denies other processes of this user access to this process's memory, unless
/// the process is a dev build, headless, or started with [`ALLOW_PTRACE_ENV`].
/// Never fails the caller: a platform without the control, or a refusal, leaves
/// the process as it was. Returns whether the control was applied.
pub fn deny_memory_access(headless: bool) -> bool {
    let opt_out = std::env::var_os(ALLOW_PTRACE_ENV).is_some_and(|v| !v.is_empty());
    let dev_build = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| crate::is_dev_build_exe(dir)))
        .unwrap_or(false);
    if decide(opt_out, dev_build, headless).is_err() {
        return false;
    }
    apply()
}

/// Applies the control, whatever the policy says (`deny_memory_access` decides first).
#[cfg(target_os = "linux")]
pub fn apply() -> bool {
    // SAFETY: prctl with PR_SET_DUMPABLE and a plain integer argument touches no memory.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) == 0 }
}

#[cfg(target_os = "macos")]
pub fn apply() -> bool {
    // PT_DENY_ATTACH (31): the process refuses a debugger from now on.
    const PT_DENY_ATTACH: libc::c_int = 31;
    // SAFETY: ptrace(PT_DENY_ATTACH, 0, NULL, 0) takes no pointer into this process.
    unsafe { libc::ptrace(PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0) == 0 }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn apply() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_process_is_hardened() {
        assert_eq!(decide(false, false, false), Ok(()));
    }

    #[test]
    fn an_opt_out_a_dev_build_and_a_headless_srv_are_left_alone() {
        assert_eq!(decide(true, false, false), Err(Skipped::OptedOut));
        assert_eq!(decide(false, true, false), Err(Skipped::DevBuild));
        assert_eq!(decide(false, false, true), Err(Skipped::Headless));
    }

    // fork, so the test process itself stays dumpable (the test harness reads its own /proc).
    #[cfg(target_os = "linux")]
    #[test]
    fn apply_marks_the_process_not_dumpable() {
        // SAFETY: the child only calls async-signal-safe functions before _exit.
        unsafe {
            let pid = libc::fork();
            assert!(pid >= 0);
            if pid == 0 {
                let before = libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0);
                let applied = apply();
                let after = libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0);
                libc::_exit(if before == 1 && applied && after == 0 {
                    0
                } else {
                    1
                });
            }
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
            assert!(
                libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                "child status {status}"
            );
        }
    }

    // What the control is for: after `apply()` another process of the user (here the parent)
    // can read the child's `/proc/<pid>/stat` but not its `environ`.
    #[cfg(target_os = "linux")]
    #[test]
    fn after_apply_another_process_cannot_read_environ_but_can_read_stat() {
        // SAFETY: the child calls only async-signal-safe functions (write, read, _exit) after fork.
        unsafe {
            let mut ready = [0 as libc::c_int; 2];
            let mut done = [0 as libc::c_int; 2];
            assert_eq!(libc::pipe(ready.as_mut_ptr()), 0);
            assert_eq!(libc::pipe(done.as_mut_ptr()), 0);
            let pid = libc::fork();
            assert!(pid >= 0);
            if pid == 0 {
                let ok = apply();
                let byte = [ok as u8];
                libc::write(ready[1], byte.as_ptr().cast(), 1);
                let mut sink = [0u8; 1];
                libc::read(done[0], sink.as_mut_ptr().cast(), 1);
                libc::_exit(0);
            }
            let mut got = [0u8; 1];
            assert_eq!(libc::read(ready[0], got.as_mut_ptr().cast(), 1), 1);
            let environ = std::fs::read(format!("/proc/{pid}/environ"));
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
            libc::write(done[1], got.as_ptr().cast(), 1);
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
            assert_eq!(got[0], 1, "apply() failed in the child");
            assert!(stat.is_ok(), "stat should stay readable: {stat:?}");
            // Root (a test run as uid 0) can read it whatever the flag says.
            if libc::geteuid() != 0 {
                assert!(
                    environ.is_err(),
                    "environ was readable by another process after apply()"
                );
            }
        }
    }

    // The claim agents depend on: a process this one starts is dumpable again. `execvp` in
    // the forked child (not `Command`), and python3 asks the kernel with PR_GET_DUMPABLE (3).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_program_the_process_starts_is_dumpable_again() {
        use std::ffi::CString;
        let prog = CString::new("python3").unwrap();
        let args = [
            prog.clone(),
            CString::new("-c").unwrap(),
            CString::new("import ctypes, sys; sys.exit(0 if ctypes.CDLL(None).prctl(3, 0, 0, 0, 0) == 1 else 7)").unwrap(),
        ];
        let argv: Vec<*const libc::c_char> = args
            .iter()
            .map(|a| a.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        // SAFETY: the child only calls prctl, execvp and _exit before leaving the forked image.
        unsafe {
            let pid = libc::fork();
            assert!(pid >= 0);
            if pid == 0 {
                if !apply() {
                    libc::_exit(9);
                }
                libc::execvp(prog.as_ptr(), argv.as_ptr());
                libc::_exit(127);
            }
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
            if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 127 {
                return; // no python3 here: nothing to ask the kernel with
            }
            assert!(
                libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                "child status {status}"
            );
        }
    }
}
