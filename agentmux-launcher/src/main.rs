// AgentMux Launcher — Sets DLL search path then spawns srv + the CEF host.
//
// Phase B.1: launcher now spawns srv directly (sibling of host) so srv
// survives host crashes. Both children are assigned to the launcher's
// Job Object J0 with KILL_ON_JOB_CLOSE; killing the launcher reaps
// the entire tree atomically via the OS.
//
// This was previously a tiny sync wrapper that just SetDllDirectoryW'd
// runtime/ then spawned the CEF host. Phase B grew it into the
// privileged owner per
// specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md.
//
// Process tree after B.1:
//   launcher (J0)
//     ├── srv     (assigned to J0; survives host crash)
//     └── host    (assigned to J0; CEF render workers inherit J0)

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod data_dir;
mod srv_spawner;

#[tokio::main]
async fn main() {
    let exe_path = std::env::current_exe().expect("cannot resolve exe path");
    let exe_dir = exe_path.parent().expect("exe has no parent directory");
    let runtime_dir = exe_dir.join("runtime");

    log(&format!(
        "starting — exe={} runtime={}",
        exe_path.display(),
        runtime_dir.display()
    ));

    // Set DLL search path so libcef.dll (in runtime/) is found by the
    // CEF host's load-time linker. SetDllDirectoryW is process-local
    // and inherited by child processes — both srv (which doesn't
    // need libcef but harmless) and host (which absolutely does).
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = runtime_dir
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
        }
    }
    log("SetDllDirectoryW done");

    let real_exe = find_cef_binary(&runtime_dir);
    log(&format!("resolved CEF binary: {}", real_exe.display()));
    if !real_exe.exists() {
        log(&format!(
            "FATAL: CEF binary not found at {}",
            real_exe.display()
        ));
        eprintln!(
            "AgentMux runtime not found in: {}\nMake sure the runtime/ folder is intact.",
            runtime_dir.display()
        );
        std::process::exit(1);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&format!("forwarding {} CLI args to host", args.len()));

    #[cfg(target_os = "windows")]
    {
        run_windows(exe_dir, &real_exe, &args).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Phase 7 covers cross-platform parity. For now the legacy
        // exec-into-host path is preserved on macOS/Linux. Phase B.1
        // is Windows-only.
        log("exec into CEF host (Unix)");
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&real_exe).args(&args).exec();
        log(&format!("FATAL: exec failed: {}", err));
        eprintln!("Failed to launch AgentMux: {}", err);
        std::process::exit(1);
    }
}

