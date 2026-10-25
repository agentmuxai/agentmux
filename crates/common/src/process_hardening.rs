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
//! starts are unaffected); on macOS `PT_DENY_ATTACH` refuses debugger attach; on Windows
//! the process's access-control list stops other processes of the user reading or
//! changing its memory (see [`win`]).
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

/// Windows: the process's access-control list keeps other processes of the user to seeing it,
/// waiting for it and ending it. See [`win`].
#[cfg(windows)]
pub fn apply() -> bool {
    win::harden()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn apply() -> bool {
    false
}

/// Lets the user's other processes read this process again, for the one caller that needs it: the
/// crash handler, just before the out-of-process writer opens the crashing process to write its
/// dump. Does nothing where [`apply`] did nothing. Call [`restore`] afterwards.
#[cfg(windows)]
pub fn relax() -> bool {
    win::relax()
}

#[cfg(not(windows))]
pub fn relax() -> bool {
    false
}

/// Puts back what [`apply`] set, after [`relax`].
#[cfg(windows)]
pub fn restore() -> bool {
    win::restore()
}

#[cfg(not(windows))]
pub fn restore() -> bool {
    false
}

/// The Windows control: a new access-control list on the process object.
///
/// Another process of the same user opens this one with `OpenProcess`, and what it can then do
/// is whatever the list grants that user. Left alone, that includes `ReadProcessMemory`,
/// injecting a thread, duplicating handles out of the process and attaching a debugger. After
/// [`harden`] the user keeps what the tools that watch and stop a process need (its existence,
/// times, working set and command line; waiting for it; ending it; putting it in a job), and
/// loses reading and writing its memory, threads, handles and the right to rewrite the list.
///
/// A handle opened before the change keeps the rights it was opened with. That is what lets the
/// launcher, which holds the handles `CreateProcess` gave it for srv and the host, still wait
/// for them, end them and write a dump of a hung host.
///
/// The system account keeps every right, so Windows Error Reporting still writes its dumps.
#[cfg(windows)]
mod win {
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, SetKernelObjectSecurity, TokenUser, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcessToken, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// `SYNCHRONIZE`: waiting for the process to end.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    /// `PROCESS_ALL_ACCESS`.
    const ALL_ACCESS: u32 = 0x001F_FFFF;

    /// What the user's other processes keep. `PROCESS_QUERY_INFORMATION` stays because
    /// `taskkill /F` refuses to end a process without it; with no `PROCESS_VM_READ` it shows
    /// nothing of the memory. `PROCESS_SET_QUOTA` is the other half of putting a process in a job.
    const USER_RIGHTS: u32 =
        PROCESS_TERMINATE | PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE;

    /// The list for a hardened process: the system account everything; the user [`USER_RIGHTS`];
    /// and an explicit empty entry for the owner. Without that last entry Windows lets the
    /// owner of an object open it for `WRITE_DAC` whatever the list says, and a process that can
    /// rewrite the list can give itself memory access again.
    pub(super) fn hardened_sddl(user_sid: &str) -> String {
        format!("D:P(A;;0x{ALL_ACCESS:X};;;SY)(A;;0x{USER_RIGHTS:X};;;{user_sid})(A;;;;;OW)")
    }

    /// The list for a process the user may open freely, as Windows gives a new one.
    pub(super) fn open_sddl(user_sid: &str) -> String {
        format!("D:P(A;;0x{ALL_ACCESS:X};;;SY)(A;;0x{ALL_ACCESS:X};;;{user_sid})")
    }

    /// A security descriptor made once from its text and kept for the life of the process, so
    /// that switching lists later (in a crash handler) allocates nothing.
    struct Prepared(usize);

    fn prepare(sddl: &str) -> Option<Prepared> {
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `wide` is NUL-terminated and outlives the call; `sd` receives a LocalAlloc'd
        // descriptor which this module keeps for the life of the process.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(wide.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
        };
        (ok != 0 && !sd.is_null()).then(|| Prepared(sd as usize))
    }

    fn set(p: &Prepared) -> bool {
        // SAFETY: the pseudo-handle for this process needs no closing; `p.0` is a valid
        // descriptor kept alive for the life of the process.
        unsafe { SetKernelObjectSecurity(GetCurrentProcess(), DACL_SECURITY_INFORMATION, p.0 as PSECURITY_DESCRIPTOR) != 0 }
    }

    static LISTS: OnceLock<Option<(Prepared, Prepared)>> = OnceLock::new();

    /// The hardened and the open list for this process's user, made on first use.
    fn lists() -> Option<&'static (Prepared, Prepared)> {
        LISTS
            .get_or_init(|| {
                let sid = user_sid()?;
                Some((prepare(&hardened_sddl(&sid))?, prepare(&open_sddl(&sid))?))
            })
            .as_ref()
    }

    /// Whether [`harden`] has taken effect: the switches below do nothing before it, and
    /// never allocate (they run in a crash handler).
    static HARDENED: AtomicBool = AtomicBool::new(false);

    pub(super) fn harden() -> bool {
        let ok = lists().is_some_and(|(hardened, _)| set(hardened));
        if ok {
            HARDENED.store(true, Ordering::SeqCst);
        }
        ok
    }

    pub(super) fn relax() -> bool {
        HARDENED.load(Ordering::SeqCst) && LISTS.get().and_then(Option::as_ref).is_some_and(|(_, open)| set(open))
    }

    pub(super) fn restore() -> bool {
        HARDENED.load(Ordering::SeqCst) && LISTS.get().and_then(Option::as_ref).is_some_and(|(hardened, _)| set(hardened))
    }

    /// This process's user as the text form of its SID.
    fn user_sid() -> Option<String> {
        // SAFETY: plain Win32 calls on this process's own token, closed on every path; the
        // buffer is 8-byte aligned and as long as the call asked for.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let got = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) != 0;
            CloseHandle(token);
            if !got {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut text: *mut u16 = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 || text.is_null() {
                return None;
            }
            let n = (0..).take_while(|&i| *text.add(i) != 0).count();
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            Some(sid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_process_is_hardened() {
        assert_eq!(decide(false, false, false), Ok(()));
    }

    #[cfg(windows)]
    #[test]
    fn the_hardened_list_names_what_the_user_keeps_and_takes_the_owner_s_implicit_rights() {
        let sddl = win::hardened_sddl("S-1-5-21-1-2-3-1001");
        // terminate, set quota, query information, query limited information, synchronize
        assert_eq!(sddl, "D:P(A;;0x1FFFFF;;;SY)(A;;0x101501;;;S-1-5-21-1-2-3-1001)(A;;;;;OW)");
        assert!(win::open_sddl("S-1-5-21-1-2-3-1001").contains("(A;;0x1FFFFF;;;S-1-5-21-1-2-3-1001)"));
    }

    /// Runs only as the child of the next test: hardens itself, then follows its stdin: "relax"
    /// switches to the open list, "restore" back, and the end of input ends it.
    #[cfg(windows)]
    #[test]
    #[ignore = "runs as the child of another_process_of_the_user_is_refused_the_memory_of_a_hardened_one"]
    fn hardened_process_child() {
        use std::io::{BufRead, Write};
        if std::env::var_os("AGENTMUX_PH_CHILD").is_none() {
            return;
        }
        println!("HARDENED {}", apply());
        std::io::stdout().flush().unwrap();
        for line in std::io::stdin().lock().lines() {
            let done = match line.unwrap().as_str() {
                "relax" => relax(),
                "restore" => restore(),
                _ => false,
            };
            println!("DONE {done}");
            std::io::stdout().flush().unwrap();
        }
    }

    #[cfg(windows)]
    fn open_error(access: u32, pid: u32) -> Option<u32> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::Threading::OpenProcess;
        // SAFETY: OpenProcess on a pid, handle closed on success.
        unsafe {
            let h = OpenProcess(access, 0, pid);
            if h.is_null() {
                Some(GetLastError())
            } else {
                CloseHandle(h);
                None
            }
        }
    }

    // The proof, with a real second process: a child of this test hardens itself and this one,
    // which is the same user, is refused its memory and kept to watching and ending it.
    #[cfg(windows)]
    #[test]
    fn another_process_of_the_user_is_refused_the_memory_of_a_hardened_one() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        const VM_READ: u32 = 0x10;
        const VM_WRITE_OPERATION: u32 = 0x28;
        const CREATE_THREAD: u32 = 0x2;
        const DUP_HANDLE: u32 = 0x40;
        const WRITE_DAC: u32 = 0x40000;
        const QUERY_LIMITED: u32 = 0x1000;
        const QUERY: u32 = 0x400;
        const TERMINATE_QUOTA: u32 = 0x101;
        const SYNCHRONIZE: u32 = 0x10_0000;
        const ACCESS_DENIED: u32 = 5;

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_hardening::tests::hardened_process_child", "--ignored", "--nocapture"])
            .env("AGENTMUX_PH_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut hardened = None;
        for line in lines.by_ref() {
            if let Some(v) = line.unwrap().strip_prefix("HARDENED ") {
                hardened = Some(v.to_string());
                break;
            }
        }
        assert_eq!(hardened.as_deref(), Some("true"), "the child hardened itself");

        for (name, access) in [
            ("read memory", VM_READ),
            ("write memory", VM_WRITE_OPERATION),
            ("start a thread", CREATE_THREAD),
            ("duplicate a handle", DUP_HANDLE),
            ("rewrite the list", WRITE_DAC),
        ] {
            assert_eq!(open_error(access, pid), Some(ACCESS_DENIED), "must not {name}");
        }
        for (name, access) in [
            ("query limited information", QUERY_LIMITED),
            ("query information", QUERY),
            ("wait", SYNCHRONIZE),
            ("end it and put it in a job", TERMINATE_QUOTA),
        ] {
            assert_eq!(open_error(access, pid), None, "must still {name}");
        }

        // The crash path's switch: while relaxed the memory can be opened, and not after restoring.
        let mut stdin = child.stdin.take().unwrap();
        let mut ask = |word: &str| {
            use std::io::Write;
            writeln!(stdin, "{word}").unwrap();
            stdin.flush().unwrap();
            lines.next().unwrap().unwrap()
        };
        assert_eq!(ask("relax"), "DONE true");
        assert_eq!(open_error(VM_READ, pid), None, "relaxed: the memory can be opened");
        assert_eq!(ask("restore"), "DONE true");
        assert_eq!(open_error(VM_READ, pid), Some(ACCESS_DENIED), "restored: refused again");

        // Ending the child: its creator's handle (this process) has kept every right.
        drop(stdin);
        assert!(child.wait().unwrap().success());
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
