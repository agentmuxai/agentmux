// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Hung-renderer auto-recovery — `RequestHandler::on_render_process_unresponsive`
//! / `on_render_process_responsive` for `AgentMuxHandler`.
//!
//! Without this, CEF's Alloy default for a hung renderer is to wait forever.
//! That is exactly what the 2026-09-22 freeze did: the renderer's Compositor
//! thread deadlocked, srv/host/launcher stayed healthy, and the window never
//! came back (docs/incident/INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md).
//!
//! CEF calls the unresponsive hook after ~15 s of input events the renderer
//! has not acknowledged. For an app-UI renderer (our own frontend) the first
//! report re-arms that timer with `wait()` — a GC pause or a legitimately
//! long task gets a second interval — and a second consecutive report kills
//! the renderer with `terminate()`. The kill lands in
//! `on_render_process_terminated`, i.e. the existing crash-budgeted recovery
//! page, so a hang recovers exactly like a crash does. Browser panes, auth
//! popups and unregistered browsers (DevTools) keep CEF's default (wait):
//! that is arbitrary web content, where a busy page is the page's business,
//! not a host fault.
//!
//! Limitation: CEF's detector only runs while input is pending, so a hang
//! nobody clicks on is never reported here. That case needs the srv-side
//! stalled-frontend signal (handoff doc §5.2).
//!
//! Spec: docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md §8.1.

use cef::*;

use super::AgentMuxHandler;
use crate::state::BrowserKind;

/// Consecutive unresponsive reports an app-UI renderer gets before the host
/// kills it. Each report is one CEF hang-monitor interval (~15 s), so 2 means
/// ~30 s with input pending and unacknowledged — far beyond any GC pause or
/// long task the frontend legitimately runs, and still well short of a user
/// giving up and killing the whole app. Two, not one, per the spec's §10-C
/// rule: never act on a single missed signal.
const UNRESPONSIVE_REPORTS_BEFORE_TERMINATE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnresponsiveAction {
    /// Not an app-UI renderer — return 0 and let CEF apply its default.
    Default,
    /// Re-arm CEF's hang timer; the hook fires again if still hung.
    Wait,
    /// Kill the renderer; recovery continues in `on_render_process_terminated`.
    Terminate,
}

/// Is this renderer our own frontend (and therefore ours to kill when hung)?
///
/// Decided from the browser's registered `BrowserKind`, not the client's
/// `is_browser_pane` flag, which a cloned client can inherit from an unrelated
/// floater — see `navigation::takes_pane_path` and issue #3028. Floaters count
/// as app UI here: their renderer is the frontend showing the floating pane,
/// and a browser pane inside one is its own browser with kind `Pane`.
///
/// `None` (no registered label) is never app UI. Killing a renderer is only
/// safe once we positively know it is ours, and unregistered browsers include
/// DevTools windows, which share the inspected window's client and so carry
/// `is_browser_pane = false`. A hang there keeps CEF's default.
fn is_app_ui_renderer(kind: Option<&BrowserKind>) -> bool {
    matches!(
        kind,
        Some(BrowserKind::TopLevel { .. }) | Some(BrowserKind::Floater { .. })
    )
}

/// What to do on the `consecutive_reports`-th unresponsive report (1-based)
/// for an app-UI renderer.
fn action_for_report(consecutive_reports: u32) -> UnresponsiveAction {
    if consecutive_reports >= UNRESPONSIVE_REPORTS_BEFORE_TERMINATE {
        UnresponsiveAction::Terminate
    } else {
        UnresponsiveAction::Wait
    }
}

impl AgentMuxHandler {
    /// Decide what to do about a hung renderer. Returns the action instead of
    /// calling the CEF callback itself: the caller must invoke `wait()` /
    /// `terminate()` only after releasing this handler's (non-reentrant) lock,
    /// since terminating re-enters the handler via
    /// `on_render_process_terminated`.
    pub(crate) fn on_render_process_unresponsive(
        &mut self,
        browser: Option<&mut Browser>,
    ) -> UnresponsiveAction {
        let Some(browser) = browser else {
            return UnresponsiveAction::Default;
        };
        let browser_id = browser.identifier();
        let mut owned = browser.clone();
        let label = self.window_label_for(&mut owned);
        let browser_kind = label
            .as_deref()
            .and_then(|l| self.state.browser_kind_for_label(l));

        if !is_app_ui_renderer(browser_kind.as_ref()) {
            tracing::warn!(
                target: "crash",
                kind = "renderer_unresponsive",
                browser_id,
                label = ?label,
                browser_kind = ?browser_kind,
                "renderer unresponsive — not an app-UI renderer, leaving it to CEF's default wait",
            );
            return UnresponsiveAction::Default;
        }

        let reports = {
            let n = self.unresponsive_reports.entry(browser_id).or_insert(0);
            *n += 1;
            *n
        };
        let action = action_for_report(reports);
        match action {
            UnresponsiveAction::Terminate => {
                self.unresponsive_reports.remove(&browser_id);
                self.terminated_unresponsive.insert(browser_id);
                tracing::error!(
                    target: "crash",
                    kind = "renderer_hung_terminated",
                    browser_id,
                    label = ?label,
                    reports,
                    "app-UI renderer still unresponsive — terminating it so the window recovers",
                );
            }
            UnresponsiveAction::Wait => {
                tracing::warn!(
                    target: "crash",
                    kind = "renderer_unresponsive",
                    browser_id,
                    label = ?label,
                    reports,
                    terminate_at = UNRESPONSIVE_REPORTS_BEFORE_TERMINATE,
                    "app-UI renderer unresponsive — waiting one more interval before terminating",
                );
            }
            UnresponsiveAction::Default => {}
        }
        action
    }

    /// The renderer acknowledged input again — reset its strike count so a
    /// later, unrelated stall starts from zero.
    pub(crate) fn on_render_process_responsive(&mut self, browser: Option<&mut Browser>) {
        let Some(browser) = browser else { return };
        let browser_id = browser.identifier();
        if let Some(reports) = self.unresponsive_reports.remove(&browser_id) {
            tracing::info!(
                target: "crash",
                kind = "renderer_responsive_again",
                browser_id,
                reports,
                "renderer responsive again — hang recovered without intervention",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_windows_and_floaters_are_app_ui() {
        for kind in [
            BrowserKind::TopLevel { is_pool: false },
            BrowserKind::TopLevel { is_pool: true },
            BrowserKind::Floater { is_pool: false },
            BrowserKind::Floater { is_pool: true },
        ] {
            assert!(is_app_ui_renderer(Some(&kind)), "{kind:?}");
        }
    }

    #[test]
    fn browser_panes_and_auth_popups_are_never_auto_terminated() {
        let pane = BrowserKind::Pane {
            block_id: "b1".into(),
        };
        assert!(!is_app_ui_renderer(Some(&pane)));
        assert!(!is_app_ui_renderer(Some(&BrowserKind::Popup)));
    }

    #[test]
    fn unregistered_browser_is_never_auto_terminated() {
        // DevTools and anything else without a registered label.
        assert!(!is_app_ui_renderer(None));
    }

    #[test]
    fn first_report_waits_second_terminates() {
        assert_eq!(action_for_report(1), UnresponsiveAction::Wait);
        assert_eq!(action_for_report(2), UnresponsiveAction::Terminate);
        assert_eq!(action_for_report(3), UnresponsiveAction::Terminate);
    }
}