/// Windows main flow: resolve paths → create J0 → spawn srv → spawn
/// host with srv endpoints in env → concurrent wait → cleanup.
#[cfg(target_os = "windows")]
async fn run_windows(
    launcher_exe_dir: &std::path::Path,
    real_exe: &std::path::Path,
    args: &[String],
) {
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    let version = env!("CARGO_PKG_VERSION");
    let is_dev = cfg!(debug_assertions);

    // 1. Resolve data_dir / config_dir / user_home_dir. Both srv and
    // host receive these via env so they don't recompute (and so they
    // can't drift). Host's existing data_dir computation in sidecar.rs
    // still runs as a fallback for `task dev` mode where the launcher
    // is not in the loop.
    let paths = match data_dir::resolve_paths(launcher_exe_dir, version, is_dev) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("FATAL: path resolution failed: {}", e));
            eprintln!("Failed to resolve AgentMux data directories: {}", e);
            std::process::exit(1);
        }
    };
    log(&format!(
        "paths resolved: data={} config={} user_home={} portable={}",
        paths.data_dir.display(),
        paths.config_dir.display(),
        paths.user_home_dir.display(),
        paths.portable_root.is_some(),
    ));
    if let Err(e) = data_dir::ensure_dirs(&paths) {
        log(&format!("FATAL: failed to create data dirs: {}", e));
        eprintln!("{}", e);
        std::process::exit(1);
    }

    // 2. Create the launcher's Job Object J0 BEFORE any spawn. Both
    // srv and host will be assigned to it (so they're siblings under
    // a single OS-enforced cleanup contract). Failure here drops us
    // into "degraded mode" — children spawn but won't be reaped on
    // launcher death.
    let job: Option<JobHandle> = match create_job_object() {
        Ok(handle) => {
            log("Job Object created (KILL_ON_JOB_CLOSE active)");
            Some(JobHandle(handle))
        }
        Err(e) => {
            log(&format!(
                "WARN: Job Object setup failed: {} (process-tree cleanup degraded)",
                e
            ));
            None
        }
    };
    let job_handle: windows_sys::Win32::Foundation::HANDLE =
        job.as_ref().map(|j| j.0).unwrap_or(std::ptr::null_mut());

    // 3. Spawn srv first. Host needs srv's endpoints to skip its own
    // spawn_backend path. Srv signals readiness via WAVESRV-ESTART on
    // stderr; the spawner returns once we see that line (or after a
    // 30s timeout).
    let (srv_result, mut srv_child) = match srv_spawner::spawn_srv(
        launcher_exe_dir,
        &paths,
        job_handle,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            log(&format!("FATAL: srv spawn failed: {}", e));
            eprintln!("Failed to start backend: {}", e);
            drop(job);
            std::process::exit(1);
        }
    };

    // CRITICAL: tokio::process::Child::wait() proactively drops
    // self.stdin before waiting (tokio source comment: "Ensure stdin
    // is closed so the child can't read from it any more"). agentmux-
    // srv has a parent-watch loop on its own stdin — when stdin reads
    // 0 bytes (EOF from a closed write end), it interprets that as
    // "parent died" and shuts itself down. tokio's wait() would
    // trigger that within milliseconds, causing srv to exit before
    // the host even mounts its first browser. Move srv's stdin out
    // of the Child into a launcher-scope binding so tokio can't see
    // it (its take() returns None) and the pipe stays open for the
    // launcher's lifetime. (Smoke test on v0.33.447 caught this.)
    let _srv_stdin_keepalive = srv_child.stdin.take();

    // 4. Spawn host SUSPENDED with srv endpoints in env vars. Host
    // honors AGENTMUX_BACKEND_WS et al. and skips its own spawn_backend.
    // Same CREATE_SUSPENDED → assign-to-job → resume race-fix pattern
    // as PR #570 (codex P2 / gemini HIGH).
    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv_result.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv_result.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv_result.pid.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv_result.auth_key)
        .env("AGENTMUX_INSTANCE_ID", &srv_result.instance_id)
        .env("AGENTMUX_DATA_DIR", paths.data_dir.to_string_lossy().to_string())
        .env(
            "AGENTMUX_CONFIG_DIR",
            paths.config_dir.to_string_lossy().to_string(),
        )
        .env(
            "AGENTMUX_USER_HOME_DIR",
            paths.user_home_dir.to_string_lossy().to_string(),
        )
        .creation_flags(CREATE_SUSPENDED)
        .kill_on_drop(false); // J0 handles cleanup; tokio's kill-on-drop would force-kill on every error path.

    let mut host_child = match host_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("FATAL: failed to spawn CEF host: {}", e));
            eprintln!("Failed to launch AgentMux: {}", e);
            // Happy path: drop(job) → KILL_ON_JOB_CLOSE reaps srv.
            // Degraded path (job is None, J0 absent): kill srv
            // explicitly because kill_on_drop(false) means the Child
            // drop wouldn't terminate it; otherwise srv orphans on
            // launcher exit. (reagent P1 + codex P2 @ main.rs:201,
            // PR #571 round-3.)
            if job.is_none() {
                let _ = srv_child.start_kill();
            }
            drop(job);
            std::process::exit(1);
        }
    };
    let host_pid = host_child.id().unwrap_or(0);
    log(&format!("spawned CEF host pid={} (suspended)", host_pid));

    // 5. Assign host to J0 BEFORE resuming. Without this, host could
    // spawn renderer processes (CEF subprocess) before joining J0 —
    // those renderers would escape KILL_ON_JOB_CLOSE.
    if job.is_some() && host_pid != 0 {
        if let Err(e) = srv_spawner::assign_pid_to_job(host_pid, job_handle) {
            log(&format!(
                "WARN: AssignProcessToJobObject(host) failed: {} — host children may escape job",
                e
            ));
        } else {
            log(&format!(
                "Job Object assigned to host pid={}, KILL_ON_JOB_CLOSE active",
                host_pid
            ));
        }
    }

    // 6. Resume host. With CREATE_SUSPENDED only the main thread
    // exists at this point, so we just need to find it via a
    // Toolhelp32 thread snapshot and ResumeThread it. From here the
    // host runs normally; every CEF render child it spawns inherits J0.
    if let Err(e) = resume_main_thread(host_pid) {
        log(&format!(
            "FATAL: failed to resume host pid={}: {} — terminating",
            host_pid, e
        ));
        // Always kill the suspended host: if J0 is held it'd reap
        // anyway, but in degraded mode (job is None) the suspended
        // host would be a permanent zombie. (PR #570 round-2 pattern.)
        let _ = host_child.start_kill();
        // Same backstop for srv in degraded mode — J0 absent, kill_on
        // _drop(false), so dropping the Child wouldn't terminate it.
        // (reagent P1 @ main.rs:238, PR #571 round-3.)
        if job.is_none() {
            let _ = srv_child.start_kill();
        }
        drop(job);
        std::process::exit(1);
    }

    // 7. Concurrent wait. Whichever child exits first triggers
    // launcher shutdown. tokio::select cancels the other branch's
    // wait future when one fires — both children's borrows are
    // released after the macro returns.
    //
    // We don't manually kill the surviving child in the happy path:
    // dropping `job` below triggers KILL_ON_JOB_CLOSE which reaps
    // the entire J0 membership. The explicit start_kill is only the
    // backstop for the degraded mode (job == None) where J0 doesn't
    // exist to do the reaping. (PR #570 round-2 backstop pattern.)
    log("entering host + srv concurrent wait");
    let exit_code = tokio::select! {
        host_status = host_child.wait() => {
            match host_status {
                Ok(s) => {
                    let code = s.code().unwrap_or(1);
                    log(&format!("CEF host exited with code {}", code));
                    code
                }
                Err(e) => {
                    log(&format!("FATAL: host wait failed: {}", e));
                    1
                }
            }
        }
        srv_status = srv_child.wait() => {
            match srv_status {
                Ok(s) => {
                    let code = s.code().unwrap_or(1);
                    log(&format!(
                        "srv exited UNEXPECTEDLY (host still running) with code {} — terminating launcher",
                        code
                    ));
                    1
                }
                Err(e) => {
                    log(&format!("FATAL: srv wait failed: {}", e));
                    1
                }
            }
        }
    };

    // 8. Cleanup. Happy path: drop(job) → KILL_ON_JOB_CLOSE reaps
    // the surviving child + CEF renderers. Degraded path (job is
    // None): explicit start_kill on both — neither will be reaped
    // by the OS, so we have to terminate them ourselves to avoid
    // orphans. (gemini PR #570 round-1 MEDIUM L105 / round-2 P1
    // backstop pattern.)
    if job.is_none() {
        log("WARN: J0 absent — explicitly killing surviving children");
        let _ = host_child.start_kill();
        let _ = srv_child.start_kill();
    }
    drop(job);
    log(&format!("launcher exiting with code {}", exit_code));
    std::process::exit(exit_code);
}

