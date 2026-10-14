// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of the single-file bootstrap module unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

/// Start a ppid polling watchdog on Linux/macOS.
/// If the parent process dies, getppid() changes (reparented to init/launchd).
/// This is safer than PR_SET_PDEATHSIG which tracks the parent *thread*, not process,
/// and can fire spuriously with async runtimes like Tokio.
///
/// `original_ppid` is the parent read at startup, not a fresh getppid(): a
/// fallback can start after the parent already died and srv was reparented,
/// and a fresh baseline would then never change. Checks once before sleeping.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start_ppid_watchdog(original_ppid: u32) {
    std::thread::spawn(move || {
        loop {
            let current_ppid = unsafe { libc::getppid() } as u32;
            if current_ppid != original_ppid {
                eprintln_lossy!(
                    "parent process died (ppid changed {} -> {}), shutting down",
                    original_ppid, current_ppid
                );
                std::process::exit(0);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

/// Event-driven parent process watcher using kqueue (macOS) or pidfd (Linux).
/// Monitors a specific PID and exits when that process terminates.
/// Falls back to PPID polling on older Linux kernels without pidfd support.
#[cfg(target_os = "macos")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            let kq = libc::kqueue();
            if kq < 0 {
                eprintln_lossy!(
                    "kqueue() failed (errno={}), falling back to ppid watchdog",
                    *libc::__error()
                );
                let _ = kq;
                start_ppid_watchdog(parent_pid);
                return;
            }

            // Register EVFILT_PROC + NOTE_EXIT on the parent PID.
            let mut changelist: [libc::kevent; 1] = std::mem::zeroed();
            changelist[0] = libc::kevent {
                ident: parent_pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };

            let ret = libc::kevent(
                kq,
                changelist.as_ptr(),
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );

            if ret < 0 {
                let errno = *libc::__error();
                libc::close(kq);
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln_lossy!(
                        "parent process {} already exited (ESRCH during kqueue registration), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                eprintln_lossy!(
                    "kevent() registration failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog(parent_pid);
                return;
            }

            eprintln_lossy!("kqueue EVFILT_PROC registered for parent pid {}", parent_pid);

            // Race condition guard: check if the parent is still alive after registering.
            // If it died between our registration and this check, we might miss the event.
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__error() == libc::ESRCH {
                libc::close(kq);
                eprintln_lossy!(
                    "parent process {} already exited (post-registration check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // Block until the parent exits. A signal srv catches (SIGCHLD
            // from any exiting child) can interrupt the wait with EINTR
            // while the parent is alive: wait again.
            let mut eventlist: [libc::kevent; 1] = std::mem::zeroed();
            let waited = loop {
                let n = libc::kevent(
                    kq,
                    std::ptr::null(),
                    0,
                    eventlist.as_mut_ptr(),
                    1,
                    std::ptr::null(),
                );
                if n > 0 {
                    break Ok(());
                }
                if n == 0 {
                    continue;
                }
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EINTR) {
                    break Err(err);
                }
            };
            libc::close(kq);

            match waited {
                Ok(()) => {
                    eprintln_lossy!(
                        "parent process {} exited (kqueue EVFILT_PROC), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    // A failed wait says nothing about the parent: keep
                    // watching it the slow way rather than shutting down.
                    eprintln_lossy!(
                        "kevent() wait failed ({}), falling back to ppid watchdog",
                        e
                    );
                    start_ppid_watchdog(parent_pid);
                }
            }
        }
    });
}

/// Event-driven parent process watcher using pidfd_open (Linux 5.3+).
/// Falls back to PPID polling on older kernels without pidfd support.
#[cfg(target_os = "linux")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            // Try pidfd_open (syscall 434 on x86_64, 434 on aarch64)
            let pidfd = libc::syscall(libc::SYS_pidfd_open, parent_pid as libc::c_int, 0 as libc::c_int);

            if pidfd < 0 {
                let errno = *libc::__errno_location();
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln_lossy!(
                        "parent process {} already exited (ESRCH from pidfd_open), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                // ENOSYS means kernel doesn't support pidfd_open — fall back
                eprintln_lossy!(
                    "pidfd_open() failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog(parent_pid);
                return;
            }

            let pidfd = pidfd as libc::c_int;

            // Race condition guard: verify parent is still alive
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__errno_location() == libc::ESRCH {
                libc::close(pidfd);
                eprintln_lossy!(
                    "parent process {} already exited (post-pidfd check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            let waited = wait_for_pidfd_exit(pidfd);
            libc::close(pidfd);

            match waited {
                Ok(()) => {
                    eprintln_lossy!(
                        "parent process {} exited (pidfd poll), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    // A failed wait says nothing about the parent: keep
                    // watching it the slow way rather than shutting down.
                    eprintln_lossy!(
                        "poll() on pidfd failed ({}), falling back to ppid watchdog",
                        e
                    );
                    start_ppid_watchdog(parent_pid);
                }
            }
        }
    });
}

/// Blocks until the process behind `pidfd` exits.
///
/// poll() is never restarted after a signal handler runs (signal(7)), and
/// srv catches SIGCHLD, which every exiting child sends. When the kernel
/// delivers one to this thread, poll() fails with EINTR while the parent is
/// still alive, so retry; only a readable pidfd means the parent exited.
#[cfg(target_os = "linux")]
fn wait_for_pidfd_exit(pidfd: libc::c_int) -> std::io::Result<()> {
    let mut pfd = libc::pollfd {
        fd: pidfd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let ret = unsafe { libc::poll(&mut pfd, 1, -1) }; // infinite timeout
        if ret > 0 {
            return Ok(());
        }
        if ret == 0 {
            continue;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
}

/// Step -1: Crash monitor branch — must be checked before any other initialization.
/// The monitor process runs a blocking minidumper::Server and exits when the
/// main process disconnects. It does not run any backend logic.
///
/// Returns `true` if this process was the crash-monitor branch (in which case
/// `main()` must return immediately without doing anything else).
pub fn maybe_run_crash_monitor() -> bool {
    #[cfg(windows)]
    {
        if std::env::args().any(|a| a == "--crash-monitor") {
            crate::crash_monitor::run_monitor();
            return true;
        }
    }
    false
}

/// Step 0: Start parent process watcher BEFORE tokio runtime does real work (Linux/macOS only).
/// On Windows, the frontend uses a Job Object with KILL_ON_JOB_CLOSE instead.
/// Uses getppid() to get the parent PID, then kqueue/pidfd to watch it (event-driven,
/// zero CPU). Falls back to PPID polling if kqueue/pidfd setup fails or parent is init/launchd.
pub fn install_process_watchers() {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let ppid = unsafe { libc::getppid() } as u32;
        if ppid <= 1 {
            // Parent is init/launchd — can't meaningfully watch it, use polling fallback
            start_ppid_watchdog(ppid);
        } else {
            start_parent_watcher(ppid);
        }
    }
}

/// Step 0b: Attach out-of-process crash dump handler (Windows only).
/// Spawns self with --crash-monitor and installs a VEH handler.
/// The returned guard must stay alive — dropping it uninstalls the VEH handler.
/// Non-fatal: if the monitor fails to start, the process continues normally
/// and WER LocalDumps still captures __fastfail crashes independently.
#[cfg(windows)]
pub fn install_crash_guard() -> Option<crate::crash_monitor::CrashHandlerGuard> {
    crate::crash_monitor::spawn_and_attach()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::wait_for_pidfd_exit;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    extern "C" fn ignore_signal(_: libc::c_int) {}

    /// On 2026-10-14 srv exited while its launcher was alive: a signal it
    /// catches landed on the watcher thread, poll() returned EINTR, and the
    /// watcher took that for the launcher exiting. The whole app quit.
    #[test]
    fn a_signal_does_not_end_the_wait_only_the_process_exiting_does() {
        // A caught signal without SA_RESTART, as srv's SIGCHLD is to poll().
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = ignore_signal as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            assert_eq!(
                libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()),
                0
            );
        }

        let mut parent = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, parent.id() as libc::c_int, 0) };
        if pidfd < 0 {
            let err = std::io::Error::last_os_error();
            parent.kill().unwrap();
            parent.wait().unwrap();
            // A kernel before 5.3, or a seccomp policy that blocks it: srv
            // uses the ppid watchdog there, and there is no pidfd to test.
            if matches!(err.raw_os_error(), Some(libc::ENOSYS) | Some(libc::EPERM)) {
                eprintln!("skipping: pidfd_open unavailable ({err})");
                return;
            }
            panic!("pidfd_open: {err}");
        }
        let pidfd = pidfd as libc::c_int;

        let (thread_tx, thread_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            thread_tx.send(unsafe { libc::pthread_self() }).unwrap();
            done_tx
                .send(wait_for_pidfd_exit(pidfd).map_err(|e| e.to_string()))
                .unwrap();
        });
        let waiter = thread_rx.recv().unwrap();

        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(unsafe { libc::pthread_kill(waiter, libc::SIGUSR1) }, 0);
        }
        let early = done_rx.recv_timeout(Duration::from_millis(300));
        assert!(
            early.is_err(),
            "the wait ended while the process was alive: {early:?}"
        );

        parent.kill().unwrap();
        parent.wait().unwrap();
        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the wait never ended");
        unsafe { libc::close(pidfd) };
        assert_eq!(result, Ok(()));
    }
}
