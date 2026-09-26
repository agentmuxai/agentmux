// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Start at login, quietly (`docs/specs/SPEC_START_WITH_OS_2026_09_25.md`
//! §3.3).
//!
//! The launcher sets `AGENTMUX_START_HIDDEN` on the first host it spawns for
//! a login start (`--background`), and only when the tray started, so there is
//! always an icon to reach the app from.
//!
//! The "main" window is still created and loaded exactly as on any start: its
//! frontend restores the session, mounts panes (which is what starts agents)
//! and starts the window pool. Only its first reveal is held. Starting with no
//! window at all was rejected: every one of those depends on "main" existing.
//!
//! The first request for a window releases the hold and shows "main" instead
//! of opening a second one: `open_new_window` (tray "New Window", a relaunch,
//! the macOS Dock) and `focus_window` for "main" (a notification click). While
//! held, the unattended period goes into the WS4 audit log, like a period with
//! every window closed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::state::AppState;

/// Env var the launcher sets.
pub const ENV: &str = "AGENTMUX_START_HIDDEN";

/// The label of the window a hidden start holds back.
pub const HELD_LABEL: &str = "main";

/// Hold state. Starts held when the env var was set at process start.
#[derive(Debug)]
pub struct StartHidden {
    held: AtomicBool,
    /// A reveal of "main" was attempted and skipped. Tells `release` whether
    /// it must reveal now, or whether the normal load path still will.
    reveal_skipped: AtomicBool,
}

impl StartHidden {
    pub fn new(held: bool) -> Self {
        Self { held: AtomicBool::new(held), reveal_skipped: AtomicBool::new(false) }
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var_os(ENV).is_some())
    }

    /// Called where a top-level window is about to be shown for the first
    /// time. `Skip::Yes` = do not show it; `first` marks the first skipped
    /// attempt (retries are skipped too), which starts the audit period.
    pub fn should_skip_reveal(&self, label: Option<&str>) -> Skip {
        if label != Some(HELD_LABEL) || !self.held.load(Ordering::SeqCst) {
            return Skip::No;
        }
        let first = !self.reveal_skipped.swap(true, Ordering::SeqCst);
        Skip::Yes { first }
    }

    /// Release the hold. Returns what the caller must do.
    pub fn release(&self) -> Release {
        if !self.held.swap(false, Ordering::SeqCst) {
            return Release::NotHeld;
        }
        if self.reveal_skipped.load(Ordering::SeqCst) {
            Release::RevealNow
        } else {
            // "main" has not reached its first reveal yet; the normal load
            // path will show it now that the hold is gone.
            Release::LoadWillReveal
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Skip {
    No,
    Yes { first: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Release {
    /// Nothing was held: open a window the ordinary way.
    NotHeld,
    /// "main" is loaded and hidden: show it.
    RevealNow,
    /// "main" is still loading: it will show itself.
    LoadWillReveal,
}

/// Hook for the reveal path. `true` = do not show this window.
pub fn hold_reveal(state: &Arc<AppState>, label: Option<&str>) -> bool {
    match state.start_hidden.should_skip_reveal(label) {
        Skip::No => false,
        Skip::Yes { first } => {
            if first {
                tracing::info!(
                    target: "startup-paint",
                    "[start-hidden] login start: \"main\" loaded but held hidden until a window is requested"
                );
                record_attention(state, true);
            }
            true
        }
    }
}

/// Release the hold if there is one, showing "main" if it is ready.
/// Returns `true` when the caller's request was satisfied by that (so it must
/// not open another window).
pub fn release_for_request(state: &Arc<AppState>, why: &str) -> bool {
    match state.start_hidden.release() {
        Release::NotHeld => false,
        Release::RevealNow => {
            tracing::info!(target: "startup-paint", why, "[start-hidden] revealing held \"main\"");
            record_attention(state, false);
            crate::client::navigation::post_show_top_level_window(state, HELD_LABEL);
            true
        }
        Release::LoadWillReveal => {
            tracing::info!(target: "startup-paint", why, "[start-hidden] hold released before \"main\" finished loading");
            true
        }
    }
}

/// Record the start or end of the held (unattended) period in the WS4 audit
/// log. Sent under `host_state`, like `host_dispatch`'s own transitions, so it
/// cannot be reordered against them.
fn record_attention(state: &Arc<AppState>, unattended: bool) {
    let _order = state.host_state.lock();
    if let Some(tx) = state.background_audit_tx.get() {
        let _ = tx.send(crate::background_audit::AuditEntry {
            at_ms: crate::background_audit::now_ms(),
            kind: if unattended {
                crate::background_audit::AuditKind::WentUnattended
            } else {
                crate::background_audit::AuditKind::Observed
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_held_changes_nothing() {
        let s = StartHidden::new(false);
        assert_eq!(s.should_skip_reveal(Some("main")), Skip::No);
        assert_eq!(s.release(), Release::NotHeld);
    }

    #[test]
    fn only_main_is_held() {
        let s = StartHidden::new(true);
        assert_eq!(s.should_skip_reveal(Some("window-abc")), Skip::No);
        assert_eq!(s.should_skip_reveal(None), Skip::No);
        assert_eq!(s.should_skip_reveal(Some("main")), Skip::Yes { first: true });
        // A retry of the same reveal is skipped too, but is not a new period.
        assert_eq!(s.should_skip_reveal(Some("main")), Skip::Yes { first: false });
    }

    #[test]
    fn a_request_after_main_loaded_reveals_it_once() {
        let s = StartHidden::new(true);
        let _ = s.should_skip_reveal(Some("main"));
        assert_eq!(s.release(), Release::RevealNow);
        // The next request is an ordinary new window.
        assert_eq!(s.release(), Release::NotHeld);
        assert_eq!(s.should_skip_reveal(Some("main")), Skip::No);
    }

    #[test]
    fn a_request_before_main_loaded_lets_the_load_reveal_it() {
        let s = StartHidden::new(true);
        assert_eq!(s.release(), Release::LoadWillReveal);
        assert_eq!(s.should_skip_reveal(Some("main")), Skip::No, "the load path shows it normally");
    }
}