/// Append a timestamped line to ~/.agentmux/logs/agentmux-launcher.log.
/// Best-effort — silently no-ops if the log dir doesn't exist yet.
pub(crate) fn log(msg: &str) {
    let log_dir = dirs_fallback_home().join(".agentmux").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("agentmux-launcher.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] v{} {}", secs, env!("CARGO_PKG_VERSION"), msg);
    }
}

/// Home dir without depending on `dirs` for THIS specific lookup.
/// Kept to avoid a dirs dep cycle from log() — log() is called from
/// data_dir::resolve_paths via failure paths, and we want it to work
/// even if `dirs` itself is mid-failure.
fn dirs_fallback_home() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Owns a Windows Job Object handle. CloseHandle on drop. The job's
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` flag means closing the last handle
/// terminates every assigned process — which is what we want as a backstop
/// if this launcher dies abruptly.
#[cfg(target_os = "windows")]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
unsafe impl Send for JobHandle {}

#[cfg(target_os = "windows")]
impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

/// Create a Job Object J0 with `KILL_ON_JOB_CLOSE`. Caller assigns
/// processes to it via `srv_spawner::assign_pid_to_job(pid, job)`.
#[cfg(target_os = "windows")]
fn create_job_object() -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("CreateJobObjectW returned null".into());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            return Err("SetInformationJobObject failed".into());
        }
        Ok(job)
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

/// Find the CEF host binary in the runtime directory.
/// Tries versioned name first (agentmux-X.Y.Z.exe), then the old
/// agentmux-cef-X.Y.Z.exe pattern for backwards compat, then plain
/// agentmux-cef.exe (dev mode).
fn find_cef_binary(runtime_dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };

    let versioned = format!("agentmux-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_path = runtime_dir.join(&versioned);
    if versioned_path.exists() {
        return versioned_path;
    }

    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let prefix = "agentmux-";
        let cef_prefix = "agentmux-cef";
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix)
                && !name.starts_with(cef_prefix)
                && !name.starts_with("agentmux-srv")
                && name.ends_with(ext)
            {
                return entry.path();
            }
        }
    }

    let versioned_old = format!("agentmux-cef-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_old_path = runtime_dir.join(&versioned_old);
    if versioned_old_path.exists() {
        return versioned_old_path;
    }

    runtime_dir.join(format!("agentmux-cef{}", ext))
}
