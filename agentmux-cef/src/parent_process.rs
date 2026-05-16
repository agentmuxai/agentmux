// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Parent-process identity check used by the launcher-IPC connection
//! guard in `main.rs`. Returns true when the host's parent process is
//! `agentmux-launcher.exe`.
//!
//! Background — see `docs/specs/SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16.md`.
//! Before this helper, the connect-to-launcher gate used
//! `is_dev_build_exe(exe_dir)` as a proxy for "the launcher is not
//! running, skip IPC". That worked when `task dev` invoked the host
//! directly. After `SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md` made
//! `task dev` spawn the host via the launcher (production-parallel
//! layout), the path-based guard wrongly skipped legitimate IPC in dev,
//! breaking `WindowOpened` / `BackendWindowIdRegistered` event delivery
//! (visible as: status-bar window-count desync and missing opacity
//! slider in dev).
//!
//! Parent-process check is a tighter discriminator: it admits the dev
//! launcher (correct) and still rejects a standalone dev host that
//! happened to inherit `AGENTMUX_LAUNCHER_PIPE` from a parent shell
//! (also correct — that's the original isolation concern).

/// The expected parent exe stem when the host is spawned by the
/// launcher. Compared case-insensitively against the OS reading.
const EXPECTED_PARENT_STEM: &str = "agentmux-launcher";

/// Returns `Some(true)` if the host's parent process is the AgentMux
/// launcher, `Some(false)` if it's something else, or `None` if the
/// parent identity couldn't be determined (process exited, permission
/// denied, snapshot iteration failed). Callers should treat `None` as
/// "fall through to the path-based guard" — see the call site.
#[cfg(target_os = "windows")]
pub fn parent_is_agentmux_launcher() -> Option<bool> {
    let parent_pid = parent_pid_windows()?;
    let parent_name = process_image_stem_windows(parent_pid)?;
    Some(parent_name.eq_ignore_ascii_case(EXPECTED_PARENT_STEM))
}

#[cfg(not(target_os = "windows"))]
pub fn parent_is_agentmux_launcher() -> Option<bool> {
    // Linux/macOS launcher integration is on a separate roadmap
    // (Phase 7 cross-platform parity per
    // SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md). On those
    // platforms the host is invoked directly by `task dev` and IPC
    // is not in play, so the parent-check is moot — return None and
    // let the path-based guard decide.
    None
}

#[cfg(target_os = "windows")]
fn parent_pid_windows() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    // SAFETY: Toolhelp32 snapshot APIs are documented to be safe to
    // call from any thread; we close the returned handle on every
    // exit path. PROCESSENTRY32W is initialized with its size field
    // before the first call as the Win32 API requires.
    unsafe {
        let me = GetCurrentProcessId();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry);
        let mut parent: Option<u32> = None;
        while ok != 0 {
            if entry.th32ProcessID == me {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
        parent
    }
}

#[cfg(target_os = "windows")]
fn process_image_stem_windows(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess returns a handle we close on every exit
    // path. The path buffer is sized at MAX_PATH (260) wide chars
    // and we pass the buffer length to QueryFullProcessImageNameW;
    // the API writes the actual length back into the same arg.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 260];
        let mut len: u32 = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let path = std::path::PathBuf::from(path);
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: on Windows the helper should return Some(_) for the
    /// test runner's parent (cargo / vstest). On other platforms it
    /// returns None by construction.
    #[test]
    fn parent_check_resolves() {
        let result = parent_is_agentmux_launcher();
        #[cfg(target_os = "windows")]
        {
            // Some platforms / CI runners may fail the snapshot under
            // restricted permissions; we accept None there. What we DO
            // assert: when it returns Some, it must be false (cargo /
            // vstest are never named "agentmux-launcher").
            if let Some(b) = result {
                assert!(!b, "parent should not be agentmux-launcher under test");
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(result.is_none(), "non-windows always returns None");
        }
    }
}
