// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Quit state (H.5) ─────────────────────────────────────────────────────

/// Host process quit lifecycle. Replaces `is_quitting: AtomicBool` at
/// `state.rs::AppState`. Three states; transitions are monotonic
/// (Running → Draining → Quit, no regression).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QuitState {
    /// Normal operation. All commands accepted (subject to per-arm rules).
    Running,
    /// `BeginDrain` dispatched. Pool refills suppressed; awaiting pool +
    /// browsers to drain.
    Draining { reason: QuitReason },
    /// `ConfirmDrained` dispatched. Host quitting; no further commands.
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QuitReason {
    /// User closed the last user-visible top-level window. Standard exit.
    LastWindowClosed,
    /// Launcher signaled HostShouldQuit (cross-process shutdown).
    LauncherRequested,
    /// External force-quit (Win32 WM_QUIT, signal, etc.).
    External,
}

impl Default for QuitState {
    fn default() -> Self { QuitState::Running }
}
