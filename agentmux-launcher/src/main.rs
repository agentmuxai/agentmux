// AgentMux Launcher — Sets DLL search path then spawns the real CEF binary.
//
// This tiny exe lives at the top of the portable directory. It:
// 1. Adds runtime/ to the DLL search path (so libcef.dll is found)
// 2. Discovers and spawns runtime/agentmux-cef-{VERSION}.exe with the same arguments
// 3. Waits for it to exit and forwards the exit code
//
// This is needed because libcef.dll is a load-time dependency of the CEF
// host — the OS loader needs it before main() runs, so SetDllDirectoryW
// in the CEF host's main() would be too late.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

/// Find the versioned agentmux-cef-* binary in the given directory.
/// Hard fails with a directory listing if not found.
fn find_cef_binary(runtime_dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("agentmux-cef-") && name_str.ends_with(ext) {
                return entry.path();
            }
        }
    }
    // Hard fail with diagnostic
    eprintln!("FATAL: No agentmux-cef-VERSION{} found in: {}", ext, runtime_dir.display());
    eprintln!("Contents of runtime/:");
    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        for entry in entries.flatten() {
            eprintln!("  {}", entry.file_name().to_string_lossy());
        }
    } else {
        eprintln!("  (directory not found or not readable)");
    }
    std::process::exit(1);
}

fn main() {
    let exe_path = std::env::current_exe().expect("cannot resolve exe path");
    let exe_dir = exe_path.parent().expect("exe has no parent directory");
    let runtime_dir = exe_dir.join("runtime");

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

    // Discover the versioned CEF host binary in runtime/
    let real_exe = find_cef_binary(&runtime_dir);

    // Forward all CLI arguments
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(target_os = "windows")]
    {
        // Spawn the CEF host and wait for it to exit
        let status = std::process::Command::new(&real_exe)
            .args(&args)
            .status();

        match status {
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("Failed to launch AgentMux: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // On Unix, exec replaces this process entirely
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&real_exe).args(&args).exec();
        eprintln!("Failed to launch AgentMux: {}", err);
        std::process::exit(1);
    }
}
