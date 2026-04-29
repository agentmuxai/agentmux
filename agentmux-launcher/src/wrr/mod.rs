// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — Window Reality Reconciliation (WRR) reducer arm.
//
// Catches the class of bug surfaced during the B.6.1 smoke test:
// a CEF browser opens, gets a Win32 HWND, and is then "lost" to
// the user (off-screen, behind another window, never foregrounded).
// The pre-B.9 reducer tracked identity (`label`, `kind`, `parent`)
// but not observability (visible? on-monitor? has the user seen
// it?), so its drift detector compared `host.browsers.len() ==
// launcher.windows.len()` — both can be in lockstep wrong about
// Win32 reality.
//
// Design lives at `docs/retro/wrr-design-2026-04-28.md`. This
// module implements the launcher-side reducer arm:
//
//   * `apply_*` functions: one per `Command::ReportHwnd*` /
//     `ReportMonitorTopologyChanged`. Each mutates `State` and
//     emits `Event::HwndDriftDetected` for every classification
//     it can determine at this transition.
//   * `severity_for(kind)`: the per-kind severity floor classifier.
//
// Pure event-driven. There is no clock task, no heartbeat — drift
// is emitted at the moment the OS-driven Command is dispatched
// through the reducer. See the design doc for the trade-off ("we
// don't catch steady-state staleness without an event").

use agentmux_common::ipc::{Event, HwndDriftKind, Rect, Severity};

use crate::state::{PendingHwnd, State};

pub mod rect;

/// Internal — the three branches of `apply_hwnd_opened` against a
/// known `label_hint`. Lifted into its own enum so the function
/// can drop the `&mut WindowMirror` borrow before calling
/// `state.bump_version()` on the `Linked`-double-link path
/// (rustc E0499).
enum HwndOpenedOutcome {
    /// Existing mirror was waiting for an HWND; linked successfully.
    Linked,
    /// Existing mirror was already linked to a different HWND. Carry
    /// the prior HWND for the drift message.
    DoubleLinkedWith(u64),
    /// No mirror exists for that label — fall through to pending
    /// stash (caller responsibility).
    NoMatchingLabel,
}

/// Phase B.9.1 — handle `Command::ReportHwndOpened`. Either:
///   1. The hwnd's `label_hint` matches an existing
///      `state.windows[label]` whose `hwnd` is `None` → link them.
///   2. No matching label → stash in `state.pending_hwnds` for a
///      later reconciliation. If the class name doesn't look like
///      an AgentMux window (filtered at the host hook, but
///      defense-in-depth here too), don't even stash.
pub fn apply_hwnd_opened(
    state: &mut State,
    hwnd: u64,
    class_name: String,
    title: String,
    label_hint: Option<String>,
    now_ms: u64,
) -> Vec<Event> {
    // Case 1: label_hint maps to an existing WindowMirror that's
    // waiting for an HWND. Happy path — link them, no drift.
    if let Some(label) = label_hint.as_deref() {
        // Read mirror state via a scoped borrow so we can release
        // it before calling `state.bump_version()` (which needs &mut
        // self on State). Result tells us which branch to take
        // outside the borrow.
        let outcome: HwndOpenedOutcome = match state.windows.get_mut(label) {
            Some(mirror) if mirror.hwnd.is_none() => {
                mirror.hwnd = Some(hwnd);
                HwndOpenedOutcome::Linked
            }
            Some(mirror) => {
                HwndOpenedOutcome::DoubleLinkedWith(mirror.hwnd.unwrap_or(0))
            }
            None => HwndOpenedOutcome::NoMatchingLabel,
        };
        match outcome {
            HwndOpenedOutcome::Linked => return vec![],
            HwndOpenedOutcome::DoubleLinkedWith(existing) => {
                let v = state.bump_version();
                return vec![Event::HwndDriftDetected {
                    kind: HwndDriftKind::HwndWithoutBrowser,
                    label: Some(label.to_string()),
                    hwnd: Some(hwnd),
                    detail: format!(
                        "ReportHwndOpened label_hint={} already linked to hwnd={}",
                        label, existing
                    ),
                    severity: severity_for(HwndDriftKind::HwndWithoutBrowser),
                    version: v,
                }];
            }
            HwndOpenedOutcome::NoMatchingLabel => { /* fall through to pending */ }
        }
    }

    // Case 2: stash as pending. Filtered class names get dropped
    // here too as belt-and-suspenders — host hook is the primary
    // filter (see `wrr/classify.rs::is_app_class` in agentmux-cef).
    state.pending_hwnds.insert(
        hwnd,
        PendingHwnd {
            class_name,
            title,
            label_hint,
            arrived_at_ms: now_ms,
        },
    );
    vec![]
}

