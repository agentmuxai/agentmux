// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PTY geometry resolution and platform-specific shell detection.

use portable_pty::PtySize;

use super::controller::ShellController;
use crate::backend::obj::RuntimeOpts;

/// PTY read buffer size (matches Go's 4096).
pub(super) const PTY_READ_BUF_SIZE: usize = 4096;

/// Detect the best available interactive shell on Windows.
///
/// Mirrors the original Go logic from pkg/util/shellutil/shellutil.go DetectLocalShellPath():
///   1. Try `pwsh`  (PowerShell 7 — cross-platform)
///   2. Try `powershell` (Windows PowerShell 5.x)
///   3. Fall back to `cmd.exe`
#[cfg(windows)]
pub(super) fn detect_local_shell_path_windows() -> String {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use agentmux_common::win32::CREATE_NO_WINDOW;
    // Try pwsh (PowerShell 7)
    if Command::new("where")
        .arg("pwsh")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "pwsh".to_string();
    }
    // Try powershell (Windows PowerShell 5.x)
    if Command::new("where")
        .arg("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "powershell".to_string();
    }
    "cmd.exe".to_string()
}

/// Stub for non-Windows builds (never called due to cfg!(windows) guard).
#[cfg(not(windows))]
pub(super) fn detect_local_shell_path_windows() -> String {
    "cmd.exe".to_string()
}

/// Resolve the initial PTY geometry from the resync `rt_opts` payload.
///
/// The agent pane is a custom UI (not xterm.js), so the PTY never receives
/// a `fitAddon` resize and must be born at the right width — otherwise the
/// first batch of agent/tool output wraps at the fallback width until a
/// post-spawn resize RPC lands, and that RPC races controller startup (it
/// can fail outright). The frontend computes cols from the pane and passes
/// them as `rtopts.termsize` on the `controllerresync` command (see
/// `usePtyWidth.ts` / `launch-flow.ts`).
///
/// Falls back to the historical 25x200 default when `rt_opts` is absent,
/// unparseable, or carries the serde-default termsize (`rows==0 && cols==0`,
/// per `obj::is_default_term_size`). Per-field guards let a cols-only
/// payload keep the default row count. Each axis is clamped to `[1, 1000]`
/// so the `i64 → u16` cast is lossless and a bogus value cannot open a
/// zero-size or wrapped-size PTY.
/// See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
pub(super) fn pty_size_from_rt_opts(rt_opts: &Option<serde_json::Value>) -> PtySize {
    // Historical fallback geometry. Cols 200 keeps the agent-pane live-log
    // from hard-wrapping at ~80 before the dynamic resize lands.
    const DEFAULT_PTY_ROWS: u16 = 25;
    const DEFAULT_PTY_COLS: u16 = 200;
    let (mut rows, mut cols) = (DEFAULT_PTY_ROWS, DEFAULT_PTY_COLS);
    if let Some(v) = rt_opts {
        if let Ok(rt) = serde_json::from_value::<RuntimeOpts>(v.clone()) {
            let ts = &rt.termsize;
            // rows==0 && cols==0 is the serde default → treat as absent.
            if !(ts.rows == 0 && ts.cols == 0) {
                if ts.cols > 0 {
                    cols = ts.cols.clamp(1, 1000) as u16;
                }
                if ts.rows > 0 {
                    rows = ts.rows.clamp(1, 1000) as u16;
                }
            }
        }
    }
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl ShellController {
    /// Inherent-method wrapper over [`pty_size_from_rt_opts`], preserving the
    /// pre-split `ShellController::pty_size_from_rt_opts(...)` call site used by
    /// `start()` and the unit tests.
    pub(super) fn pty_size_from_rt_opts(rt_opts: &Option<serde_json::Value>) -> PtySize {
        pty_size_from_rt_opts(rt_opts)
    }
}
