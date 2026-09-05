// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Workstream 0 Phase 1 prerequisite #2 (issue #2977,
//! `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §7) — bounded
//! liveness confirmation for a PROMOTED pool window.
//!
//! ## The gap this closes
//!
//! `docs/retro/retro-fresh-vm-suspend-orphaned-frontend-2026-09-03.md`
//! documents that `promote_pool_window`'s Windows liveness check
//! (`IsWindow()` against a cached HWND) proves only that the OS hasn't
//! destroyed or recycled the window handle. It cannot prove the renderer
//! behind that handle is responsive or that its page's connection survived
//! — and critically, a suspend/resume cycle does not destroy window handles,
//! so `IsWindow()` returns true for a corpse. That retro's own correction
//! notes this is a real, code-level gap independent of what caused the
//! incident it was originally written about.
//!
//! Note this is NOT specific to the cached-HWND fallback branch: CEF's own
//! `window_handle()` is equally intact across a suspend, so a fix scoped to
//! only the "weaker evidence" branch would miss the actual failure mode.
//! The check here is therefore applied to every promote, after the fact,
//! regardless of which branch resolved the HWND.
//!
//! ## The signal
//!
//! `register_backend_window` is already established in this codebase as the
//! canonical proof that a window's frontend "actually loaded and
//! round-tripped IPC, not just that a CreateWindowTask was posted" — see
//! `commands/window/meta.rs`'s `pending_reproject_closures.confirm(label)`
//! call site (SPEC_PILLAR1_STEP4 Phase 3 addendum, reagent P1 PR #2032).
//! This module reuses that exact signal, and deliberately mirrors
//! `PendingReprojectClosures`' stage/confirm shape.
//!
//! A promoted pool window reaches it end-to-end: `?pool=1` short-circuits
//! the renderer's init until `pool:promote` arrives
//! (`frontend/app/init/pool.ts`), then `initHostNewWindow()` runs and calls
//! `registerBackendWindow` (`frontend/app-init.ts`). So a confirmation
//! proves the renderer is alive, executing JS, and able to reach both srv
//! and the host — the three things `IsWindow()` cannot establish. Because
//! pool mode short-circuits BEFORE that call, a pool window cannot confirm
//! at spawn time; the only possible confirmation is post-promote.
//!
//! Deliberately NOT used: `on_load_end`. The promote path never navigates —
//! `pool.ts` applies the workspace id with `history.replaceState`, so no
//! load event fires and a load-based gate would time out on every healthy
//! promote.
//!
//! ## Epoch guard
//!
//! Watches are keyed by label but carry a monotonic epoch, so a timer that
//! fires late cannot consume a watch armed by a *later* promote of the same
//! label. Same discipline as the browser-pane load watchdog's epoch
//! (`browser_pane::callbacks`).

use std::collections::HashMap;

/// How long a promoted pool window has to prove its renderer is alive
/// before the promote is treated as having handed back a corpse.
///
/// Chosen to be generous rather than tight: the cost of a FALSE POSITIVE is
/// a redundant extra window (annoying, recoverable — and the honest cost the
/// retro's recommendation #2 explicitly accepts), while the cost of being
/// too tight is duplicating windows for merely-slow-but-healthy promotes on
/// a loaded machine. For calibration: the cold path's own window creation is
/// documented at ~2.5-3.5s, and a promoted window still has to run
/// `initHostNewWindow` (a full srv round trip) before it can register.
pub const PROMOTE_LIVENESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Armed post-promote liveness watches, keyed by window label.
///
/// Mirrors `PendingReprojectClosures`' shape (a thin named wrapper over a
/// map, so the logic is unit-testable without a live instance) with one
/// addition: a monotonic epoch per watch, so a late timer can't consume a
/// newer watch for the same label.
#[derive(Debug, Default)]
pub struct PromoteLivenessWatches {
    /// label → epoch of the currently-armed watch for that label.
    armed: HashMap<String, u64>,
    next_epoch: u64,
}

impl PromoteLivenessWatches {
    /// Arm a watch for `label`. Returns the epoch the caller must pass back
    /// to `take_if_unconfirmed` when its timer fires. Re-arming a label that
    /// already has a watch replaces it (and the older epoch is thereby
    /// invalidated — its timer becomes a no-op).
    pub fn arm(&mut self, label: String) -> u64 {
        self.next_epoch += 1;
        let epoch = self.next_epoch;
        self.armed.insert(label, epoch);
        epoch
    }

