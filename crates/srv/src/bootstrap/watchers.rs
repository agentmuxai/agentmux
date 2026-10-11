// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of the single-file bootstrap module unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

/// Start a ppid polling watchdog on Linux/macOS.
/// If the parent process dies, getppid() changes (reparented to init/launchd).
/// This is safer than PR_SET_PDEATHSIG which tracks the parent *thread*, not process,
/// and can fire spuriously with async runtimes like Tokio.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start_ppid_watchdog() {
    let original_ppid = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current_ppid = unsafe { libc::getppid() };
            if current_ppid != original_ppid {
                eprintln!(
                    "parent process died (ppid changed {} -> {}), shutting down",
                    original_ppid, current_ppid
                );
                std::process::exit(0);
            }
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
                eprintln!(
                    "kqueue() failed (errno={}), falling back to ppid watchdog",
                    *libc::__error()
                );
                let _ = kq;
                start_ppid_watchdog();
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
                    eprintln!(
                        "parent process {} already exited (ESRCH during kqueue registration), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                eprintln!(
                    "kevent() registration failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            eprintln!("kqueue EVFILT_PROC registered for parent pid {}", parent_pid);

            // Race condition guard: check if the parent is still alive after registering.
            // If it died between our registration and this check, we might miss the event.
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__error() == libc::ESRCH {
                libc::close(kq);
                eprintln!(
                    "parent process {} already exited (post-registration check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // Block until the parent exits.
            let mut eventlist: [libc::kevent; 1] = std::mem::zeroed();
            let n = libc::kevent(
                kq,
                std::ptr::null(),
                0,
                eventlist.as_mut_ptr(),
                1,
                std::ptr::null(),
            );
            libc::close(kq);

            if n > 0 {
                eprintln!(
                    "parent process {} exited (kqueue EVFILT_PROC), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "kevent() wait returned {} (errno={}), shutting down",
                    n,
                    *libc::__error()
                );
            }
            std::process::exit(0);
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
                    eprintln!(
                        "parent process {} already exited (ESRCH from pidfd_open), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                // ENOSYS means kernel doesn't support pidfd_open — fall back
                eprintln!(
                    "pidfd_open() failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            let pidfd = pidfd as libc::c_int;

            // Race condition guard: verify parent is still alive
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__errno_location() == libc::ESRCH {
                libc::close(pidfd);
                eprintln!(
                    "parent process {} already exited (post-pidfd check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // poll() on the pidfd — blocks until the process exits
            let mut pfd = libc::pollfd {
                fd: pidfd,
                events: libc::POLLIN,
                revents: 0,
            };

            let ret = libc::poll(&mut pfd, 1, -1); // infinite timeout
            libc::close(pidfd);

            if ret > 0 {
                eprintln!(
                    "parent process {} exited (pidfd poll), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "poll() on pidfd returned {} (errno={}), shutting down",
                    ret,
                    *libc::__errno_location()
                );
            }
            std::process::exit(0);
        }
    });
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
            start_ppid_watchdog();
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