/// Phase B.9.1 — handle `Command::ReportHwndDestroyed`. Three
/// outcomes:
///   1. HWND links to a `WindowMirror` AND we already received a
///      `ReportWindowClosed` for that label (mirror is gone) →
///      no drift, expected ordering.
///   2. HWND links to a `WindowMirror` that's STILL in
///      `state.windows` → CEF didn't report close yet. Renderer
///      probably crashed → `OrphanDestroy` drift.
///   3. HWND was pending (never linked) → drop the pending entry,
///      no drift (it never claimed to be a real window).
pub fn apply_hwnd_destroyed(state: &mut State, hwnd: u64) -> Vec<Event> {
    // Case 3: pending → just drop.
    if state.pending_hwnds.remove(&hwnd).is_some() {
        return vec![];
    }

    // Find the label whose mirror is linked to this HWND, if any.
    let orphan_label: Option<String> = state
        .windows
        .iter()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .map(|(label, _)| label.clone());

    if let Some(label) = orphan_label {
        // Case 2: orphan destroy. Clear the link AND the mirror
        // entry — the window is gone from Win32, regardless of
        // what CEF thinks. Future `ReportWindowClosed` for the
        // label will be a no-op (closed-on-missing is silently
        // tolerated upstream).
        let _ = state.windows.remove(&label);
        let _ = state.instance_registry.remove(&label);
        let v = state.bump_version();
        return vec![Event::HwndDriftDetected {
            kind: HwndDriftKind::OrphanDestroy,
            label: Some(label.clone()),
            hwnd: Some(hwnd),
            detail: format!(
                "HWND destroyed without preceding ReportWindowClosed for label={}",
                label
            ),
            severity: severity_for(HwndDriftKind::OrphanDestroy),
            version: v,
        }];
    }

    // Case 1 (or: HWND was already removed from a mirror by a
    // prior `WindowClosed`, then this destroy is the natural
    // follow-up). No drift.
    vec![]
}

/// Phase B.9.1 — handle `Command::ReportHwndVisibilityChanged`.
/// Drift fires only on `visible=false` for a known label that has
/// not been foregrounded since open: the user can't see this
/// window and never has.
pub fn apply_hwnd_visibility_changed(
    state: &mut State,
    hwnd: u64,
    visible: bool,
) -> Vec<Event> {
    let mut drift: Option<Event> = None;
    let mut version_to_bump = false;
    let label_for_drift: Option<String> = state
        .windows
        .iter_mut()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .and_then(|(label, mirror)| {
            mirror.visible = visible;
            if !visible && !mirror.foregrounded_since_open {
                version_to_bump = true;
                Some(label.clone())
            } else {
                None
            }
        });
    if version_to_bump {
        let v = state.bump_version();
        drift = Some(Event::HwndDriftDetected {
            kind: HwndDriftKind::HiddenSinceOpen,
            label: label_for_drift,
            hwnd: Some(hwnd),
            detail: "Window hidden without ever being foregrounded since open".to_string(),
            severity: severity_for(HwndDriftKind::HiddenSinceOpen),
            version: v,
        });
    }
    drift.into_iter().collect()
}

/// Phase B.9.1 — handle `Command::ReportHwndForegroundChanged`.
/// Updates the "has been seen" flag. Never emits drift directly
/// — its role is to suppress future `HiddenSinceOpen` emissions.
pub fn apply_hwnd_foreground_changed(state: &mut State, hwnd: u64, now_ms: u64) -> Vec<Event> {
    if let Some((_, mirror)) = state.windows.iter_mut().find(|(_, m)| m.hwnd == Some(hwnd)) {
        mirror.foregrounded_since_open = true;
        mirror.last_foreground_at_ms = Some(now_ms);
    }
    vec![]
}

