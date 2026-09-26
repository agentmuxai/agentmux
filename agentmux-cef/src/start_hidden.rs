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

use std::sync::{Arc, Mutex};

use crate::state::AppState;

/// Env var the launcher sets.
pub const ENV: &str = "AGENTMUX_START_HIDDEN";

/// The label of the window a hidden start holds back.
pub const HELD_LABEL: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// "main" is held. `reveal_skipped`: its first reveal was attempted and
    /// skipped, so a release must reveal it; otherwise the load path still
    /// will.
    Held { reveal_skipped: bool },
    Released,
}

/// Hold state. Starts held when the env var was set at process start.
///
/// One lock over both facts (held, and whether a reveal was skipped), not two
/// atomics: the load path (UI thread) and a request (IPC thread) race, and
/// with separate flags a release could decide "the load path will show it"
/// while the load path decided "held, skip it", leaving "main" hidden for
/// good (ReAgent P1 on #3854). Under one lock exactly one side shows it.
#[derive(Debug)]
pub struct StartHidden {
    hold: Mutex<Hold>,
}

impl StartHidden {
    pub fn new(held: bool) -> Self {
        let hold = if held { Hold::Held { reveal_skipped: false } } else { Hold::Released };
        Self { hold: Mutex::new(hold) }
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var_os(ENV).is_some())
    }

    /// Called where a top-level window is about to be shown for the first
    /// time. `Skip::Yes` = do not show it; `first` marks the first skipped
    /// attempt (retries are skipped too), which starts the audit period.
    pub fn should_skip_reveal(&self, label: Option<&str>) -> Skip {
        self.should_skip_reveal_then(label, || {})
    }

    /// `should_skip_reveal`, running `on_first_skip` under the hold lock, so
    /// the audit entry it writes cannot be reordered against `release_then`'s.
    fn should_skip_reveal_then(&self, label: Option<&str>, on_first_skip: impl FnOnce()) -> Skip {
        if label != Some(HELD_LABEL) {
            return Skip::No;
        }
        let mut hold = self.hold.lock().unwrap_or_else(|e| e.into_inner());
        match *hold {
            Hold::Released => Skip::No,
            Hold::Held { reveal_skipped } => {
                *hold = Hold::Held { reveal_skipped: true };
                if !reveal_skipped {
                    on_first_skip();
                }
                Skip::Yes { first: !reveal_skipped }
            }
        }
    }

    /// Release the hold. Returns what the caller must do.
    pub fn release(&self) -> Release {
        self.release_then(|| {})
    }

    /// `release`, running `on_reveal` under the hold lock when it returns
    /// `RevealNow` (see `should_skip_reveal_then`).
    fn release_then(&self, on_reveal: impl FnOnce()) -> Release {
        let mut hold = self.hold.lock().unwrap_or_else(|e| e.into_inner());
        let was = std::mem::replace(&mut *hold, Hold::Released);
        match was {
            Hold::Released => Release::NotHeld,
            Hold::Held { reveal_skipped: true } => {
                on_reveal();
                Release::RevealNow
            }
            // "main" has not reached its first reveal yet; it now will, and
            // will find the hold gone.
            Hold::Held { reveal_skipped: false } => Release::LoadWillReveal,
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
    let skip = state.start_hidden.should_skip_reveal_then(label, || {
        tracing::info!(
            target: "startup-paint",
            "[start-hidden] login start: \"main\" loaded but held hidden until a window is requested"
        );
        record_attention(state, true);
    });
    matches!(skip, Skip::Yes { .. })
}

/// Release the hold if there is one, showing "main" if it is ready.
/// Returns `true` when the caller's request was satisfied by that (so it must
/// not open another window).
pub fn release_for_request(state: &Arc<AppState>, why: &str) -> bool {
    match state.start_hidden.release_then(|| record_attention(state, false)) {
        Release::NotHeld => false,
        Release::RevealNow => {
            tracing::info!(target: "startup-paint", why, "[start-hidden] revealing held \"main\"");
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
/// log. Called under the hold lock (so a skip and a release are logged in the
/// order they happened) and sent under `host_state`, like `host_dispatch`'s own
/// transitions, so it cannot be reordered against them either. Lock order is
/// hold → `host_state`; nothing takes the hold while holding `host_state`.
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

    /// ReAgent P1 on #3854: a release racing the load path's first reveal must
    /// end with exactly one of them showing "main", never neither.
    #[test]
    fn a_release_racing_the_load_path_always_shows_main_exactly_once() {
        for _ in 0..2000 {
            let s = std::sync::Arc::new(StartHidden::new(true));
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let (s1, b1) = (s.clone(), barrier.clone());
            let load = std::thread::spawn(move || {
                b1.wait();
                s1.should_skip_reveal(Some("main"))
            });
            barrier.wait();
            let release = s.release();
            let skip = load.join().unwrap();
            let load_shows = skip == Skip::No;
            let release_shows = release == Release::RevealNow;
            assert!(load_shows ^ release_shows, "skip={skip:?} release={release:?}");
        }
    }
}
