// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Optional system-tray / menu-bar icon — issue #2977 Workstream 1,
//! `docs/specs/SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`.
//!
//! ## Why this lives in the launcher, not the host
//!
//! §2 of the spec puts tray ownership in `agentmux-launcher` deliberately: the
//! icon must survive `host` crashes and restarts, and the launcher is the
//! process that outlives them. It is also the process that already knows how
//! to ask for a new window (`second_instance::forward_open_new_window`).
//!
//! ## The event-loop problem (§7.5)
//!
//! The spec originally assumed the tray would "coexist with whatever message
//! pump the launcher already runs for its splash". **There is no such pump on
//! Windows** — the splash creates a real window but registers `DefWindowProcW`
//! and polls, which works only because it is layered, non-activating, and
//! takes no input. The launcher's real loop is Tokio. So the Windows backend
//! here *introduces* a message pump on a dedicated thread; see `windows.rs`.
//!
//! macOS is the opposite: the main thread already pumps `NSApplication` for
//! the process lifetime (when the splash is enabled), so the menu-bar item has
//! a host loop already. Linux uses `ksni`, which is pure D-Bus and needs no
//! toolkit loop at all.
//!
//! ## Opt-in
//!
//! Off unless `AGENTMUX_TRAY` is set (presence-based, matching the
//! `AGENTMUX_DEV` / `AGENTMUX_BACKGROUND_SERVICE` idiom). Workstream 4 of the
//! issue requires the feature be opt-in and off by default, and requires the
//! icon to be a *reliable* indicator that the background service is running —
//! so the tray is only meaningful alongside `AGENTMUX_BACKGROUND_SERVICE`, and
//! `should_enable` encodes that pairing rather than leaving it to callers.

use std::sync::mpsc;

#[cfg(target_os = "windows")]
mod windows;

/// What the user picked from the tray menu. Platform backends translate their
/// native click/menu events into these; everything downstream is
/// platform-neutral, so the handling logic is written and tested once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// "New Window" (or "Start AgentMux" when the service is down) — also a
    /// left-click / double-click on the icon itself.
    /// Reuses the existing `open_new_window` forward rather than introducing a
    /// second way to make a window (see §7.5.1: the reopen path already exists
    /// and is what the macOS reopen delegate uses).
    OpenWindow,
    /// "Quit AgentMux" — a genuine, user-intended full shutdown, distinct from
    /// closing the last window while background-service mode is on.
    Quit,
}

/// A menu entry, resolved from the platform-neutral model below. Kept as data
/// (rather than built directly against `muda`/`ksni` types) so the menu's
/// composition is unit-testable without a display server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub action: TrayAction,
}

/// The tray menu, as data.
///
/// Deliberately tiny. The spec's §2 rejects a native OS menu as the *panel*
/// ("too limited for agent chat" — that is Workstream 3, a real CEF window),
/// so this menu is only a launcher: open a window, or quit. Anything richer
/// belongs in the panel, not here.
///
/// `running` drives the first item's label so the icon doubles as an honest
/// status readout, which Workstream 4 requires of it ("the tray icon itself
/// must be a reliable 'is it actually running' indicator").
pub fn menu_model(running: bool) -> Vec<MenuItem> {
    vec![
        MenuItem {
            // "New Window" rather than "Open AgentMux": the icon's presence
            // already says AgentMux is up, so "Open" read as a no-op for a
            // state the user can see is true. When the service is NOT
            // reachable the wording still has to change — "New Window" would
            // promise something that cannot happen — hence "Start AgentMux".
            label: if running {
                "New Window".to_string()
            } else {
                "Start AgentMux".to_string()
            },
            action: TrayAction::OpenWindow,
        },
        MenuItem {
            label: "Quit AgentMux".to_string(),
            action: TrayAction::Quit,
        },
    ]
}

/// Tooltip shown on hover. Same honesty requirement as the menu label: it must
/// say whether the background service is actually up, not just that an icon
/// exists.
pub fn tooltip(running: bool) -> String {
    if running {
        "AgentMux — running in the background".to_string()
    } else {
        "AgentMux — not running".to_string()
    }
}

