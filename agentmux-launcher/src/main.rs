// AgentMux Launcher — Sets DLL search path then spawns the real CEF binary.
//
// This tiny exe lives at the top of the portable directory. It:
// 1. Adds runtime/ to the DLL search path (so libcef.dll is found)
// 2. Spawns runtime/agentmux-<version>.exe with the same arguments
// 3. Waits for it to exit and forwards the exit code
//
// This is needed because libcef.dll is a load-time dependency of the CEF
// host — the OS loader needs it before main() runs, so SetDllDirectoryW
// in the CEF host's main() would be too late.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    let exe_path = std::env::current_exe().expect("cannot resolve exe path");
    let exe_dir = exe_path.parent().expect("exe has no parent directory");
    let runtime_dir = exe_dir.join("runtime");

    log(&format!("starting — exe={} runtime={}", exe_path.display(), runtime_dir.display()));

    // Set DLL search path so libcef.dll (in runtime/) is found by the child process
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

    // Resolve the real CEF host binary in runtime/.
    let real_exe = find_cef_binary(&runtime_dir);
    log(&format!("resolved CEF binary: {}", real_exe.display()));

    if !real_exe.exists() {
        log(&format!("FATAL: CEF binary not found at {}", real_exe.display()));
        eprintln!(
            "AgentMux runtime not found in: {}\nMake sure the runtime/ folder is intact.",
            runtime_dir.display()
        );
        std::process::exit(1);
    }

    // Forward all CLI arguments
    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&format!("spawning CEF host with {} args", args.len()));

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        // Spawn the host SUSPENDED so its main thread can't run — and
        // therefore can't spawn the srv sidecar or CEF render-process
        // children — before we've assigned it to the Job Object.
        // Without CREATE_SUSPENDED there's a real race where the host
        // forks children in the gap between Command::spawn and
        // AssignProcessToJobObject; those children would escape the
        // job's KILL_ON_JOB_CLOSE backstop. (Microsoft / Raymond Chen
        // pattern; codex P2 + gemini HIGH on PR #570 round-1.)
        let child = std::process::Command::new(&real_exe)
            .args(&args)
            .creation_flags(CREATE_SUSPENDED)
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                log(&format!("FATAL: failed to spawn CEF host: {}", e));
                eprintln!("Failed to launch AgentMux: {}", e);
                std::process::exit(1);
            }
        };

        let host_pid = child.id();
        log(&format!("spawned CEF host pid={} (suspended)", host_pid));

        // Create a Job Object with KILL_ON_JOB_CLOSE and assign the
        // suspended host to it. CEF render-process workers and the srv
        // sidecar that the host will spawn AFTER resume inherit the job
        // automatically. When this launcher process exits — for any
        // reason, including hard-kill via Task Manager — the OS closes
        // the job handle and the entire process tree is reaped.
        //
        // If creation/assignment fails, log + still resume the host
        // (degraded cleanup is better than a dead suspended host).
        let job = match create_job_object_for_child(host_pid) {
            Ok(handle) => {
                log(&format!(
                    "Job Object assigned to pid={}, KILL_ON_JOB_CLOSE active",
                    host_pid
                ));
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

        // Now resume the host. With CREATE_SUSPENDED only the main
        // thread exists at this point, so we just need to find it via
        // a Toolhelp32 thread snapshot and ResumeThread it. From here
        // the host runs normally, but every child it spawns will
        // inherit the job we just attached.
        if let Err(e) = resume_main_thread(host_pid) {
            log(&format!(
                "FATAL: failed to resume host pid={}: {} — terminating",
                host_pid, e
            ));
            // The host is currently suspended and will never run.
            // If Job Object setup succeeded, dropping it reaps the
            // tree via KILL_ON_JOB_CLOSE. If the job is None (creation
            // failed earlier with the WARN log), drop is a no-op and
            // the suspended host would survive as a permanent zombie
            // holding resources and blocking subsequent launches —
            // explicitly child.kill() it as a backstop.
            // (reagent P1 + codex P2 + gemini MEDIUM, PR #570 round-2)
            let _ = child.kill();
            drop(job);
            std::process::exit(1);
        }

        match child.wait() {
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                log(&format!("CEF host exited with code {}", code));
                // Drop the job handle. KILL_ON_JOB_CLOSE then reaps
                // any child the host left behind (srv, CEF render
                // workers) — it's the backstop for unclean host
                // exits, not a no-op. On a clean host exit those
                // children typically already exited via their own
                // job/parent-watcher chains; this guarantees they
                // don't survive even when those mechanisms didn't
                // fire. (gemini PR #570 round-1 MEDIUM L105.)
                drop(job);
                std::process::exit(code);
            }
            Err(e) => {
                log(&format!("FATAL: wait failed: {}", e));
                // Same backstop as the resume-failure path: if the job
                // is None (creation failed), we must explicitly kill
                // the host to avoid leaving an orphan running with no
                // cleanup signal. (gemini MEDIUM @ L148, PR #570
                // round-2.)
                let _ = child.kill();
                drop(job);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        log("exec into CEF host (Unix)");
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&real_exe).args(&args).exec();
        log(&format!("FATAL: exec failed: {}", err));
        eprintln!("Failed to launch AgentMux: {}", err);
        std::process::exit(1);
    }
}

/// Append a timestamped line to ~/.agentmux/logs/agentmux-launcher.log.
/// Best-effort — silently no-ops if the log dir doesn't exist yet.
fn log(msg: &str) {
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

/// Home dir without the `dirs` crate (keep launcher zero-dep beyond windows-sys).
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

/// Create a Job Object with `KILL_ON_JOB_CLOSE` and assign the given PID.
/// Mirrors `agentmux-cef::sidecar::create_job_object_for_child`; lifted to
/// the launcher so the entire host process tree (host + srv + CEF render
/// children) is wrapped one level higher.
#[cfg(target_os = "windows")]
fn create_job_object_for_child(
    pid: u32,
) -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("Failed to create job object".into());
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
            return Err("Failed to set job object info".into());
        }

        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            CloseHandle(job);
            return Err(format!("Failed to open process {}", pid));
        }

        let ok = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if ok == 0 {
            CloseHandle(job);
            return Err("Failed to assign process to job object".into());
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
fn resume_main_thread(pid: u32) -> Result<(), String> {
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
                // Reset dwSize before each Thread32Next per Win32 contract.
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

    // 1. Try exact versioned name matching this launcher's version (new naming)
    let versioned = format!("agentmux-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_path = runtime_dir.join(&versioned);
    if versioned_path.exists() {
        return versioned_path;
    }

    // 2. Scan for any agentmux-<version>.exe — handles minor version mismatch
    //    between launcher and CEF host (new naming, no "cef" in artifact name)
    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let prefix = "agentmux-";
        let cef_prefix = "agentmux-cef";
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Match agentmux-<version>.exe but not agentmux-cef*.exe or agentmux-srv*.exe
            if name.starts_with(prefix)
                && !name.starts_with(cef_prefix)
                && !name.starts_with("agentmux-srv")
                && name.ends_with(ext)
            {
                return entry.path();
            }
        }
    }

    // 3. Backwards compat: old naming used agentmux-cef-<version>.exe
    let versioned_old = format!("agentmux-cef-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_old_path = runtime_dir.join(&versioned_old);
    if versioned_old_path.exists() {
        return versioned_old_path;
    }

    // 4. Fall back to plain name (dev mode)
    runtime_dir.join(format!("agentmux-cef{}", ext))
}
