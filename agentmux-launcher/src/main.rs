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
        let status = std::process::Command::new(&real_exe)
            .args(&args)
            .status();

        match status {
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                log(&format!("CEF host exited with code {}", code));
                std::process::exit(code);
            }
            Err(e) => {
                log(&format!("FATAL: failed to spawn CEF host: {}", e));
                eprintln!("Failed to launch AgentMux: {}", e);
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
