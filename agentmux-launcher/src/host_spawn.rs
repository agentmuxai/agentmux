// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::logging::log;

/// Spawn the CEF host suspended, assign it to the launcher's Job Object, and
/// resume it. Returns the running child, or `None` if any step failed — the
/// caller decides (fatal on first launch, give-up on a restart). `splash_event`
/// is passed on every launch — including restarts — so a relaunched host can
/// still dismiss a splash left pending by a host that crashed pre-first-frame.
/// `disable_gpu` is the retry ladder's rung-2 degraded mode (spec §7): when set
/// the host is launched with `--disable-gpu` (software rendering).
#[cfg(target_os = "windows")]
pub(crate) fn spawn_host_supervised(
    real_exe: &std::path::Path,
    args: &[String],
    srv: &crate::srv_spawner::SrvSpawnResult,
    host_env: &[(&'static str, std::ffi::OsString)],
    pipe_path: &str,
    job_present: bool,
    job_handle: windows_sys::Win32::Foundation::HANDLE,
    splash_event: Option<&str>,
    disable_gpu: bool,
) -> Option<tokio::process::Child> {
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    // The launcher's resolved `real_exe` lives in `runtime/`; pass its
    // parent dir as AGENTMUX_HOME so the host can anchor asset lookups
    // (frontend/index.html, etc.) on something stable rather than
    // `std::env::current_exe()`. Windows's GetModuleFileName keeps
    // returning the original load-time path even after the parent dir
    // is renamed or unlinked out from under it — the 2026-05-28
    // incident pattern. AGENTMUX_HOME is whatever path *we* (the
    // launcher) successfully resolved real_exe through, so it always
    // points at the runtime dir that actually contains the binaries.
    // See docs/retro/retro-portable-rm-running-install-2026-05-28.md.
    let host_runtime_dir = real_exe.parent().map(|p| p.to_path_buf());

    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_PENDING_MIGRATIONS", srv.pending_migrations.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_HOST_REG_SECRET", &srv.host_reg_secret)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        .envs(host_env.iter().cloned())
        // Auto-start (issue #2977 WS2): translate the launcher's own
        // `--background` flag into the env the HOST actually reads. The flag
        // is forwarded to the host in `args` too, but nothing there consumes
        // it — `AGENTMUX_BACKGROUND_SERVICE` is the real switch, so without
        // this an auto-started instance would open a normal foreground window
        // and still quit on last-window-close (Codex P1 on PR #2999).
        // `background_env_for` also sets `AGENTMUX_TRAY`; see its doc for why
        // the indicator is not optional in this particular context.
        .envs(crate::autostart::background_env_for(args))
        .env("AGENTMUX_LAUNCHER_PIPE", pipe_path)
        // Explicit stdio instead of inheriting the launcher's own —
        // `CreateProcess(..., CREATE_SUSPENDED, ...)` for this GUI-subsystem
        // target fails outright with ERROR_NOT_SUPPORTED (os error 50) when
        // it inherits stdio handles that trace back through an MSYS/Git-Bash
        // process chain (e.g. this launcher itself spawned from an agent's
        // own Bash tool call) — confirmed by direct experiment: the same
        // binary starts cleanly when given real Win32 file handles instead
        // of inherited MSYS ones. `Stdio::null()`, not `::piped()`: the host
        // has no stdout/stderr protocol the launcher parses (contrast
        // `srv_spawner`'s piped handles, which ARE consumed — the ESTART
        // readiness signal and log forwarding) — piping without ever
        // reading would risk a silent hang if the host wrote enough output
        // to fill the pipe buffer. See docs/retro/retro-cef-host-spawn-
        // fails-under-inherited-nonstandard-stdio-2026-08-17.md.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_SUSPENDED)
        .kill_on_drop(false); // J0 handles cleanup.
    if let Some(dir) = host_runtime_dir {
        host_cmd.env("AGENTMUX_HOME", dir);
    }
    if let Some(name) = splash_event {
        host_cmd.env("AGENTMUX_SPLASH_EVENT", name);
    }
    // Retry-ladder rung 2 (spec §7): software rendering — no GPU process to
    // crash. A Chromium switch the host forwards to CEF.
    if disable_gpu {
        host_cmd.arg("--disable-gpu");
    }

    let mut host_child = match host_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("failed to spawn CEF host: {}", e));
            return None;
        }
    };
    let host_pid = host_child.id().unwrap_or(0);
    log(&format!("spawned CEF host pid={} (suspended)", host_pid));

    // Assign to J0 BEFORE resuming so CEF render children inherit the job.
    if job_present && host_pid != 0 {
        match crate::srv_spawner::assign_pid_to_job(host_pid, job_handle) {
            Ok(()) => log(&format!(
                "Job Object assigned to host pid={}, KILL_ON_JOB_CLOSE active",
                host_pid
            )),
            Err(e) => log(&format!(
                "WARN: AssignProcessToJobObject(host) failed: {} — host children may escape job",
                e
            )),
        }
    }

    // Resume the suspended main thread.
    if let Err(e) = resume_main_thread(host_pid) {
        log(&format!("failed to resume host pid={}: {}", host_pid, e));
        let _ = host_child.start_kill();
        return None;
    }
    Some(host_child)
}

