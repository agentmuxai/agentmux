// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Close/lifecycle operations for `BrowserPaneManager`: `close`,
//! `close_with`, `drain_closed_label`, `replay_pending_create`, and the
//! Windows keyboard-focus-orphaning fix (`reclaim_focus_after_pane_destroy`).
//! Split out of `browser_panes.rs` — see that module's doc comment.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::{AppStateCloseOps, BrowserPaneCloseOps, BrowserPaneManager};

impl BrowserPaneManager {
    /// Close a pane by destroying its app-owned wrapper HWND (not CEF's own
    /// HWND, and never via `close_browser()`) and dropping the Browser Arc.
    ///
    /// We deliberately do **not** call `host.close_browser(force)`. Empirically
    /// (host-log trace v0.33.251 and v0.33.252 in `SPEC_BROWSER_PANE_LIFECYCLE.md`
    /// §4), CEF Alloy treats the pane Browser and the main Browser as a single
    /// close unit when the pane's outer HWND is a child of main's top-level:
    /// `close_browser(pane)` fires `do_close` on main too. Previous attempts
    /// (force=0, force=1, a cascade-guard cancelling main's do_close) either
    /// quit the whole app or orphaned the pane's pixels while blocking the
    /// pane's own teardown.
    ///
    /// As of `SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md` (and
    /// corrected per retro-browser-pane-renderer-leak-2026-07-07), instead:
    /// 1. Remove the Browser from `state.browsers` so subsequent lookups miss.
    /// 2. Reparent our app-owned wrapper HWND (`browser_pane::wrapper`) out to
    ///    a genuine top-level window, THEN Win32 `DestroyWindow` it — never
    ///    CEF's own HWND directly, and never `close_browser()`. The reparent
    ///    is load-bearing: destroying the wrapper while it was still a
    ///    `WS_CHILD` of main delivered `WM_DESTROY` via parent-hierarchy
    ///    tear-down, which CEF does NOT treat as a browser close — the
    ///    browser/renderer silently survived every close (live-confirmed via
    ///    CDP). Made top-level first, the destroy is structurally identical
    ///    to the floater's proven teardown and CEF runs its real close
    ///    pipeline. See `destroy_wrapper_hwnd`'s doc for the full mechanism.
    /// 3. Drop our `Browser` Arc. `on_before_close` fires via the
    ///    reparent-then-destroy teardown; `drain_closed_label` stays
    ///    idempotent as a backstop for any remaining async-timing edge case.
    ///
    /// Trade-off: because we bypass `close_browser`, Chromium's `beforeunload`
    /// handler doesn't run. Acceptable for a browser pane (no form data the
    /// user expects to persist across close). If beforeunload becomes
    /// important, revisit.
    pub fn close(&self, block_id: &str, state: &Arc<AppState>) {
        crate::browser_pane::trace::pane_trace(block_id, "close", "");
        // Phase H.1.d (PR #5) — sole pane-close entry point. The reducer
        // flips Live→Closing atomically and returns the entry's label iff
        // the transition fired. None means missing or already-Closing —
        // both idempotent no-ops; we don't dispatch CompleteBrowserPaneClose in
        // those cases (codex P2 PR #655 race), avoiding the entry removal
        // while another in-flight close is still tearing down the HWND.
        let close_out = state.host_dispatch(
            crate::reducer::HostCommand::EnqueueBrowserPaneClose {
                block_id: block_id.to_string(),
            },
        );
        let label = match close_out.closed_browser_pane_label {
            Some(l) => l,
            None => return,
        };

        // Drop this pane's media grants. Placed after the Live→Closing
        // transition fired (a None above means "missing or already closing" —
        // another close is mid-teardown and owns the cleanup), so it runs
        // exactly once per real close.
        //
        // Grants must not outlive their pane: they are pane-scoped precisely so
        // that a decision made for one pane cannot apply to another, and block
        // ids are not reused in practice — but if one ever were, a stale entry
        // would silently hand a new pane the previous occupant's camera grant.
        // SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md §3.2.
        state.media_grants.lock().clear_pane(block_id);
        crate::browser_panes::media_prompt::cancel_pane_any_thread(block_id);
        #[cfg(target_os = "windows")]
        {
            let ops = AppStateCloseOps(state);
            Self::close_with(&label, &ops);
            // Keyboard-focus orphaning fix: destroying the pane's native HWND
            // doesn't hand Win32 focus back, so if the pane held focus (common
            // on redock — the dragged pane is focused) keystrokes are orphaned
            // app-wide until something reclaims focus. Post a focus reclaim for
            // the surviving foreground window so its render widget (which hosts
            // the whole DOM UI — agent/terminal panes included) regains focus
            // and native panes are defocused. Mirrors the manual recovery the
            // user otherwise triggers by switching windows. See
            // ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md §1.
            Self::reclaim_focus_after_pane_destroy(state);
        }
        // Linux/macOS — Views path. Marshal the BrowserView detach onto the
        // CEF UI thread (remove_child_view is UI-thread-only); the underlying
        // Browser's on_before_close fires asynchronously and clears
        // state.browsers via the existing callback (callbacks::on_before_close_browser_pane).
        #[cfg(not(target_os = "windows"))]
        {
            let mut task = DetachBrowserPaneViewTask::new(state.clone(), label.clone());
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
        let close_out = state.host_dispatch(
            crate::reducer::HostCommand::CompleteBrowserPaneClose {
                block_id: block_id.to_string(),
            },
        );
        tracing::info!(block_id, label, "browser pane closed");

        // The explicit-close path removes the reducer entry here (not via the
        // async drain), so it must ALSO replay any create deferred while this
        // block_id was Closing — otherwise the deferred create is orphaned
        // (pane never loads + leak). The reducer removed the stash atomically
        // and handed it back here. reagent P1/P2 on PR #1168.
        self.replay_pending_create(state, close_out.pending_browser_pane_create_to_replay);

        // PR #6 H.7 kick — top up the pool now that this pane has closed.
        // `spawn_pool_window` is internally idempotent (single-flight +
        // below-target check), so calling on every pane close is safe.
        //
        // Cross-platform: the original `weak_ptr.h:250` race that prompted
        // an earlier Windows-only cfg-gate is gone. With the deferred
        // OverlayController destroy (see
        // browser_pane/creation_views.rs::detach_browser_pane_view),
        // close() no longer destroys the controller synchronously — it
        // just stashes it for on_before_close to destroy later. Creating
        // a new pool window here therefore can't race a synchronous
        // destroy of the just-closed pane's View. drain_closed_label's
        // pool kick can't be relied on as the sole refill source either:
        // CompleteBrowserPaneClose (dispatched above) already removed the
        // reducer entry, so DrainBrowserPaneByLabel inside
        // drain_closed_label is a no-op and never reaches its
        // spawn_pool_window() call (codex P2 on PR #788).
        crate::commands::window_pool::spawn_pool_window(state);
    }

    /// The testable side-effect body of `close()`. Given a pane's `label`,
    /// remove its Browser handle and destroy its HWND. The state-machine
    /// transition (Live→Closing) and the entry removal (CompleteBrowserPaneClose)
    /// happen in `close()` via reducer dispatch — `close_with` is purely
    /// the FFI side-effects that follow.
    fn close_with(label: &str, ops: &dyn BrowserPaneCloseOps) {
        if let Some(hwnd) = ops.take_browser_hwnd(label) {
            ops.destroy_hwnd(hwnd);
            tracing::info!(label, "pane wrapper destroy dispatched");
        }
    }

    /// Post a focus reclaim for the foreground window after a pane HWND was
    /// destroyed, so Win32 keyboard focus returns to that window's main render
    /// widget (which hosts the entire DOM UI — agent/terminal panes included)
    /// and native panes are defocused. Windows-only: the orphaning is a Win32
    /// native-child-window issue. The empty label tells `MainFocusReclaimTask`
    /// to resolve the foreground agentmux window itself, which is correct for
    /// both redock (the floater is gone → the target window is foreground) and
    /// an in-window close (that window stays foreground).
    #[cfg(target_os = "windows")]
    fn reclaim_focus_after_pane_destroy(state: &Arc<AppState>) {
        let mut task = crate::ui_tasks::MainFocusReclaimTask::new(state.clone(), String::new());
        cef::post_task(cef::ThreadId::UI, Some(&mut task));
    }

    /// Called from CEF's `on_before_close` if/when it fires for a pane
    /// browser. The explicit `close()` path usually clears the entry first,
    /// so this is a no-op in that case — but `on_before_close` may still
    /// fire async as Chromium's refcount hits zero, and `DrainBrowserPaneByLabel`
    /// is idempotent so the callback is safe.
    ///
    /// Also destroys the app-owned wrapper HWND (`browser_pane::wrapper`) if
    /// one is still registered for this label — reagent P1 on PR #1957: if
    /// CEF's `OnBeforeClose` fires WITHOUT `close()` having run first (a
    /// crash, or something else calling `close_browser()` directly on this
    /// pane), CEF tears down its own child HWND on its own initiative, but
    /// that never touches OUR wrapper (destroying a child never destroys its
    /// parent) — the wrapper would otherwise survive as a permanently
    /// orphaned, childless window with nothing left to ever clean it up,
    /// since the only other destroy site is `close_with` in the explicit
    /// path. `take_wrapper_hwnd` is a no-op `None` when `close()` already
    /// destroyed it, keeping this idempotent like the rest of the function.
    pub fn drain_closed_label(&self, state: &Arc<AppState>, label: &str) {
        #[cfg(target_os = "windows")]
        if let Some(wrapper_hwnd) = crate::browser_pane::wrapper::take_wrapper_hwnd(label) {
            crate::browser_pane::wrapper::destroy_wrapper_hwnd(wrapper_hwnd as *mut std::ffi::c_void);
        }

        let out = state.host_dispatch(
            crate::reducer::HostCommand::DrainBrowserPaneByLabel {
                label: label.to_string(),
            },
        );
        if let Some(block_id) = out.drained_browser_pane_block_id {
            tracing::info!(label, block_id = %block_id, "browser pane drained via on_before_close");

            // Drop media grants and pending prompts on THIS path too.
            //
            // This is the CEF-initiated teardown (crash, or the page/pane
            // closing itself), and it removes the reducer entry without going
            // through `close()` — so `close()`'s cleanup never runs for it, and
            // a later `close()` returns early because the entry is already
            // gone. Missing it would leave the grant behind, and
            // `replay_pending_create` below can immediately recreate a pane for
            // the SAME block id — handing the new pane the previous occupant's
            // camera/mic access with no prompt. Hence: before the replay, not
            // after (codex P2 on PR #2897).
            state.media_grants.lock().clear_pane(&block_id);
            crate::browser_panes::media_prompt::cancel_pane_any_thread(&block_id);
            // Same keyboard-focus orphaning fix as close() — the async
            // on_before_close drain also destroys the pane HWND.
            #[cfg(target_os = "windows")]
            Self::reclaim_focus_after_pane_destroy(state);
            // PR #6 H.7 kick — see `close()` for rationale. The
            // on_before_close path is the async drain; pool refill that
            // was deferred while the pane was Closing should now resume.
            crate::commands::window_pool::spawn_pool_window(state);

            // Deterministic redock re-create: the reducer removed the stash
            // atomically and handed back any create deferred while this
            // block_id was Closing. Replay it (now Fresh).
            self.replay_pending_create(state, out.pending_browser_pane_create_to_replay);
        }
    }

    /// Replay a browser-pane create that the reducer deferred while `block_id`
    /// was `Closing` and handed back on close-completion (via
    /// `DispatchOutput.pending_browser_pane_create_to_replay`). Called from
    /// BOTH close-completion paths — the async `drain_closed_label`
    /// (`DrainBrowserPaneByLabel`) and the explicit `close()`
    /// (`CompleteBrowserPaneClose`). No-op if nothing was deferred. The old
    /// entry is gone by now, so the replayed `create` gets `Fresh` and posts
    /// the `CreateBrowserPaneTask`. The stash lived in (and was removed from)
    /// the reducer's `HostState` under the host_state lock — no separate map,
    /// no TOCTOU.
    fn replay_pending_create(
        &self,
        state: &Arc<AppState>,
        pending: Option<(String, crate::state::PendingBrowserPaneCreate)>,
    ) {
        if let Some((block_id, p)) = pending {
            tracing::info!(block_id, "replaying deferred browser pane create after close");
            let rect = Rect { x: p.x, y: p.y, width: p.width, height: p.height };
            if let Err(e) = self.create(state, &block_id, &p.url, rect, &p.window_label) {
                tracing::warn!(block_id, error = %e, "deferred browser pane create replay failed");
            }
        }
    }
}

// ── UI-thread marshalling tasks ─────────────────────────────────────────────
//
// `Window::remove_child_view` must run on the CEF UI thread. `close()` is
// called from IPC handler tasks on tokio threads, so we wrap the UI-thread
// body in a `wrap_task!` struct and post it via `post_task(ThreadId::UI, ...)`
// — same pattern as `ui_tasks::CloseWindowTask` / `MaximizeWindowTask` / etc.
//
// The same constraint applies to the Windows path's Win32 calls:
// `DestroyWindow` only works from the thread that created the window (the CEF
// UI thread, via CreateBrowserPaneTask) — see `DestroyPaneWrapperTask` below
// and retro-browser-pane-renderer-leak-2026-07-07 for the leak that calling
// it from the IPC thread silently caused.

/// Windows pane-wrapper destroy, marshalled to the CEF UI thread (the
/// wrapper's owning thread — Win32 DestroyWindow is owner-thread-only).
/// Posted by `AppStateCloseOps::destroy_hwnd`; the actual
/// hide → reparent-to-top-level → DestroyWindow sequence lives in
/// `browser_pane::wrapper::destroy_wrapper_hwnd`.
#[cfg(target_os = "windows")]
wrap_task! {
    pub struct DestroyPaneWrapperTask {
        hwnd: isize,
    }

    impl Task {
        fn execute(&self) {
            crate::browser_pane::wrapper::destroy_wrapper_hwnd(
                self.hwnd as *mut std::ffi::c_void,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct DetachBrowserPaneViewTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            crate::browser_pane::creation_views::detach_browser_pane_view(
                &self.state, &self.label,
            );
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────
//
// Phase H.1.d/e (PR #5): The pane state machine lives in the host reducer
// (`HostState.browser_panes`). Lifecycle transition tests — Live→Closing, idempotent
// no-ops for missing or already-Closing entries, label sequence monotonicity,
// drain-by-label — are now in `crate::reducer::tests`.
//
// What remains here: the FFI seam. `close_with` only takes a label and
// drives `BrowserPaneCloseOps`; tests verify it forwards label → take → destroy
// in order, with a None-returning `take` short-circuiting the destroy.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Recording mock for `BrowserPaneCloseOps`. Tests inspect `taken` and
    /// `destroyed` to assert what close_with did.
    struct MockCloseOps {
        registered: parking_lot::Mutex<HashMap<String, usize>>,
        taken: parking_lot::Mutex<Vec<String>>,
        destroyed: parking_lot::Mutex<Vec<usize>>,
    }

    impl MockCloseOps {
        fn new() -> Self {
            Self {
                registered: parking_lot::Mutex::new(HashMap::new()),
                taken: parking_lot::Mutex::new(Vec::new()),
                destroyed: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn register(&self, label: &str, hwnd: usize) {
            self.registered.lock().insert(label.to_string(), hwnd);
        }

        fn taken_labels(&self) -> Vec<String> {
            self.taken.lock().clone()
        }

        fn destroyed_hwnds(&self) -> Vec<usize> {
            self.destroyed.lock().clone()
        }
    }

    impl BrowserPaneCloseOps for MockCloseOps {
        fn take_browser_hwnd(&self, label: &str) -> Option<usize> {
            self.taken.lock().push(label.to_string());
            self.registered.lock().remove(label)
        }

        fn destroy_hwnd(&self, hwnd: usize) {
            self.destroyed.lock().push(hwnd);
        }
    }

    #[test]
    fn close_with_take_then_destroy_in_order() {
        let ops = MockCloseOps::new();
        ops.register("browser-pane-b1-1", 0xABCD);

        BrowserPaneManager::close_with("browser-pane-b1-1", &ops);

        assert_eq!(ops.taken_labels(), vec!["browser-pane-b1-1"]);
        assert_eq!(ops.destroyed_hwnds(), vec![0xABCD]);
    }

    #[test]
    fn close_with_no_hwnd_skips_destroy() {
        // Browser was already gone (rare race — explicit close raced with
        // an external close). take returns None; destroy must NOT be called.
        let ops = MockCloseOps::new(); // no register() — lookup will miss

        BrowserPaneManager::close_with("browser-pane-missing", &ops);

        assert_eq!(ops.taken_labels(), vec!["browser-pane-missing"]);
        assert!(ops.destroyed_hwnds().is_empty(),
            "destroy_hwnd must not be called when take_browser_hwnd returns None");
    }
}
