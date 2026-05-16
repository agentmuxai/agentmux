// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Parent-process identity check used by the launcher-IPC connection
//! guard in `main.rs`. Returns true when the host's parent process is
//! the AgentMux launcher.
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

/// Exe filenames we accept as "the AgentMux launcher." Compared
/// case-insensitively after stripping the `.exe` extension.
///
/// - `agentmux-launcher` — the Cargo bin name. Used directly in dev
///   (`task dev` copies `target/release/agentmux-launcher.exe` into
///   `dist/cef-dev/`).
/// - `agentmux` — the user-facing name in portable / installed builds.
///   `scripts/package-portable.sh` copies the launcher to
///   `agentmux.exe` so the icon the user double-clicks reads as
///   "AgentMux", not "AgentMux Launcher." `PROCESSENTRY32W.szExeFile`
///   returns the on-disk file name, so the parent stem from a
///   production launch is `agentmux`, not `agentmux-launcher` —
///   codex P1 on PR #882 round 1 caught this would regress every
///   portable build.
const ACCEPTED_PARENT_STEMS: &[&str] = &["agentmux-launcher", "agentmux"];

/// Returns `Some(true)` if the host's parent process is the AgentMux
/// launcher (under any of its on-disk names), `Some(false)` if it's
/// something else, or `None` if the parent identity couldn't be
/// determined (snapshot creation failed, parent process exited
/// between PID discovery and lookup). Callers treat `None` as "fall
/// through to the path-based guard" — see the call site.
#[cfg(target_os = "windows")]
pub fn parent_is_agentmux_launcher() -> Option<bool> {
    let parent_exe = parent_exe_file_windows()?;
    let stem = parent_exe
        .strip_suffix(".exe")
        .or_else(|| parent_exe.strip_suffix(".EXE"))
        .unwrap_or(&parent_exe);
    Some(
        ACCEPTED_PARENT_STEMS
            .iter()
            .any(|accepted| stem.eq_ignore_ascii_case(accepted)),
    )
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

/// Walk the Toolhelp32 process snapshot in a single pass to:
///   1. Find the current PID's entry → record `th32ParentProcessID`.
///   2. Find the entry where `th32ProcessID == parent_pid` → capture
///      its `szExeFile`.
///
/// Uses `PROCESSENTRY32W.szExeFile` (a fixed 260-wide-char buffer of
/// the executable's *filename only*, no path) rather than
/// `QueryFullProcessImageNameW`. Per codex P2 on PR #882 round 2,
/// the latter could fail when a Windows checkout's staged launcher
/// path exceeds MAX_PATH — `szExeFile` is filename-only and never
/// hits that limit.
#[cfg(target_os = "windows")]
fn parent_exe_file_windows() -> Option<String> {
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

        // First pass: find current PID's parent PID.
        let mut parent_pid: Option<u32> = None;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            if entry.th32ProcessID == me {
                parent_pid = Some(entry.th32ParentProcessID);
                break;
            }
            ok = Process32NextW(snap, &mut entry);
        }

        let parent_pid = match parent_pid {
            Some(p) => p,
            None => {
                CloseHandle(snap);
                return None;
            }
        };

        // Second pass: re-snapshot to walk from start. CreateToolhelp32Snapshot's
        // cursor isn't documented as rewindable, so the safest approach is a
        // fresh snapshot rather than relying on iterator state after `break`.
        CloseHandle(snap);
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut parent_exe: Option<String> = None;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            if entry.th32ProcessID == parent_pid {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                parent_exe = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                break;
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
        parent_exe
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
            // vstest are never named "agentmux-launcher" or "agentmux").
            if let Some(b) = result {
                assert!(!b, "parent should not be the AgentMux launcher under test");
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(result.is_none(), "non-windows always returns None");
        }
    }
}