/// Spawn the CEF host on Unix with the srv endpoints + canonical
/// data-dir env handed off, mirroring `spawn_host_supervised` minus the
/// Windows-only machinery (Job Object assignment, CREATE_SUSPENDED /
/// resume, named-pipe handle, splash event). `disable_gpu` is the retry
/// ladder's degraded rung (software rendering). Returns the running
/// child or `None` if spawn failed.
///
/// AGENTMUX_HOME is intentionally NOT set: on the flat dev layout the
/// host's `current_exe().parent()` fallback resolves to the same dir the
/// launcher would export, so omitting it keeps asset + framework lookup
/// byte-identical to the legacy direct-invoke (`task dev:standalone`)
/// path. Phase 2 will set it once the production runtime/ layout lands
/// on macOS.
#[cfg(not(target_os = "windows"))]
pub(crate) fn spawn_host_unix(
    real_exe: &std::path::Path,
    args: &[String],
    srv: &crate::srv_spawner::SrvSpawnResult,
    host_env: &[(&'static str, std::ffi::OsString)],
    disable_gpu: bool,
) -> Option<tokio::process::Child> {
    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_PENDING_MIGRATIONS", srv.pending_migrations.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_HOST_REG_SECRET", &srv.host_reg_secret)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        // Parent-identity stamp: our pid == the host's getppid (we spawn it
        // directly). A dev-build host normally ignores AGENTMUX_BACKEND_WS
        // (it could be a stale value inherited from a parent agentmux pane);
        // this lets the host verify the hand-off is genuinely ours THIS run
        // and adopt our launcher-owned srv instead of double-spawning. See
        // agentmux-cef/src/main.rs::launcher_is_genuine_parent.
        .env("AGENTMUX_LAUNCHER_PID", std::process::id().to_string())
        .envs(host_env.iter().cloned())
        // We reap children ourselves on shutdown (SIGTERM, then SIGKILL
        // backstop) — kill_on_drop would SIGKILL the host the moment the
        // Child is dropped, robbing CEF of the chance to reap its render
        // subprocesses cleanly.
        .kill_on_drop(false)
        // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 unix parity — make the
        // host its own process-group leader (pgid == its own pid) instead of
        // inheriting the launcher's group. This is the prerequisite for
        // `kill_process_group_forcefully`'s `kill(-pgid, SIGKILL)`: without
        // an explicit group, the host and any CEF subprocesses it forks
        // (renderer/GPU/zygote) sit in the launcher's OWN group, and a
        // group-kill from the teardown backstop would take the launcher
        // down with it mid-cleanup. Scoped exactly to what this call spawns
        // — the same blast-radius bound Windows gets for free from owning
        // its Job Object.
        .process_group(0);
    if disable_gpu {
        host_cmd.arg("--disable-gpu");
    }
    // Linux process-tree reap: PR_SET_PDEATHSIG asks the kernel to SIGKILL
    // this child the moment its parent (the launcher) dies, even abnormally
    // (e.g. launcher panic, OOM, SIGKILL from outside). This is the Linux
    // analogue of Windows' Job Object KILL_ON_JOB_CLOSE that A0 wires up
    // (SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05 §3 step 3 and
    // §A1.4). Without it, a launcher crash orphans the CEF host onto PID 1
    // and the user is left with a zombie tree until manual cleanup.
    //
    // The closure runs between fork() and execve() in the child; per
    // POSIX it MUST NOT allocate, take locks, or call async-signal-unsafe
    // functions. `prctl(2)` is async-signal-safe. macOS lacks prctl, so
    // this is gated to Linux; macOS uses NSTask's auto-reap behavior.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        // Safety: prctl is async-signal-safe. No allocation, no syscalls
        // beyond prctl itself. Errors from prctl are non-fatal — the host
        // still spawns; we just lose the auto-reap guarantee.
        unsafe {
            host_cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                Ok(())
            });
        }
    }
    match host_cmd.spawn() {
        Ok(c) => {
            let pid = c.id().unwrap_or(0);
            log(&format!("spawned CEF host pid={} (unix)", pid));
            Some(c)
        }
        Err(e) => {
            log(&format!("failed to spawn CEF host: {}", e));
            None
        }
    }
}

