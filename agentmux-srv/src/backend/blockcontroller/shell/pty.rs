// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PTY geometry resolution and platform-specific shell detection.

use portable_pty::PtySize;

use super::controller::ShellController;
use crate::backend::obj::RuntimeOpts;

/// PTY read buffer size (matches Go's 4096).
pub(super) const PTY_READ_BUF_SIZE: usize = 4096;

/// How long the PTY output flusher waits, after the first chunk of a new
/// batch, for more chunks to arrive before broadcasting — see
/// `docs/reports/REPORT_RENDERER_CPU_UNBATCHED_PTY_OUTPUT_2026_09_11.md`.
/// A fast producer (a build, a verbose test run, a busy agent) delivers
/// output across many separate `read()` returns in quick succession; without
/// this window, each one fired its own file write + WebSocket broadcast +
/// OSC/translation pass, and the aggregate IPC/wakeup rate was measurably
/// costing real CPU in the renderer and GPU-helper processes. 20ms is well
/// under human-perceptible latency (and under round-trip costs already
/// elsewhere in this pipeline), so a single short burst — a keystroke echo,
/// a one-line response — still appears with no felt delay.
pub(super) const PTY_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(20);

/// Hard cap on how many bytes one coalesced batch accumulates before it is
/// flushed regardless of the time window — bounds both broadcast latency and
/// memory during a sustained, very fast burst (e.g. `yes` piped through a
/// shell pane). 64x a single PTY_READ_BUF_SIZE read: generous headroom for
/// real bursts (a build's stdout in a 20ms window) while still bounded.
pub(super) const PTY_COALESCE_MAX_BYTES: usize = 64 * PTY_READ_BUF_SIZE;

/// Capacity of the channel handing PTY-read chunks from the blocking read
/// loop to the async coalescing flusher (reagentx P1 on PR #3206).
/// `PTY_COALESCE_MAX_BYTES` only bounds the batch currently being
/// assembled — it does nothing about chunks still sitting in the channel
/// if the flusher falls behind the read rate (slow disk, lock contention,
/// several busy panes at once). An unbounded channel there would let a
/// sustained fast producer (`yes`, a large `cat`) grow queued memory
/// without limit. 128 chunks x up to ~PTY_READ_BUF_SIZE each is ~512KiB of
/// worst-case queued-but-not-yet-batched data per pane — real headroom
/// above one coalesced batch, but strictly bounded. The read loop sends
/// via `blocking_send`, which blocks the calling (already-blocking-pool)
/// thread once the channel is full, propagating real backpressure to the
/// PTY read rate — and from there, via the kernel's own PTY buffer, to the
/// child process itself, the same way a slow terminal reader naturally
/// backpressures a fast writer.
pub(super) const PTY_CHANNEL_CAPACITY: usize = 128;

/// Bound on how long the wait task waits for the coalescing flusher to
/// drain trailing output after the direct child has already been reaped
/// (codex P1 on PR #3206, third round). Normally resolves almost
/// immediately: the read loop observes real PTY EOF once the direct child
/// exits and drops `pty_tx`, closing the flusher's channel. But EOF depends
/// on every process holding the PTY slave fd closing it, not just the
/// direct child — a background descendant that inherited the fd and
/// ignores SIGHUP can keep it open indefinitely, in which case the flusher
/// would never see its channel close and its `JoinHandle` would never
/// resolve on its own. An unbounded await here would then hang child
/// reaping's own downstream cleanup (STATUS_DONE, run_lock release)
/// forever, for a case that must never block them. 10s mirrors
/// `persistent.rs`'s identical stdout-reader bound for the same
/// descendant-held-descriptor scenario: generous enough that normal
/// flushing (bounded by `PTY_COALESCE_WINDOW`, milliseconds) never trips
/// it, but a hard ceiling so a genuinely stuck descendant can't hang pane
/// teardown. Unlike `persistent.rs`'s bound, expiry does NOT abort the
/// flusher (reagentx P1 on PR #3206, same round): there, one combined async
/// reader task both reads and processes, so aborting it genuinely stops the
/// read. Here reading (a separate `spawn_blocking` doing a raw, blocking,
/// un-cancellable OS `read()`) and flushing are two different tasks —
/// aborting only the flusher can't reclaim the read loop's thread either
/// way, and would additionally make every later `blocking_send` find the
/// receiver gone and silently drop it, permanently losing any further
/// output from a still-live descendant. Letting the `JoinHandle` simply
/// drop on timeout detaches the flusher (and transitively its read loop)
/// to keep running independently in the background instead, so nothing
/// produced after teardown gives up waiting is silently lost.
pub(super) const FLUSHER_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
