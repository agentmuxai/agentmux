// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The small shapes the commands and events share: rectangles, window kinds and snapshots,
//! and the enums that name a phase, a severity, a kind of drift or an error.

use serde::{Deserialize, Serialize};

/// Phase B.9.1 — rectangle in Win32 screen coordinates (pixels).
/// Matches Windows' `RECT` semantics: `right` and `bottom` are one
/// past the last included pixel, so `right - left == width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Window role in the multi-window model — the single definition, used by
/// the host (`agentmux-cef::state` re-exports it) and deserialized by the
/// launcher. The host used to carry a byte-identical private copy and map
/// between the two variant by variant; that copy is gone
/// (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// Independent AgentMux window. Appears in the Windows taskbar.
    FullInstance,
    /// Hidden from the taskbar; closes when its parent FullInstance closes.
    Subwindow,
}

/// Phase D.1 — serializable view of one window in the launcher
/// reducer's canonical state. Maps 1:1 to the launcher's internal
/// `WindowMirror` minus `opened_at` (which is launcher-local clock
/// data not meaningful to subscribers).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub label: String,
    pub kind: WindowKind,
    pub parent_label: Option<String>,
    pub hwnd: Option<u64>,
    pub visible: bool,
    pub iconic: bool,
    pub last_rect: Option<Rect>,
    pub foregrounded_since_open: bool,
}

/// Phase B.9.1 — six classes of CEF↔Win32 disagreement the reducer
/// can detect at event-dispatch time. See the WRR design doc for
/// the per-kind triggering Command and reducer action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HwndDriftKind {
    /// `state.windows` has a label whose `hwnd` field never got
    /// populated, AND a follow-up event arrived that should have
    /// reconciled it. CEF says open, Win32 has no matching HWND.
    BrowserWithoutHwnd,
    /// HWND in the host's report doesn't map to any
    /// `state.windows` / `state.pool` label. Win32 has it, CEF
    /// didn't open it. The "stray taskbar" case.
    HwndWithoutBrowser,
    /// HWND for a known label was never SHOWN (no foreground
    /// event) since open and a subsequent state transition has
    /// elapsed. User can't see it.
    HiddenSinceOpen,
    /// Window rect doesn't intersect any monitor in
    /// `state.monitors`. Off-screen orphan.
    OffMonitor,
    /// `ReportHwndDestroyed` arrived without a preceding
    /// `ReportWindowClosed` for the matching label. Renderer
    /// crashed, took the HWND with it.
    OrphanDestroy,
    /// `ReportWindowClosed` arrived, but subsequent OS events for
    /// the HWND keep firing (it never went away on the Win32 side).
    LingeringHwnd,
    /// Phase B.9.3 — host process is alive and registered, but
    /// `state.windows` just transitioned to empty. The host's own
    /// close path doesn't reap the warm pool when the last
    /// user-visible window closes (pool windows hold
    /// `state.browsers` non-empty, so `quit_message_loop` never
    /// fires). The launcher's reducer is the only place that knows
    /// "all user-meaningful labels are gone" cleanly, so we
    /// surface the signal here. Paired with `Event::HostShouldQuit`
    /// emitted in the same reducer call (see B.9.3 saga).
    OrphanInstance,
}

/// Phase B.9.1 — drift severity, assigned per drift kind by the
/// launcher's `severity_for` (`crates/launcher/src/wrr/mod.rs`). The
/// operator-tunable severity floor the B.9 design described was never
/// built: nothing filters events by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// Coarse-grained launcher state. Spec §4: Starting → Running →
/// Quitting → Dead, no other transitions allowed. The reducer in
/// agentmux-launcher::reducer enforces this; a violation panics
/// (Job Object reaps via OS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecyclePhase {
    /// Initial state; launcher has not yet seen the host register.
    Starting,
    /// Host has registered and the canonical state is being
    /// maintained. Steady state.
    Running,
    /// Quit { reason } received, ack outstanding to subscribers.
    /// Phase B.3 keeps this state-shape only — the actual Quit
    /// command lands in a later sub-PR.
    Quitting,
    /// Cleanup done; launcher about to exit. Transient.
    Dead,
}

/// Phase B.4 follow-up — which mirror diverged. Tagged so subscribers
/// can route alerts (windows-drift might page; pool-drift is more
/// ephemeral since the pool turns over fast).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    Windows,
    Pool,
}

/// Discriminant for `Event::Error` — keeps clients structured against
/// failure modes without parsing message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Couldn't deserialize the line into a Command.
    InvalidCommand,
    /// Command sent before Register.
    NotRegistered,
    /// Register sent twice on the same connection.
    AlreadyRegistered,
}
