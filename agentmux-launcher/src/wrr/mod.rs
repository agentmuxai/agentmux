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

/// Internal — the four branches of `apply_hwnd_opened` against a
/// known `label_hint`. Lifted into its own enum so the function
/// can drop the `&mut WindowMirror` borrow before calling
/// `state.bump_version()` on the drift-emitting path (rustc E0499).
enum HwndOpenedOutcome {
    /// Existing mirror was waiting for an HWND; linked successfully.
    Linked,
    /// Existing mirror was already linked to a DIFFERENT HWND. The
    /// explicit `on_after_created` path is authoritative — REPAIR by
    /// overwriting the stale link, and emit `HwndWithoutBrowser`
    /// drift to surface the prior incorrect link for diagnostics.
    /// Carry the prior HWND for the drift message.
    /// (PR #664: replaces the no-repair behavior that caused the
    /// `panel grows but no window appears` user symptom under
    /// burst creates.)
    Repaired(u64),
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
        //
        // `drain_pending` is set when the link succeeds so we can
        // remove a matching stale `pending_hwnds` entry AFTER
        // releasing the mirror borrow. The dual-source design
        // (WinEvent CREATE + on_after_created) can leave a stale
        // pending entry; without draining it,
        // `apply_hwnd_destroyed` would early-return on the
        // stale entry and skip the orphan-destroy chain.
        // (reagent #600 P1.)
        // PR #664 codex P1 round 5 — STEAL HWND from any other mirror
        // that currently claims it. The drain-on-WindowOpened may have
        // wrong-linked the same HWND to a different label earlier; if
        // we don't clear that other mirror's link, we end up with TWO
        // mirrors pointing to the same HWND. `apply_hwnd_destroyed`
        // uses `iter().find(...)` which returns the FIRST match — the
        // other mirror would persist as a ghost row forever.
        //
        // Scan FIRST (immutable borrow), then mutate via `get_mut(label)`.
        let stolen_from: Option<String> = state
            .windows
            .iter()
            .find(|(other_label, m)| {
                other_label.as_str() != label && m.hwnd == Some(hwnd)
            })
            .map(|(other_label, _)| other_label.clone());