/// Send SIGTERM to a child so it can shut down gracefully — for the CEF
/// host that means reaping its render subprocesses (a tokio `start_kill`
/// SIGKILL would orphan them); for srv it's a clean shutdown. No-op if the
/// child has already exited (its pid may have been reaped). Best-effort:
/// the SIGKILL grace-window backstop in `run_unix` catches anything that
/// ignores SIGTERM.
#[cfg(not(target_os = "windows"))]
pub(crate) fn terminate_child_gracefully(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: kill(2) with a process-scoped pid + a constant signal —
        // no memory is touched. A stale pid just returns ESRCH (ignored).
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Send `signal` to an entire process group in one shot (negated pid — see
/// the two public wrappers below for why this targets a whole group, not
/// just one process). Shared by both so the return-value handling — a
/// silently-ignored failure would otherwise let a genuinely-failed kill
/// (anything but the expected "already empty" ESRCH) read as a successful
/// teardown — lives in exactly one place. Logs on any failure OTHER than
/// ESRCH (the routine no-op case: the group was already empty, e.g. a prior
/// SIGTERM's own graceful shutdown already reaped it).
#[cfg(not(target_os = "windows"))]
fn kill_process_group(child: &tokio::process::Child, signal: libc::c_int, signal_name: &str) {
    let Some(pid) = child.id() else { return };
    // SAFETY: kill(2) with a process-scoped pid + a constant signal — no
    // memory is touched. A stale pid just returns ESRCH (ignored below).
    let rc = unsafe { libc::kill(-(pid as libc::pid_t), signal) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            crate::logging::log(&format!(
                "WARN: kill(-{}, {}) failed: {}",
                pid, signal_name, err
            ));
        }
    }
}

/// SIGTERM an entire process group — gives a process that installs a signal
/// handler (srv does: SIGINT/SIGTERM → `shell_sessions.stop_all()`,
/// `agentmux-srv/src/main.rs`) a chance to run its own graceful shutdown
/// before the harder `kill_process_group_forcefully` follows. Matters
/// specifically for srv: its tracked agent shells (`shell_node.rs`) are each
/// spawned into THEIR OWN process group (`.process_group(0)`, same mechanism
/// this backstop itself uses, applied for a different scoping purpose), so
/// they are NOT members of srv's own group and a bare group-SIGKILL of srv
/// would orphan them. `stop_all()` reaches them anyway — `kill_tree()`
/// signals each tracked shell BY ITS OWN pid/pgid directly
/// (`kill(-shell_pgid, SIGTERM)` then `SIGKILL`), independent of ambient
/// process-group membership. host is not presumed capable of a meaningful
/// graceful response here (the whole premise of a teardown-backstop
/// scenario is that its UI thread is wedged) but SIGTERM-ing it too is
/// harmless — a wedged process with no custom SIGTERM handler just
/// terminates on it the same as SIGKILL would.
#[cfg(not(target_os = "windows"))]
pub(crate) fn kill_process_group_gracefully(child: &tokio::process::Child) {
    kill_process_group(child, libc::SIGTERM, "SIGTERM");
}

/// SIGKILL an entire process group in one shot — the unix analogue of
/// Windows' `TerminateJobObject`. `child` must have been spawned with
/// `.process_group(0)` (both `spawn_host_unix` and `srv_spawner::spawn_srv`
/// do this) so its pgid equals its own pid; negating the pid targets the
/// whole group instead of just the one process, reaping any descendants
/// (e.g. CEF's forked renderer/GPU subprocesses) the launcher never tracks
/// individually. Used by the teardown backstop
/// (SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11) as the hard backstop after
/// `kill_process_group_gracefully`'s brief grace window — reserved for
/// whatever a graceful SIGTERM didn't reap (a wedged-or-dead srv, or shells
/// its own `stop_all()` couldn't finish in time). No-op if the child has
/// already exited (its pid may have been reaped; a stale pid's negated kill
/// returns ESRCH, ignored).
#[cfg(not(target_os = "windows"))]
pub(crate) fn kill_process_group_forcefully(child: &tokio::process::Child) {
    kill_process_group(child, libc::SIGKILL, "SIGKILL");
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    //! `kill_process_group_forcefully` and the `.process_group(0)` calls in
    //! this file / `srv_spawner.rs` are the prerequisite the SPEC_LAUNCHER_
    //! TEARDOWN_BACKSTOP_2026_07_11 unix teardown backstop depends on — a
    //! survey ahead of this change (issue #2188) confirmed NO unix child of
    //! the launcher was ever placed in its own process group, so
    //! `kill(-pgid, SIGKILL)` had no safe target. These exercise the two
    //! real invariants against actually-spawned processes rather than
    //! asserting on the builder call alone, since a wrong signature (e.g.
    //! `.process_group(pid)` instead of `.process_group(0)`) would compile
    //! fine and silently fail to create a new group.

    use super::{kill_process_group_forcefully, kill_process_group_gracefully};

    /// `.process_group(0)` makes the spawned child its own group leader —
    /// pgid must equal its own pid, not the test process's pgid.
    #[tokio::test]
    async fn process_group_zero_makes_the_child_its_own_group_leader() {
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("5").kill_on_drop(true).process_group(0);
        let mut child = cmd.spawn().expect("spawn sleep");
        let pid = child.id().expect("child has a pid") as libc::pid_t;

        // SAFETY: getpgid with process-scoped pids; no memory touched.
        let pgid = unsafe { libc::getpgid(pid) };
        let our_pgid = unsafe { libc::getpgid(0) };
        assert_eq!(pgid, pid, "process_group(0) must make pgid == pid");
        // The behavioral half of the invariant (reagent P1, PR #2200): the
        // child must NOT be in our own group, since that's exactly what
        // makes it stop receiving a group-directed signal (e.g. a terminal
        // Ctrl+C, which the shell delivers to its whole foreground group) —
        // the tradeoff this change makes host/srv exit via the launcher's
        // explicit terminate_child_gracefully instead of direct propagation.
        assert_ne!(
            pgid, our_pgid,
            "process_group(0) must isolate the child from our own group, \
             or a group-directed signal (e.g. terminal Ctrl+C) would still reach it"
        );

        let _ = child.kill().await;
    }

    /// The kill actually reaches the process (signal-terminated, not a
    /// silent no-op) — the behavioral half of the prerequisite, mirroring
    /// what the teardown backstop's live-fire path depends on.
    #[tokio::test]
    async fn kill_process_group_forcefully_terminates_a_real_child() {
        use std::os::unix::process::ExitStatusExt;

        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("5").kill_on_drop(false).process_group(0);
        let mut child = cmd.spawn().expect("spawn sleep");

        kill_process_group_forcefully(&child);

        let status = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait())
            .await
            .expect("child did not exit after group kill")
            .expect("wait succeeded");
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    /// A child that already exited (pid reaped) must not panic or error —
    /// the teardown arm calls this unconditionally on both host and srv
    /// without checking liveness first.
    #[tokio::test]
    async fn kill_process_group_forcefully_is_a_noop_on_an_already_exited_child() {
        let mut cmd = tokio::process::Command::new("true");
        cmd.kill_on_drop(false).process_group(0);
        let mut child = cmd.spawn().expect("spawn true");
        let _ = child.wait().await; // let it exit and get reaped

        kill_process_group_forcefully(&child); // must not panic
    }

    /// `kill_process_group_gracefully` sends SIGTERM, not SIGKILL — the
    /// half of the two-phase teardown a process WITH a signal handler (srv)
    /// gets a real chance to act on.
    #[tokio::test]
    async fn kill_process_group_gracefully_sends_sigterm_not_sigkill() {
        use std::os::unix::process::ExitStatusExt;

        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("5").kill_on_drop(false).process_group(0);
        let mut child = cmd.spawn().expect("spawn sleep");

        kill_process_group_gracefully(&child);

        let status = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait())
            .await
            .expect("child did not exit after graceful group kill")
            .expect("wait succeeded");
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    /// Same no-op-on-already-exited guarantee as the forceful variant — the
    /// teardown arm calls both unconditionally.
    #[tokio::test]
    async fn kill_process_group_gracefully_is_a_noop_on_an_already_exited_child() {
        let mut cmd = tokio::process::Command::new("true");
        cmd.kill_on_drop(false).process_group(0);
        let mut child = cmd.spawn().expect("spawn true");
        let _ = child.wait().await; // let it exit and get reaped

        kill_process_group_gracefully(&child); // must not panic
    }
}

/// Resume the (single) main thread of a CREATE_SUSPENDED process.
///
/// Walks a Toolhelp32 thread snapshot to find the one thread belonging
/// to `pid` (a freshly-spawned suspended process has only its main
/// thread), opens it with THREAD_SUSPEND_RESUME, and ResumeThread's it.
///
/// Errors come from snapshot creation, OpenThread, or ResumeThread
/// returning `(DWORD)-1`. A `ResumeThread` return of 0 means the thread
/// was already running (impossible if the process was just created
/// suspended) — treated as success.
#[cfg(target_os = "windows")]
pub(crate) fn resume_main_thread(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
        THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err("CreateToolhelp32Snapshot failed".into());
        }

        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        let mut found = false;
        if Thread32First(snap, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        let prev = ResumeThread(thread);
                        CloseHandle(thread);
                        if prev == u32::MAX {
                            CloseHandle(snap);
                            return Err(format!(
                                "ResumeThread failed for tid={}",
                                entry.th32ThreadID
                            ));
                        }
                        found = true;
                        break;
                    }
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                if Thread32Next(snap, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snap);
        if !found {
            return Err(format!("no thread found for pid={}", pid));
        }
        Ok(())
    }
}
