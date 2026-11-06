// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Microsoft Store install asks Windows to open AgentMux again after the Store updates it.
//! The Store closes a packaged app to swap its package and doesn't start it again by itself;
//! an app that called `RegisterApplicationRestart` is opened again by Windows.
//!
//! It is registered here, in the launcher (`agentmux.exe`, the package's entry point), and not
//! in the CEF host, which the launcher spawns: Windows reopens the registered process, so the
//! launcher brings the tray, srv and the host back together, where a registered host would come
//! back alone. The registration is for an update only, never after a crash, a hang or a reboot.
//! The host writes the marker that says how the install ended (`agentmux-cef`,
//! `commands/store_restart.rs`).
//!
//! NOT run against a packaged install by whoever wrote it: whether a path is a Store install's is
//! unit-tested, the Windows call is type-checked only.

use std::path::Path;

/// Whether `exe` runs from a Store package's folder (`C:\Program Files\WindowsApps\<package>\...`).
pub(crate) fn is_packaged(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps"))
}

/// Register this process to be opened again after an update, when it is a Store install.
pub(crate) fn register_if_packaged() {
    let Ok(exe) = std::env::current_exe() else { return };
    if !is_packaged(&exe) {
        return;
    }
    use windows_sys::Win32::System::Recovery::{RegisterApplicationRestart, RESTART_NO_CRASH, RESTART_NO_HANG, RESTART_NO_REBOOT};
    // No command line: the launcher opens as a double-click does.
    let hr = unsafe { RegisterApplicationRestart(std::ptr::null(), RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT) };
    if hr == 0 {
        crate::logging::log("store update: registered to be opened again after an update");
    } else {
        crate::logging::log(&format!("store update: could not register to be opened again after an update (hresult {hr:#x})"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_in_the_packages_folder_is_a_store_install() {
        assert!(is_packaged(Path::new(r"C:\Program Files\WindowsApps\AgentMux_0.60.16.0_x64__abc\agentmux.exe")));
        assert!(is_packaged(Path::new(r"c:\program files\windowsapps\AgentMux_1_x64__abc\agentmux.exe")));
    }

    #[test]
    fn other_installs_are_not() {
        assert!(!is_packaged(Path::new(r"C:\Users\u\AppData\Local\Programs\AgentMux\agentmux.exe")));
        assert!(!is_packaged(Path::new(r"C:\Program Files\AgentMux\agentmux.exe")));
        // A folder merely containing the word is not the packages folder.
        assert!(!is_packaged(Path::new(r"C:\tools\MyWindowsApps2\agentmux.exe")));
    }
}