        let mut drain_pending = false;
        let outcome: HwndOpenedOutcome = match state.windows.get_mut(label) {
            Some(mirror) if mirror.hwnd.is_none() => {
                mirror.hwnd = Some(hwnd);
                drain_pending = true;
                HwndOpenedOutcome::Linked
            }
            Some(mirror) if mirror.hwnd == Some(hwnd) => {
                // Same HWND already linked. This is a benign
                // duplicate from the dual-source design: WinEvent
                // CREATE hook reports first (label_hint=None,
                // pending), then `on_after_created` reports
                // explicitly (label_hint=Some, this path). Or
                // vice versa under timing variation. No-op,
                // no drift. (codex #600 P2.)
                //
                // Drain any matching stale pending entry. Without
                // this, `apply_hwnd_destroyed` would early-return
                // on the stale pending entry instead of running
                // the orphan-destroy chain. (reagent #600 P1.)
                drain_pending = true;
                HwndOpenedOutcome::Linked
            }
            Some(mirror) => {
                // PR #664 — REPAIR instead of just emitting drift.
                // The explicit `on_after_created` path is the
                // AUTHORITATIVE source for label↔HWND linking. The
                // launcher's drain-on-WindowOpened (in
                // `handle_report_window_opened`) provides best-effort
                // linking but can wrong-pick under burst creates;
                // when the explicit path arrives later it REPAIRS
                // any stale link by overwriting.
                //
                // The `prior` HWND that was wrongly linked here will
                // be re-attributed when ITS OWN on_after_created
                // fires (same flow, recursive REPAIR if needed).
                //
                // We still emit `HwndWithoutBrowser` drift so the
                // existence of a stale link is visible in the log
                // for diagnostic purposes. Without the drift event,
                // the silent repair would mask real bugs.
                let prior = mirror.hwnd.unwrap_or(0);
                mirror.hwnd = Some(hwnd);
                drain_pending = true;
                HwndOpenedOutcome::Repaired(prior)
            }
            None => HwndOpenedOutcome::NoMatchingLabel,
        };
        // Apply the steal AFTER the get_mut borrow is released.
        // (codex P1 round 5) Maintains the 1:1 HWND↔label invariant
        // that `apply_hwnd_destroyed`'s find()-by-hwnd relies on.
        // Steal is meaningful only when we actually claimed the HWND
        // (Linked or Repaired); NoMatchingLabel falls through to
        // pending stash without claiming.
        let stole = matches!(outcome, HwndOpenedOutcome::Linked | HwndOpenedOutcome::Repaired(_))
            && stolen_from.is_some();
        if stole {
            if let Some(other_label) = stolen_from.as_deref() {
                if let Some(other) = state.windows.get_mut(other_label) {
                    other.hwnd = None;
                }
            }
        }
        if drain_pending {
            state.pending_hwnds.remove(&hwnd);
        }
        match outcome {
            HwndOpenedOutcome::Linked if !stole => return vec![],
            HwndOpenedOutcome::Linked => {
                // Linked + stole: we cleanly linked our mirror, but
                // had to take the HWND from another label that was
                // wrongly holding it (drain wrong-pick that wasn't
                // yet repaired). Emit drift so the steal is visible.
                let v = state.bump_version();
                let other = stolen_from.as_deref().unwrap_or("?");
                return vec![Event::HwndDriftDetected {
                    kind: HwndDriftKind::HwndWithoutBrowser,
                    label: Some(label.to_string()),
                    hwnd: Some(hwnd),
                    detail: format!(
                        "ReportHwndOpened label_hint={} linked hwnd={} (stole from label={})",
                        label, hwnd, other
                    ),
                    severity: severity_for(HwndDriftKind::HwndWithoutBrowser),
                    version: v,
                }];
            }
            HwndOpenedOutcome::Repaired(existing) => {
                // Repair is a normal self-healing path: the launcher's
                // best-effort drain in `handle_report_window_opened`
                // wrong-picked an HWND under burst-create concurrency,
                // and the explicit `apply_hwnd_opened` from
                // `client.rs::on_after_created` is now correcting it.
                // Logging this at Error severity as a `HwndDriftDetected`
                // event flooded the renderer with one drift per fresh
                // top-level (6 in a clean v0.33.696 session) and made
                // genuine drifts harder to spot.
                //
                // Now: log via tracing only, no event. The pure-state
                // mutation (mirror.hwnd overwrite + drain_pending +
                // optional steal-clear on the prior holder) still
                // happens above. Linked + stole still emits drift —
                // that's a different shape (clean link that had to
                // claim a wrongly-held HWND, which the prior holder's
                // own `apply_hwnd_opened` may not yet have repaired).
                let stolen_suffix = stolen_from
                    .as_deref()
                    .map(|s| format!(" (stole from label={})", s))
                    .unwrap_or_default();
                crate::log(&format!(
                    "[wrr] hwnd_repaired label={} prior_hwnd={} new_hwnd={}{}",
                    label, existing, hwnd, stolen_suffix
                ));
                return vec![];
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
pub fn apply_hwnd_destroyed(state: &mut State, hwnd: u64, host_running: bool) -> Vec<Event> {
    // Drain any pending entry first. Don't early-return: the
    // dual-source design (WinEvent CREATE + explicit
    // on_after_created link) can leave a stale pending entry
    // co-existing with a linked mirror — we still need to run
    // the mirror check below to fire the orphan-destroy chain
    // on a renderer crash. (reagent #600 P1.)
    let _ = state.pending_hwnds.remove(&hwnd);

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
        //
        // Emit the SAME shutdown events the normal close path
        // (`handle_report_window_closed`) would emit:
        // `WindowClosed` (so subscribers prune mirrors / atoms)
        // and `WindowInstanceReleased` (so the InstancePanel
        // count drops). Without these the frontend would show a
        // stale window after a renderer crash. Order: drift first
        // (so logs lead with "this is bad"), then the shutdown
        // events. (reagent #600 P1.)
        let _ = state.windows.remove(&label);
        let released_num = state.instance_registry.remove(&label);
        let _ = state.backend_window_ids.remove(&label);
        let v_drift = state.bump_version();
        let drift = Event::HwndDriftDetected {
            kind: HwndDriftKind::OrphanDestroy,
            label: Some(label.clone()),
            hwnd: Some(hwnd),
            detail: format!(
                "HWND destroyed without preceding ReportWindowClosed for label={}",
                label
            ),
            severity: severity_for(HwndDriftKind::OrphanDestroy),
            version: v_drift,
        };
        let v_closed = state.bump_version();
        let closed = Event::WindowClosed {
            label: label.clone(),
            version: v_closed,
            // crash-detected close: host's on_before_close didn't
            // run, so the F.6 cleanup saga must skip this trigger
            // (it would never receive the panes-reaped / pool-drain
            // terminal reports).
            crash_detected: true,
        };
        let mut out = vec![drift, closed];
        if let Some(num) = released_num {
            let v_released = state.bump_version();
            out.push(Event::WindowInstanceReleased {
                label,
                num,
                version: v_released,
            });
        }

        // Mirror the OrphanInstance + HostShouldQuit pair the normal
        // close path emits at `reducer/window.rs::handle_report_window_closed`.
        // Without this, a crash-detected last-window close empties
        // `state.windows` but never wakes the host's orphan reconciler
        // (which only listens to `HostShouldQuit`), so the warm pool
        // browsers stay alive and the host doesn't quit. Caller passes
        // `host_running` so wrr stays out of the connection module's
        // private API.
        //
        // Workstream 0 Phase 1 — same background-service exclusion as the
        // normal close path (see its comment): don't arm the orphan/
        // teardown-backstop machinery for an intentionally-resting host.
        if state.windows.is_empty() && !state.background_service_enabled && host_running {
            let v_drift = state.bump_version();
            out.push(Event::HwndDriftDetected {
                kind: HwndDriftKind::OrphanInstance,
                label: None,
                hwnd: None,
                detail:
                    "Last user-visible window destroyed (crash-detected); host still alive (likely holding warm pool)"
                        .to_string(),
                severity: Severity::Warn,
                version: v_drift,
            });
            let v_quit = state.bump_version();
            out.push(Event::HostShouldQuit { version: v_quit });
        }

        return out;
    }

    // Case 1 (or: HWND was already removed from a mirror by a
    // prior `WindowClosed`, then this destroy is the natural
    // follow-up). No drift.
    vec![]
}

/// Placement grace window. CEF creates top-level windows hidden,
/// runs `SetWindowPos` to place them, then shows them. The
/// intermediate `WM_HIDE` events arrive before `WM_FOREGROUND`,
/// which would otherwise look like `HiddenSinceOpen` drift on
/// every fresh window. Hides occurring within this window of the
/// host's `ReportWindowOpened` are part of normal placement and
/// don't count.
const HIDDEN_SINCE_OPEN_GRACE_MS: u64 = 500;

/// Phase B.9.1 — handle `Command::ReportHwndVisibilityChanged`.
/// Drift fires only on `visible=false` for a known label that has
/// not been foregrounded since open AND is past the post-open
/// placement grace window.
pub fn apply_hwnd_visibility_changed(
    state: &mut State,
    hwnd: u64,
    visible: bool,
    now_ms: u64,
) -> Vec<Event> {
    let mut drift: Option<Event> = None;
    let mut version_to_bump = false;
    let label_for_drift: Option<String> = state
        .windows
        .iter_mut()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .and_then(|(label, mirror)| {
            mirror.visible = visible;
            // Visibility=true at any time clears any deferred hide
            // — the placement transition completed, no drift needed.
            if visible {
                mirror.hidden_since_open_deferred = false;
                return None;
            }
            // Drift-storm cap: HiddenSinceOpen fires AT MOST ONCE per
            // window per session. The cap flag is monotonic for the
            // window's lifetime. The placement grace check below is
            // additive: hides during placement set `hidden_since_open_deferred`
            // (without arming the cap) so the next reducer call past
            // the grace via `drain_deferred_hidden_since_open` can
            // fire the drift. Hides past the grace fire immediately.
            let past_grace =
                now_ms.saturating_sub(mirror.opened_at_ms) > HIDDEN_SINCE_OPEN_GRACE_MS;
            if past_grace
                && !mirror.foregrounded_since_open
                && !mirror.hidden_since_open_emitted
            {
                mirror.hidden_since_open_emitted = true;
                mirror.hidden_since_open_deferred = false;
                version_to_bump = true;
                Some(label.clone())
            } else if !past_grace
                && !mirror.foregrounded_since_open
                && !mirror.hidden_since_open_emitted
            {
                // Suppressed during placement grace. Mark as deferred
                // so a later reducer call past the grace window can
                // promote this to a drift if the window is still
                // hidden (codex P2 PR #725 round 1 — without this,
                // a stuck-hidden window that gets no further
                // visibility events permanently loses the signal).
                mirror.hidden_since_open_deferred = true;
                None
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

/// Sweep `hidden_since_open_deferred` mirrors and emit drift for any
/// that have crossed the placement grace boundary while still hidden
/// and never foregrounded. Called from `reducer::update` AFTER every
/// command processes so any recovery event the command itself
/// dispatched (visible=true / foreground change / window closed) has
/// a chance to clear the deferred state first. Without the AFTER
/// ordering, a slow placement whose first post-grace event is the
/// recovery itself would fire a spurious drift before the recovery
/// runs (codex P2 PR #725 round 2).
///
/// Even with the AFTER ordering, this pass is the heartbeat that
/// catches stuck-hidden windows whose own `ReportHwndVisibilityChanged`
/// was suppressed during grace: any subsequent unrelated command
/// past the grace promotes the deferred state to a fired drift.
///
/// (codex P2 PR #725 round 1 — addresses the "no recheck after grace"
/// concern. Stuck-hidden windows that produce ZERO further commands
/// are still a hole — we'd need a periodic timer for that — but
/// realistic launcher traffic generates events constantly, so this
/// catches the practical cases.)
pub fn drain_deferred_hidden_since_open(state: &mut State, now_ms: u64) -> Vec<Event> {
    let stuck: Vec<(String, Option<u64>)> = state
        .windows
        .iter()
        .filter(|(_, m)| m.hidden_since_open_deferred)
        .filter(|(_, m)| !m.visible)
        .filter(|(_, m)| !m.foregrounded_since_open)
        .filter(|(_, m)| !m.hidden_since_open_emitted)
        .filter(|(_, m)| now_ms.saturating_sub(m.opened_at_ms) > HIDDEN_SINCE_OPEN_GRACE_MS)
        .map(|(label, m)| (label.clone(), m.hwnd))
        .collect();
    let mut events = Vec::with_capacity(stuck.len());
    for (label, hwnd) in stuck {
        if let Some(mirror) = state.windows.get_mut(&label) {
            mirror.hidden_since_open_emitted = true;
            mirror.hidden_since_open_deferred = false;
        }
        let v = state.bump_version();
        events.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::HiddenSinceOpen,
            label: Some(label),
            hwnd,
            detail: "Window hidden without ever being foregrounded since open (deferred from placement grace)".to_string(),
            severity: severity_for(HwndDriftKind::HiddenSinceOpen),
            version: v,
        });
    }
    events
}

/// Phase B.9.1 — handle `Command::ReportHwndForegroundChanged`.
/// Updates the "has been seen" flag. Never emits drift directly
/// — its role is to suppress future `HiddenSinceOpen` emissions.
pub fn apply_hwnd_foreground_changed(state: &mut State, hwnd: u64, now_ms: u64) -> Vec<Event> {
    if let Some((_, mirror)) = state.windows.iter_mut().find(|(_, m)| m.hwnd == Some(hwnd)) {
        mirror.foregrounded_since_open = true;
        mirror.last_foreground_at_ms = Some(now_ms);
        // Foreground = window made it to the user. Clear any deferred
        // hide so the drain pass doesn't fire spurious drift past the
        // grace for a window the user actually saw.
        mirror.hidden_since_open_deferred = false;
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
    let mut events: Vec<Event> = Vec::new();
    let monitors = state.monitors.clone();

    // Phase B.9.1 diagnostic — single line per position event so
    // operators can correlate with host-side hook activity.
    let linked_label_diag: Option<String> = state
        .windows
        .iter()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .map(|(label, _)| label.clone());
    crate::log(&format!(
        "[ipc] WRR-POS hwnd={:#x} rect=({},{})-({},{}) linked={:?} monitors={} pending={}",
        hwnd, new_rect.left, new_rect.top, new_rect.right, new_rect.bottom,
        linked_label_diag, monitors.len(), state.pending_hwnds.len()
    ));

    // Sentinel-aware drift suppression: CEF Views creates Win32
    // top-level windows at the (-32000,-32000) / (-31970,-31970)
    // hidden-sentinel positions for a brief moment between
    // CreateWindow and the first SetWindowPos that places them.
    // Firing OffMonitor on every window's open transient produces
    // log noise without surfacing a real bug. Suppress drift for
    // these positions; if the window stays at the sentinel
    // (genuine bug), the *follow-up* event that lands at a
    // non-sentinel off-monitor position WILL fire — and if it
    // never moves, the corrective branch below acts on the FIRST
    // sentinel report (since the foregrounded_since_open guard
    // catches it).
    let is_sentinel = is_win32_hidden_sentinel(&new_rect);

    // Resolve the mirror: collect everything we need from a scoped
    // borrow, then release it before calling `state.bump_version()`
    // (rustc E0499 — same trick as `apply_hwnd_opened`).
    //
    // Storm-cap snapshot: capture the prior emit-flags so the gate
    // logic below knows whether each side-effect has fired before.
    // `apply_hwnd_position_changed` fires per WM_MOVE during a
    // drag — without the caps, a window dragged across an off-
    // monitor region storms the renderer with drift + corrective
    // events.
    struct Resolved {
        label: String,
        off_monitor: bool,
        foregrounded_since_open: bool,
        off_monitor_drift_emitted: bool,
        corrective_window_move_emitted: bool,
    }
    let resolved: Option<Resolved> = state
        .windows
        .iter_mut()
        .find(|(_, m)| m.hwnd == Some(hwnd))
        .map(|(label, mirror)| {
            mirror.last_rect = Some(new_rect);
            let off_monitor =
                !monitors.is_empty() && !rect::intersects_any(&new_rect, &monitors);
            Resolved {
                label: label.clone(),
                off_monitor,
                foregrounded_since_open: mirror.foregrounded_since_open,
                off_monitor_drift_emitted: mirror.off_monitor_drift_emitted,
                corrective_window_move_emitted: mirror.corrective_window_move_emitted,
            }
        });

    let Some(r) = resolved else {
        return events;
    };

    if !r.off_monitor {
        return events;
    }

    // Window is off all monitors. Fire drift unless we're in the
    // open-transient sentinel state (per above) OR the cap has
    // already fired for this window.
    let mut fire_drift = false;
    let mut fire_corrective = false;
    if !is_sentinel && !r.off_monitor_drift_emitted {
        fire_drift = true;
    }

    // Phase B.9.2 — pure-reducer self-heal. If the window has
    // never been foregrounded, this off-monitor state is from the
    // open transition (NOT user action), so we emit a corrective
    // move. The host's WRR subscriber applies it via SetWindowPos
    // on the UI thread. The Win32 hidden sentinel is INCLUDED in
    // the corrective trigger (we always want to move a window off
    // the sentinel before the user notices), even though it's
    // suppressed for drift to avoid log noise.
    //
    // Compute the corrective target ONCE (reagent P2 PR #722 round 2)
    // — used both as the gate for `fire_corrective` and as the
    // event payload below.
    let corrective_target = if !r.foregrounded_since_open && !r.corrective_window_move_emitted {
        pick_primary_centered(&monitors)
    } else {
        None
    };
    if corrective_target.is_some() {
        fire_corrective = true;
    }

    if fire_drift {
        // Mark the cap before bumping the version so re-entrant
        // event handlers see consistent state.
        if let Some((_, mirror)) = state
            .windows
            .iter_mut()
            .find(|(_, m)| m.hwnd == Some(hwnd))
        {
            mirror.off_monitor_drift_emitted = true;
        }
        let v = state.bump_version();
        events.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::OffMonitor,
            label: Some(r.label.clone()),
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

    if let Some(target) = corrective_target.filter(|_| fire_corrective) {
        if let Some((_, mirror)) = state
            .windows
            .iter_mut()
            .find(|(_, m)| m.hwnd == Some(hwnd))
        {
            mirror.corrective_window_move_emitted = true;
        }
        let v = state.bump_version();
        events.push(Event::CorrectiveWindowMove {
            hwnd,
            target_rect: target,
            reason: HwndDriftKind::OffMonitor,
            version: v,
        });
    }

    events
}

/// Phase B.9.1 — Win32 sentinel positions for "this window is
/// hidden." CEF parks new windows here briefly between create
/// and first paint; same value family (`SW_HIDE` analog used by
/// `ITaskbarList::DeleteTab` removed windows). We suppress drift
/// emission for these but DO trigger corrective move (the user
/// shouldn't see the sentinel).
fn is_win32_hidden_sentinel(r: &Rect) -> bool {
    // Both classic ((-32000,-32000)) and the (-31970, -31970)
    // CEF-Views variant. Plus a generous epsilon for either
    // axis since the bottom-right corner is offset by a default
    // window size (e.g. (-31840,-31972)).
    (r.left <= -31000 && r.top <= -31000) || (r.right <= -31000 && r.bottom <= -31000)
}

/// Phase B.9.2 — pick a corrective target rect: centered on the
/// first monitor at a sensible default size (1280x800 or 70% of
/// monitor, whichever is smaller). `None` if the monitor list is
/// empty (caller suppresses corrective in that case).
fn pick_primary_centered(monitors: &[Rect]) -> Option<Rect> {
    let primary = monitors.first()?;
    let mw = primary.right - primary.left;
    let mh = primary.bottom - primary.top;
    if mw <= 0 || mh <= 0 {
        return None;
    }
    let w = std::cmp::min(1280, (mw as f32 * 0.7) as i32);
    let h = std::cmp::min(800, (mh as f32 * 0.7) as i32);
    let cx = primary.left + (mw - w) / 2;
    let cy = primary.top + (mh - h) / 2;
    Some(Rect {
        left: cx,
        top: cy,
        right: cx + w,
        bottom: cy + h,
    })
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
    // Gate emission on `off_monitor_drift_emitted` (codex P2 PR
    // #722 round 3): without this, repeated topology changes
    // (display hot-plug or rapid resolution change) re-emit drift
    // for the same stranded window every event.
    let stranded: Vec<(String, u64, Rect)> = state
        .windows
        .iter()
        .filter_map(|(label, mirror)| {
            if mirror.off_monitor_drift_emitted {
                return None;
            }
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
        if let Some((_, mirror)) = state
            .windows
            .iter_mut()
            .find(|(l, _)| **l == label)
        {
            mirror.off_monitor_drift_emitted = true;
        }
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
        // Phase B.9.3 — OrphanInstance is operationally significant
        // (process tree won't quit) but isn't a state-machine bug
        // per se; it's an observation about cross-process lifecycle.
        // WARN matches the other "user can't see / use this" kinds.
        HwndDriftKind::OrphanInstance => Severity::Warn,
    }
}