/// Should the tray be started at all?
///
/// Requires BOTH the tray opt-in and background-service mode. A tray icon
/// without background-service mode would be actively misleading: closing the
/// last window would still quit the whole app, leaving an icon that either
/// vanishes instantly or (worse) lingers pointing at a dead instance —
/// precisely the "unreliable indicator" Workstream 4 calls out as the
/// cautionary case. Pairing them is enforced here rather than documented and
/// hoped for.
pub fn should_enable(tray_opt_in: bool, background_service: bool) -> bool {
    tray_opt_in && background_service
}

/// Read the opt-in from the environment. Presence-based, matching
/// `AGENTMUX_DEV` and `AGENTMUX_BACKGROUND_SERVICE`.
pub fn tray_opt_in_from_env() -> bool {
    std::env::var("AGENTMUX_TRAY").is_ok()
}

/// Background-service mode, as the host reads it. The launcher needs its own
/// read because it decides whether to show the icon before/independently of
/// the host reporting in.
pub fn background_service_from_env() -> bool {
    std::env::var("AGENTMUX_BACKGROUND_SERVICE").is_ok()
}

/// Start the tray if enabled, returning the receiver the caller polls for
/// user actions. `None` when the tray is disabled or unsupported on this
/// platform — callers treat that as "no tray", never as an error.
///
/// Non-fatal by construction: a tray that fails to start must never take the
/// app down with it. The launcher supervises `srv` and `host`; a cosmetic
/// icon is the least important thing it owns.
pub fn start_if_enabled(
    _data_dir: std::path::PathBuf,
    _dir_hash: String,
) -> Option<mpsc::Receiver<TrayAction>> {
    if !should_enable(tray_opt_in_from_env(), background_service_from_env()) {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        match windows::spawn(_data_dir, _dir_hash) {
            Ok(rx) => {
                crate::log("tray: started (windows)");
                Some(rx)
            }
            Err(e) => {
                crate::log(&format!("tray: failed to start, continuing without it: {}", e));
                None
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // macOS and Linux backends land in the follow-up PRs for this
        // workstream; the opt-in simply does nothing there for now rather
        // than pretending to have started.
        crate::log("tray: enabled but no backend on this platform yet — continuing without it");
        None
    }
}

#[cfg(test)]
mod tray_model_tests {
    use super::*;

    #[test]
    fn tray_requires_both_opt_in_and_background_service() {
        assert!(should_enable(true, true));
        // Tray without background-service mode would be a lying indicator:
        // the app still quits on last-window-close.
        assert!(!should_enable(true, false));
        // Background-service mode without the tray opt-in is a supported
        // configuration (that is what shipped in #2983) — just no icon.
        assert!(!should_enable(false, true));
        assert!(!should_enable(false, false));
    }

    #[test]
    fn menu_offers_new_window_and_quit_in_that_order() {
        // Two items, deliberately. The panel entry was removed: `open_panel`
        // still exists as a host IPC command (issue #2977 WS3, PR #3002), it
        // is simply not reachable from the tray for now.
        let m = menu_model(true);
        assert_eq!(m.len(), 2, "menu is deliberately minimal");
        assert_eq!(m[0].action, TrayAction::OpenWindow);
        assert_eq!(m[1].action, TrayAction::Quit);
    }

    #[test]
    fn menu_and_tooltip_report_running_state_honestly() {
        // WS4: the icon must be a reliable "is it actually running"
        // indicator, so both surfaces have to change with the state.
        assert_eq!(menu_model(true)[0].label, "New Window");
        assert_eq!(menu_model(false)[0].label, "Start AgentMux");
        assert!(tooltip(true).contains("running in the background"));
        assert!(tooltip(false).contains("not running"));
    }

    #[test]
    fn quit_is_always_offered_regardless_of_running_state() {
        // A user must always be able to get rid of a background process from
        // the icon that represents it — including when it is wedged enough
        // that `running` reads false.
        for running in [true, false] {
            assert!(
                menu_model(running).iter().any(|i| i.action == TrayAction::Quit),
                "quit missing when running={}",
                running
            );
        }
    }
}
