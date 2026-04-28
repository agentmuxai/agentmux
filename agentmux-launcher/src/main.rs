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
        // Spawn the host and capture the Child handle. We use spawn()+wait()
        // (instead of status()) so we can grab the host's PID and assign it
        // to a Job Object before it has a chance to spawn its own children
        // (the agentmux-srv sidecar and CEF render-process workers).
        let child = std::process::Command::new(&real_exe).args(&args).spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                log(&format!("FATAL: failed to spawn CEF host: {}", e));
                eprintln!("Failed to launch AgentMux: {}", e);
                std::process::exit(1);
            }
        };

        let host_pid = child.id();
        log(&format!("spawned CEF host pid={}", host_pid));

        // Create a Job Object with KILL_ON_JOB_CLOSE and assign the host to
        // it. CEF render-process workers and the srv sidecar that the host
        // spawns will inherit the job. When this launcher process exits —
        // for any reason, including being killed via Task Manager — the OS
        // closes the job handle and the entire process tree is reaped.
        //
        // If creation/assignment fails, log and continue: pre-Job-Object
        // behavior is the fallback, no functional regression.
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

        match child.wait() {
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                log(&format!("CEF host exited with code {}", code));
                drop(job); // close job handle; host already exited so KILL_ON_JOB_CLOSE is a no-op
                std::process::exit(code);
            }
            Err(e) => {
                log(&format!("FATAL: wait failed: {}", e));
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
struct JobHandle(*mut std::ffi::c_void);

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
fn create_job_object_for_child(pid: u32) -> Result<*mut std::ffi::c_void, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    // CreateJobObjectW is not exported by windows-sys 0.59's JobObjects
    // feature, so we link to kernel32.dll directly.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(
            lpjobattributes: *const std::ffi::c_void,
            lpname: *const u16,
        ) -> *mut std::ffi::c_void;
    }

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