/// Phase B.9.1 — handle `Command::ReportHwndIconicChanged`. Updates
/// state. No drift directly — operator can read steady state via
/// `--diag wrr` (B.9.2) if they want to see who's minimized.
pub fn apply_hwnd_iconic_changed(state: &mut State, hwnd: u64, iconic: bool) -> Vec<Event> {
    if let Some((_, mirror)) = state.windows.iter_mut().find(|(_, m)| m.hwnd == Some(hwnd)) {
        mirror.iconic = iconic;
    }
    vec![]
}

/// Phase B.9.1 — handle `Command::ReportHwndPositionChanged`.
/// Compares the new rect against `state.monitors`; emits
/// `OffMonitor` drift if it doesn't intersect any monitor.
/// Suppressed when `state.monitors` is empty (we don't yet know
/// the topology — first `ReportMonitorTopologyChanged` will
/// reconcile every label's `last_rect` against fresh monitors).
pub fn apply_hwnd_position_changed(state: &mut State, hwnd: u64, new_rect: Rect) -> Vec<Event> {
    let mut drift: Vec<Event> = Vec::new();
    let monitors = state.monitors.clone();
    let label_for_drift: Option<String> = state
        .windows
        .iter_mut()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .and_then(|(label, mirror)| {
            mirror.last_rect = Some(new_rect);
            if !monitors.is_empty() && !rect::intersects_any(&new_rect, &monitors) {
                Some(label.clone())
            } else {
                None
            }
        });
    if let Some(label) = label_for_drift {
        let v = state.bump_version();
        drift.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::OffMonitor,
            label: Some(label),
            hwnd: Some(hwnd),
            detail: format!(
                "Window rect ({},{})-({},{}) does not intersect any of {} monitors",
                new_rect.left, new_rect.top, new_rect.right, new_rect.bottom,
                monitors.len()
            ),
            severity: severity_for(HwndDriftKind::OffMonitor),
            version: v,
        });
    }
    drift
}

/// Phase B.9.1 — handle `Command::ReportMonitorTopologyChanged`.
/// Replaces `state.monitors` wholesale, then re-evaluates every
/// known mirror's `last_rect` against the new set. Emits
/// `OffMonitor` for any window that newly falls off (e.g. user
/// unplugged the external display where the window lived).
pub fn apply_monitor_topology_changed(state: &mut State, rects: Vec<Rect>) -> Vec<Event> {
    state.monitors = rects;
    let monitors = state.monitors.clone();
    if monitors.is_empty() {
        // Headless / fully-disconnected — suppress drift; nothing
        // is "off" if there's no "on".
        return vec![];
    }
    let mut events: Vec<Event> = Vec::new();
    let stranded: Vec<(String, u64, Rect)> = state
        .windows
        .iter()
        .filter_map(|(label, mirror)| {
            let r = mirror.last_rect?;
            let h = mirror.hwnd?;
            if rect::intersects_any(&r, &monitors) {
                None
            } else {
                Some((label.clone(), h, r))
            }
        })
        .collect();
    for (label, hwnd, rect) in stranded {
        let v = state.bump_version();
        events.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::OffMonitor,
            label: Some(label),
            hwnd: Some(hwnd),
            detail: format!(
                "Monitor topology change stranded window at ({},{})-({},{})",
                rect.left, rect.top, rect.right, rect.bottom
            ),
            severity: severity_for(HwndDriftKind::OffMonitor),
            version: v,
        });
    }
    events
}

/// Phase B.9.1 — per-kind severity classifier. The split is
/// deliberate: `OrphanDestroy` and `HwndWithoutBrowser` indicate a
/// real divergence between CEF identity and Win32 reality (a CEF
/// bug or a missed report path) — ERROR. `OffMonitor`,
/// `HiddenSinceOpen`, `LingeringHwnd` are operationally
/// significant (user can't see / use the window) but don't
/// indicate a state-machine bug — WARN. `BrowserWithoutHwnd` is
/// commonly transient (race window between OS create and host
/// link) — INFO; only meaningful if it doesn't reconcile.
pub fn severity_for(kind: HwndDriftKind) -> Severity {
    match kind {
        HwndDriftKind::OrphanDestroy => Severity::Error,
        HwndDriftKind::HwndWithoutBrowser => Severity::Error,
        HwndDriftKind::OffMonitor => Severity::Warn,
        HwndDriftKind::HiddenSinceOpen => Severity::Warn,
        HwndDriftKind::LingeringHwnd => Severity::Warn,
        HwndDriftKind::BrowserWithoutHwnd => Severity::Info,
    }
}
