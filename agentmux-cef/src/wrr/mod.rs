// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — Window Reality Reconciliation (WRR), host-side
// hook layer.
//
// Subscribes to Win32 events that surface HWND lifecycle and
// observability transitions, and forwards each as a typed
// `Command::ReportHwnd*` over the existing launcher IPC pipe.
// The launcher's reducer arm classifies divergences and emits
// `Event::HwndDriftDetected` (see
// `agentmux-launcher/src/wrr/mod.rs`).
//
// Pure event-driven: every report is in response to an OS
// notification, never on a timer. The one heartbeat-shaped
// caveat — position-change debounce — is purely a wire-volume
// optimization (drag a window to its final position; the
// reducer only needs the final rect, not 60 intermediate ones).
// Debounce IS bounded; a final position event always lands
// after the burst settles (see `position_debounce.rs`).
//
// Design lives at `docs/retro/wrr-design-2026-04-28.md`.

#[cfg(target_os = "windows")]
pub mod classify;
#[cfg(target_os = "windows")]
pub mod position_debounce;
#[cfg(target_os = "windows")]
pub mod win_event;

#[cfg(target_os = "windows")]
pub use win_event::{install_hooks, uninstall_hooks};

#[cfg(not(target_os = "windows"))]
pub fn install_hooks() {
    // WRR is Windows-only — Phase 7 will revisit when cross-platform
    // window-state mirroring lands.
}

#[cfg(not(target_os = "windows"))]
pub fn uninstall_hooks() {}
