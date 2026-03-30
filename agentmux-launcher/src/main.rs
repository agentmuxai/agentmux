// AgentMux Launcher — Sets DLL search path then spawns the real CEF binary.
//
// This tiny exe lives at the top of the portable directory. It:
// 1. Adds runtime/ to the DLL search path (so libcef.dll is found)
// 2. Spawns runtime/agentmux-cef.exe with the same arguments
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

    // Resolve the real CEF host binary in runtime/
    let real_exe = runtime_dir.join(if cfg!(target_os = "windows") {
        "agentmux-cef.exe"
    } else {
        "agentmux-cef"
    });

    if !real_exe.exists() {
        eprintln!(
            "AgentMux runtime not found at: {}\nMake sure the runtime/ folder is intact.",
            real_exe.display()
        );
        std::process::exit(1);
    }

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