    /// Called from `register_backend_window` for EVERY registering label.
    /// Returns true iff this label had an armed watch — i.e. this
    /// registration is the liveness proof a promote was waiting for.
    /// `false` for every ordinary (non-promoted) window registration, which
    /// is the overwhelmingly common case.
    pub fn confirm(&mut self, label: &str) -> bool {
        self.armed.remove(label).is_some()
    }

    /// The watched window went away (closed by the user, or destroyed) before
    /// it ever confirmed. Returns true iff a watch was actually cancelled.
    ///
    /// Distinct from `confirm` only in intent and logging: confirm means
    /// "proved alive", cancel means "no longer exists, so there is nothing
    /// to replace". Both must stop the timer from opening a window — a
    /// deliberate close within the timeout is not the corpse scenario this
    /// feature targets (ReAgent P1 on PR #2987).
    pub fn cancel(&mut self, label: &str) -> bool {
        self.armed.remove(label).is_some()
    }

    /// Timer expiry. Returns true iff a watch for `label` at exactly
    /// `epoch` is still armed — meaning the promote was never confirmed and
    /// the caller should fall back. Removes it, so the fallback fires at
    /// most once. Returns false (without disturbing anything) when the
    /// watch was already confirmed, or when a newer promote for the same
    /// label has superseded this epoch.
    pub fn take_if_unconfirmed(&mut self, label: &str, epoch: u64) -> bool {
        match self.armed.get(label) {
            Some(&current) if current == epoch => {
                self.armed.remove(label);
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.armed.len()
    }
}

/// Pure decision: given an expired, unconfirmed watch, should the fallback
/// actually open a replacement window?
///
/// Split out of the timer thread so all three interlocks are testable
/// without a live instance.
///
/// - `unconfirmed` — the watch survived to expiry (see `take_if_unconfirmed`).
/// - `quit_state != Running` — a drain/quit began inside the timeout window.
///   Opening a window then would both surprise the user mid-shutdown and
///   race teardown with a cold-path spawn. The instance is going away
///   regardless; an unconfirmed promote no longer needs replacing.
/// - `browser_still_registered` — the promoted window still exists in the
///   host's browser registry. This is the LEVEL-TRIGGERED guard for
///   "the user closed (or crashed) the torn-off window inside the timeout"
///   (ReAgent P1 on PR #2987): a deliberate close is not the corpse scenario,
///   and replacing a window the user just dismissed is exactly the spurious
///   extra window this feature must not cause.
///
///   Checked here at the decision point rather than relying solely on
///   cancelling from each close path, deliberately: this codebase already
///   learned that lesson once, when an EDGE-triggered quit gate missed close
///   paths and orphaned the process tree — `reducer/quit.rs`'s header
///   documents the level-triggered `reconcile_quit` rewrite that replaced it.
///   A promoted window can leave through several paths (`on_before_close`,
///   Windows parking close, WRR crash-destroy, orphan cleanup), and this
///   check cannot miss one, now or after a future refactor.
///   `on_pool_window_destroyed` *also* cancels eagerly (cheap, and it closes
///   the narrow window where a close lands between expiry and this read),
///   but correctness does not depend on having found every such site.
pub fn should_open_fallback(
    unconfirmed: bool,
    quit_state: &super::QuitState,
    browser_still_registered: bool,
) -> bool {
    unconfirmed
        && browser_still_registered
        && matches!(quit_state, super::QuitState::Running)
}

#[cfg(test)]
mod promote_liveness_tests {
    use super::*;

    #[test]
    fn unconfirmed_watch_reports_for_fallback_on_expiry() {
        let mut w = PromoteLivenessWatches::default();
        let epoch = w.arm("window-pool-abc".to_string());
        assert!(
            w.take_if_unconfirmed("window-pool-abc", epoch),
            "a promote that never registered must report unconfirmed"
        );
    }

    #[test]
    fn confirmed_watch_does_not_fall_back() {
        let mut w = PromoteLivenessWatches::default();
        let epoch = w.arm("window-pool-abc".to_string());
        assert!(w.confirm("window-pool-abc"), "confirm reports it was armed");
        assert!(
            !w.take_if_unconfirmed("window-pool-abc", epoch),
            "a confirmed promote must never fall back when its timer fires"
        );
    }

    #[test]
    fn fallback_fires_at_most_once() {
        let mut w = PromoteLivenessWatches::default();
        let epoch = w.arm("window-pool-abc".to_string());
        assert!(w.take_if_unconfirmed("window-pool-abc", epoch));
        assert!(
            !w.take_if_unconfirmed("window-pool-abc", epoch),
            "a second expiry for the same watch must not spawn a second window"
        );
    }

    #[test]
    fn confirm_is_false_for_ordinary_window_registrations() {
        // main, cold-path windows, and never-promoted labels all flow
        // through register_backend_window — none of them may be mistaken
        // for a promote confirmation.
        let mut w = PromoteLivenessWatches::default();
        w.arm("window-pool-real".to_string());
        assert!(!w.confirm("main"));
        assert!(!w.confirm("window-cold-path"));
        assert_eq!(w.len(), 1, "unrelated registrations leave the real watch alone");
        assert!(w.confirm("window-pool-real"));
    }

    #[test]
    fn a_stale_timer_cannot_consume_a_newer_watch_for_the_same_label() {
        // The epoch guard: promote #1's timer fires AFTER promote #2 armed
        // a fresh watch for the same label. It must not consume #2's watch
        // (which would both skip #2's real protection and spawn a spurious
        // fallback window).
        let mut w = PromoteLivenessWatches::default();
        let stale = w.arm("window-pool-abc".to_string());
        let current = w.arm("window-pool-abc".to_string());
        assert_ne!(stale, current, "re-arming must allocate a fresh epoch");
        assert!(
            !w.take_if_unconfirmed("window-pool-abc", stale),
            "the stale timer must be a no-op"
        );
        assert!(
            w.take_if_unconfirmed("window-pool-abc", current),
            "the current watch must still be armed and able to fall back"
        );
    }

    #[test]
    fn watches_are_independent_per_label() {
        let mut w = PromoteLivenessWatches::default();
        let a = w.arm("window-pool-a".to_string());
        let b = w.arm("window-pool-b".to_string());
        assert_eq!(w.len(), 2);
        assert!(w.confirm("window-pool-a"));
        assert_eq!(w.len(), 1);
        assert!(
            !w.take_if_unconfirmed("window-pool-a", a),
            "confirmed label must not fall back"
        );
        assert!(
            w.take_if_unconfirmed("window-pool-b", b),
            "the other label's watch is unaffected by its sibling"
        );
    }

    #[test]
    fn expiry_for_a_never_armed_label_is_a_noop() {
        let mut w = PromoteLivenessWatches::default();
        assert!(!w.take_if_unconfirmed("window-pool-never", 1));
    }

    #[test]
    fn fallback_opens_only_for_an_unconfirmed_watch_on_a_running_instance() {
        use crate::state::QuitState;
        assert!(should_open_fallback(true, &QuitState::Running, true));
        // A confirmed promote never opens a replacement, whatever the
        // instance is doing.
        assert!(!should_open_fallback(false, &QuitState::Running, true));
    }

    #[test]
    fn fallback_never_opens_a_window_while_the_instance_is_shutting_down() {
        use crate::state::{QuitReason, QuitState};
        // The drain interlock: a quit that began inside the 10s timeout
        // window must not get a surprise window racing teardown.
        assert!(!should_open_fallback(
            true,
            &QuitState::Draining { reason: QuitReason::LastWindowClosed },
            true,
        ));
        assert!(!should_open_fallback(
            true,
            &QuitState::Draining { reason: QuitReason::LauncherRequested },
            true,
        ));
        assert!(!should_open_fallback(true, &QuitState::Quit, true));
    }

    /// ReAgent P1 on PR #2987: tear off a window, then close it (or it
    /// crashes) inside the 10s timeout, before it ever registered. That
    /// close is intentional, not the suspend/corpse scenario — replacing it
    /// would pop an unwanted window on a user who just dismissed one.
    #[test]
    fn fallback_never_replaces_a_window_the_user_already_closed() {
        use crate::state::QuitState;
        assert!(
            !should_open_fallback(true, &QuitState::Running, false),
            "a promoted window that is no longer registered must not be replaced"
        );
    }

    #[test]
    fn cancel_stops_the_timer_from_opening_a_window() {
        let mut w = PromoteLivenessWatches::default();
        let epoch = w.arm("window-pool-abc".to_string());
        assert!(w.cancel("window-pool-abc"), "cancel reports it was armed");
        assert!(
            !w.take_if_unconfirmed("window-pool-abc", epoch),
            "a cancelled watch must not fall back when its timer fires"
        );
    }

    #[test]
    fn cancel_is_false_for_a_window_that_was_never_promoted() {
        // on_pool_window_destroyed runs for PRE-promote pool deaths too —
        // those never armed a watch and must be a silent no-op.
        let mut w = PromoteLivenessWatches::default();
        assert!(!w.cancel("window-pool-never-promoted"));
    }
}
