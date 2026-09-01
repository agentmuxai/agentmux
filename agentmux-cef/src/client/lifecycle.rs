// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LifeSpanHandler methods for `AgentMuxHandler` — browser creation,
//! window/pool registration, cascade close, and the last-window quit gate.
//! Extracted verbatim from client/mod.rs.

use cef::*;

use crate::state::WindowKind;

use super::helpers::backend_close_window;
use super::{dlog, AgentMuxHandler};
#[cfg(target_os = "windows")]
use super::install_main_window_floater_cascade_hook;
#[cfg(target_os = "windows")]
use super::wndproc::{install_top_level_focus_restore_hook, set_window_icon, skip_taskbar};

/// Bounded retry window for the `backend_window_id` shadow-map lookup in
/// `on_before_close` — see docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md.
/// 5 attempts * 200ms comfortably covers the host->launcher->host
/// `register_backend_window` round trip observed in the 2026-07-04
/// pagefile-test session (windows promoted ~2s apart, no visible lag),
/// while staying well clear of user-perceived latency for a window that's
/// already closing.
pub(crate) const BACKEND_WINDOW_ID_RETRY_ATTEMPTS: u32 = 5;
pub(crate) const BACKEND_WINDOW_ID_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

/// Upper bound on `AgentMuxHandler::pending_popups` — no realistic pane has
/// this many OAuth sign-in popups mid-creation at once. Caps a leak if CEF
/// never fires `on_after_created` for a popup it declined to create.
const POPUP_PENDING_CAP: usize = 8;

/// Poll `lookup` up to `max_attempts` times, sleeping `delay` between each
/// attempt (via the injected `sleep` so tests don't have to wait in real
/// time), returning the first `Some` result or `None` if every attempt
/// misses. Extracted from `on_before_close`'s backend_window_id retry so
/// the race-closing behavior is unit-testable without a real CEF `Browser`.
pub(crate) fn retry_backend_window_id_lookup(
    max_attempts: u32,
    delay: std::time::Duration,
    mut lookup: impl FnMut() -> Option<String>,
    sleep: impl Fn(std::time::Duration),
) -> Option<String> {
    for _ in 0..max_attempts {
        sleep(delay);
        if let Some(window_id) = lookup() {
            return Some(window_id);
        }
    }
    None
}

impl AgentMuxHandler {
    pub(crate) fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let Some(browser) = browser.cloned() else {
            tracing::error!("[on_after_created] browser is None — skipping registration");
            return;
        };
        tracing::info!("Browser created (total: {})", self.browser_list.len() + 1);

        // Each OAuth popup allowed by on_before_popup is created (as the next
        // browser(s) on this pane's handler) — record its id so do_close can
        // close its hosting Views window (the stray-blank-window fix). Counter,
        // not a bool, so two popups in flight both get tagged. `is_popup` then
        // routes it away from the full-window treatment below (classified as
        // BrowserKind::Popup, excluded from the quit watchdog by type, and
        // skipping the focus-restore / OS-close-routing / floater-cascade hooks
        // and launcher FullInstance registration a real top-level gets — that
        // OS-close-routing hook was also what prevented the popup's own window
        // from closing cleanly).
        let is_popup = self.pending_popups > 0;
        if is_popup {
            self.pending_popups -= 1;
            let mut b = browser.clone();
            self.popup_browser_ids.insert(b.identifier());
            tracing::info!(
                target: "oauth-popup",
                popup_id = b.identifier(),
                "[oauth-popup] step 3/4: on_after_created — tagged + classified BrowserKind::Popup (EXCLUDED from last-window quit gate; skips full-window hooks)",
            );
        }

        // Phase 1 diagnostic tracing — find the exact line that silences the
        // UI thread under concurrent window creation. See
        // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md.
        let t0 = std::time::Instant::now();

        // Phase B.5 (window_meta step d) — pop the pre-create
        // handoff entry. Pre-step-d this was a label-only queue +
        // separate `window_meta.insert` from the caller; now it's
        // a single `PendingWindowCreation` carrying label + kind +
        // parent_instance_id, eliminating the parallel-write race
        // between caller and on_after_created.
        //
        // First-browser shortcut: "main" never has a pre-create
        // handoff (host startup spawns it directly), so we
        // synthesize a FullInstance entry. Subsequent windows pop
        // their entry; if the queue is empty (legacy paths /
        // unexpected races) fall back to a generated UUID label
        // with FullInstance defaults.
        // Phase H.2.b — reducer-aware emptiness check with fallback.
        let pending = if is_popup {
            // An OAuth popup is created by CEF via on_before_popup returning
            // false — it has NO `EnqueuePendingWindowCreation` entry of its own.
            // It must therefore NOT run the shared DequeuePendingWindowCreation
            // below: doing so would consume a *concurrent* legitimate
            // window/pane/pool's queued entry, leaving that real creation to
            // fall back to a synthesized random-UUID FullInstance and lose its
            // true kind/parent_instance_id (reagent P1 round 3 on #2545).
            // Synthesize a popup-labelled entry locally instead; the `is_popup`
            // gates below route it away from all top-level/FullInstance
            // treatment regardless of this label.
            crate::state::PendingWindowCreation {
                label: format!("popup-{}", uuid::Uuid::new_v4()),
                kind: WindowKind::FullInstance,
                parent_instance_id: None,
            }
        } else if self.state.browsers_is_empty() {
            crate::state::PendingWindowCreation {
                label: "main".to_string(),
                kind: WindowKind::FullInstance,
                parent_instance_id: None,
            }
        } else {
            // Phase F.1 — dequeue via the host reducer. The reducer
            // emits PendingWindowQueueEmpty on miss; the fallback
            // (synthesize a UUID-labelled FullInstance entry) lives
            // in the legacy code path it always has.
            tracing::info!(
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[on-after-created] dispatching DequeuePendingWindowCreation"
            );
            let out = self
                .state
                .host_dispatch(crate::reducer::HostCommand::DequeuePendingWindowCreation);
            tracing::info!(
                elapsed_us = t0.elapsed().as_micros() as u64,
                dequeued_some = out.dequeued.is_some(),
                "[on-after-created] DequeuePendingWindowCreation returned"
            );
            out.dequeued.unwrap_or_else(|| {
                let lbl = format!("window-{}", uuid::Uuid::new_v4());
                tracing::warn!(label = %lbl, "[on_after_created] no pending creation entry — defaulting to FullInstance");
                crate::state::PendingWindowCreation {
                    label: lbl,
                    kind: WindowKind::FullInstance,
                    parent_instance_id: None,
                }
            })
        };
        let label = pending.label.clone();
        let pending_kind = pending.kind;
        let pending_parent = pending.parent_instance_id.clone();

        // Phase H.2.d — legacy `state.browsers.insert` removed. Reducer's
        // `RegisterBrowser` (dispatched below) is now the sole canonical
        // mutation site. Smoke test on 0.33.585 verified parallel-write
        // parity (zero drift across 18 RegisterBrowser/Unregister pairs).
        let total = self.state.host_state.lock().browsers.len() + 1;
        tracing::info!(
            label = %label,
            elapsed_us = t0.elapsed().as_micros() as u64,
            total,
            "[on-after-created] registering browser via reducer",
        );
        dlog(&format!("on_after_created: registered label={} total={}", label, total));

        let is_top_level_window = !label.starts_with("browser-pane-");

        // Determine BrowserKind from the LABEL prefix, not the
        // AgentMuxClient `is_browser_pane` flag. Smoke test on 0.33.586 found
        // top-level windows misclassified as `Pane { block_id: "" }`
        // because `CreateWindowTask::execute` reuses an existing
        // browser's CEF Client via `first_browser()` — if the iteration
        // happens to pick a pane, the new window inherits `is_browser_pane=true`
        // and the label-stripping in this branch produces an empty
        // block_id (since the label starts with `window-` not
        // `browser-pane-`). LABEL is the source of truth. See
        // docs/retro/smoke-test-0.33.586-and-pr5-plan-2026-05-02.md.
        //
        // Classification (LABEL is the source of truth at registration; after
        // this the typed `BrowserKind` is authoritative — don't re-parse labels):
        //   - `browser-pane-<uuid>-<seq>`            → Pane { block_id: uuid }
        //   - `floating-<uuid>` / `floating-pool-<uuid>` → Floater { is_pool }
        //       (is_pool=true only while a pane-pool floater is still warm /
        //        unpromoted; a direct floater registers visible → is_pool:false)
        //   - `window-pool-*` still unpromoted        → TopLevel { is_pool: true }
        //   - everything else (main, window-*, promoted pool windows) →
        //     TopLevel { is_pool: false }
        // Floaters get their own variant so the last-window quit gate excludes
        // them BY TYPE (invariant FP-LIFE) instead of a `floating-pool-` string
        // check that missed direct `floating-<uuid>` floaters. Check `floating-`
        // BEFORE `window-pool-` (a `floating-pool-` label also starts `floating-`).
        let kind = if is_popup {
            // Transient OAuth sign-in popup — CEF owns its window; excluded
            // from the last-window quit gate by type.
            crate::state::BrowserKind::Popup
        } else if let Some(rest) = label.strip_prefix("browser-pane-") {
            let block_id = rest
                .rfind('-')
                .map(|i| rest[..i].to_string())
                .unwrap_or_default();
            crate::state::BrowserKind::Pane { block_id }
        } else if label.starts_with("floating-") {
            let is_pool = label.starts_with("floating-pool-")
                && self.state.is_unpromoted_pane_pool_label(&label);
            crate::state::BrowserKind::Floater { is_pool }
        } else if label.starts_with("window-pool-")
            && self.state.is_unpromoted_pool_label(&label)
        {
            crate::state::BrowserKind::TopLevel { is_pool: true }
        } else {
            crate::state::BrowserKind::TopLevel { is_pool: false }
        };
        self.state.host_dispatch(
            crate::reducer::HostCommand::RegisterBrowser {
                label: label.clone(),
                browser: browser.clone(),
                kind,
            },
        );

        // Phase B.5 (window_meta step d, refined) — write host's
        // local `window_meta` ONCE here, synchronously from the
        // popped pending entry. This is no longer the authoritative
        // state (the launcher's `state.windows` is); it's a
        // host-internal cache that covers two scenarios where the
        // launcher-fed shadow can't:
        //
        // 1. `task dev` mode — no launcher IPC at all, shadow stays
        //    empty forever. open_subwindow's parent validation +
        //    cascade-close need a synchronous local source.
        // 2. Cascade-close race — child opens just before parent
        //    closes; on_after_created→ReportWindowOpened→launcher
        //    →WindowOpened→shadow round-trip hasn't completed by
        //    the time parent's on_before_close runs. Without the
        //    local write, `subwindow_children_of` would miss the
        //    child and skip cascade close.
        //
        // The retired piece (step d's intent) is the
        // **caller-side parallel write** — drag/window/window_pool
        // no longer write meta themselves. Single canonical
        // mutation site here. (codex P1 PR #592 round-2.)
        if is_top_level_window && !is_popup {
            let mut metas = self.state.window_meta.lock();
            metas.insert(
                label.clone(),
                crate::state::WindowMeta {
                    label: label.clone(),
                    kind: pending_kind,
                    parent_instance_id: pending_parent.clone(),
                },
            );
        }

        // No DwmExtendFrameIntoClientArea — it causes the white flash.
        // CEF Views handles frameless + resize via its delegate.

        // Set the taskbar/title bar icon from the embedded exe resource, and
        // for `Subwindow` top-levels, hide them from the taskbar via
        // ITaskbarList::DeleteTab.
        #[cfg(target_os = "windows")]
        {
            // Prefer CEF Views' `Window::window_handle()` — it targets the
            // specific top-level window for THIS browser, avoiding the
            // `find_own_top_level_window` fallback's "first visible HWND"
            // ambiguity when multiple windows exist.
            let mut browser_mut = browser.clone();
            let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser_mut))
                .and_then(|bv| bv.window())
                .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                .filter(|p| !p.is_null());

            let hwnd = views_top_hwnd.unwrap_or_else(|| {
                browser.host()
                    .and_then(|h| {
                        let wh = h.window_handle();
                        if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                    })
                    .unwrap_or_else(|| unsafe {
                        crate::commands::window::find_own_top_level_window()
                    })
            });

            if !hwnd.is_null() {
                unsafe { set_window_icon(hwnd); }

                // Subclass for the focus-restore-on-WM_ACTIVATE behavior
                // (window-reactivate-focus-restore spec §5.1.3). Observes
                // WM_ACTIVATE only; all messages pass through to CEF.
                // Install on every top-level — both `main` and Subwindow.
                if is_top_level_window && !is_popup {
                    unsafe { install_top_level_focus_restore_hook(hwnd); }

                    // Shift+window-edge resize (spec SPEC_RESIZE_DEFAULT_FLIP_
                    // AND_WINDOW_EDGE_SHIFT_2026_08_26.md §3.4): observe the
                    // native size loop (WM_SIZING/WM_EXITSIZEMOVE) and forward
                    // windowresize:* events to this window's renderer. Pure
                    // observer-passthrough — safe on main and Subwindows alike.
                    unsafe {
                        super::wndproc::install_window_edge_resize_hook(
                            &self.state,
                            hwnd,
                            &label,
                        );
                    }
                }

                // OS-close routing (task #30): Alt+F4 / taskbar-close
                // deliver WM_CLOSE straight to the Views wndproc, which
                // parks the browser with no srv cleanup — the same defect
                // class close_window_by_label had (#2087), via the OS
                // entry point. Reroute WM_CLOSE on every SECONDARY
                // `window-*` top-level through CloseWindowTask. NOT main:
                // main's OS-close feeds the tuned WRR last-window quit
                // sequence (Pillar 2), which owns process shutdown. Not
                // floaters: their outer-popup wndproc close works (#1957).
                if is_top_level_window && !is_popup && label.starts_with("window-") {
                    unsafe {
                        super::wndproc::install_window_close_routing_hook(
                            &self.state,
                            hwnd,
                            &label,
                        );
                    }
                }

                // Floater cascade hook (issue #1560): replaces the Win32
                // owned-window z-order/minimize/destroy invariant now that
                // floaters are unowned WS_POPUP windows. Install on all
                // FullInstance windows (not pool, not subwindow) so that
                // closing or minimizing any main window cascades to its floaters.
                #[cfg(target_os = "windows")]
                if is_top_level_window
                    && !is_popup
                    && pending_kind == WindowKind::FullInstance
                    && !label.starts_with("window-pool-")
                    && !label.starts_with("floating-pool-")
                {
                    unsafe { install_main_window_floater_cascade_hook(hwnd); }
                }

                // Subwindow? Hide from taskbar. Full instances and browser-pane
                // child HWNDs skip this branch.
                if is_top_level_window {
                    // Phase B.5 (window_meta step d) — read kind from
                    // the pending entry we just popped. No
                    // window_meta lookup, no race window.
                    if pending_kind == WindowKind::Subwindow {
                        unsafe { skip_taskbar(hwnd); }
                    }
                }
            }
        }

        // Pane-specific on_after_created work (Z-order raise + Win32 focus
        // subclass install) lives in `crate::browser_pane::callbacks` after Phase 4
        // of the modularization split.
        if self.is_browser_pane {
            crate::browser_pane::callbacks::on_after_created_browser_pane(&self.state, &browser);
        }

        // Phase B.4 — report top-level windows to the launcher's
        // read-only state mirror. Skips browser-pane child HWNDs,
        // tab pool windows (`window-pool-*`), and pane pool windows
        // (`floating-pool-*`). Pane pool windows are excluded here AND
        // in `host_counts_snapshot` (state.rs) so the launcher mirror's
        // windows count stays in sync with the host count on all platforms.
        // No-op if launcher IPC isn't connected (`task dev` mode).
        if is_top_level_window && !is_popup && !label.starts_with("window-pool-") && !label.starts_with("floating-pool-") {
            // Phase B.5 (window_meta step d) — kind/parent come
            // from the pending entry we popped at the top of this
            // fn, not a window_meta lookup.
            let wire_kind = match pending_kind {
                WindowKind::FullInstance => agentmux_common::ipc::WindowKind::FullInstance,
                WindowKind::Subwindow => agentmux_common::ipc::WindowKind::Subwindow,
            };
            crate::launcher_ipc::report_window_opened(label.clone(), wire_kind, pending_parent.clone());

            // Phase B.9.1 (WRR) — authoritative HWND link. We have
            // both the label (popped from PendingWindowCreation
            // above) and the native HWND (computed in the
            // #[cfg(target_os = "windows")] block above as
            // `views_top_hwnd` / `hwnd`). Sending an explicit
            // ReportHwndOpened with `label_hint = Some(label)` here
            // eliminates the race between the OS-driven
            // EVENT_OBJECT_CREATE (which my hook captures with
            // `label_hint = None` because pending_window_creations
            // may already have been popped by the time the OS event
            // bubbles back) and CEF's lifecycle. The OS-event path
            // still runs as belt-and-suspenders for non-CEF windows
            // / future detection of strays. (The prior pending_hwnds
            // entry from the OS event is harmless — it ages out on
            // the next event-driven reconciliation pass.)
            #[cfg(target_os = "windows")]
            {
                // Recompute the HWND here — the prior #[cfg] block
                // computed `hwnd` as a local that's not in scope at
                // this site. The CEF Browser API is cheap to query
                // a second time. Precedence: Views' window handle →
                // host's window handle. NO fallback to
                // `find_own_top_level_window()` — that function uses
                // `EnumWindows` and returns the FIRST visible window
                // belonging to this process, which in a multi-window
                // session is some OTHER window's HWND. Sending that
                // as authoritative `Some(label)` would corrupt the
                // OTHER label's mirror via the `Repaired` arm in
                // `apply_hwnd_opened`. (reagent P1 PR #664 round 3.)
                //
                // If both Views and host return null (transient
                // lifecycle case), skip the explicit dispatch. The
                // launcher's drain-on-WindowOpened fallback links
                // the recent pending HWND from WM_CREATE — that's
                // the sole link path when `hwnd_val=0`. The drain
                // is reliable when WM_CREATE arrived recently (within
                // the launcher's 2s age limit); the only failure mode
                // is no WM_CREATE-pending entry within that window,
                // in which case the mirror stays hwnd=None — same
                // outcome as pre-PR-664 for that edge case, no worse.
                let mut browser_for_wrr = browser.clone();
                let views_hwnd = browser_view_get_for_browser(Some(&mut browser_for_wrr))
                    .and_then(|bv| bv.window())
                    .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                    .filter(|p| !p.is_null());
                let host_hwnd = browser.host().and_then(|h| {
                    let wh = h.window_handle();
                    if wh.0.is_null() {
                        None
                    } else {
                        Some(wh.0 as *mut std::ffi::c_void)
                    }
                });
                let hwnd_val = views_hwnd.or(host_hwnd).map(|p| p as u64).unwrap_or(0);
                if hwnd_val != 0 {
                    crate::launcher_ipc::report_hwnd_opened(
                        hwnd_val,
                        "Chrome_WidgetWin_1".to_string(),
                        label.clone(),
                        Some(label.clone()),
                    );
                } else {
                    // Both sources null. Launcher's drain-on-WindowOpened
                    // fallback should still link the pending HWND from
                    // WM_CREATE; if that race lost too, the mirror
                    // stays hwnd=None for this window — degraded but
                    // not corrupted. Log at WARN so the regression is
                    // visible if it happens.
                    tracing::warn!(
                        target: "wrr",
                        label = %label,
                        "[wrr] on_after_created: hwnd_val=0 from both Views and host — \
                         relying on launcher's pending_hwnds drain fallback"
                    );
                }
            }
            // Phase B.4 follow-up — drift check after the open.
            crate::launcher_ipc::compute_and_report_host_counts(&self.state);
        }

        self.browser_list.push(browser);

        // Tear-off Phase 6 — pre-warmed window pool.
        // - When the "main" window registers, kick off the initial pool spawn.
        // - When a "window-pool-*" window registers, log only — actual
        //   queue insertion waits for the frontend's renderer-ready IPC
        //   so emit_event_to_window doesn't race the listener install.
        if label == "main" {
            crate::commands::window_pool::init_pool(&self.state);
            crate::commands::window_pool::init_pane_pool(&self.state);

            // SPEC_PILLAR1_STEP4 Phase 2 fix — "main" registering here is
            // our proof CEF's UI-thread message loop is actually pumping
            // posted tasks (see `ui_thread_gate`'s doc comment for why this
            // matters: `post_task(ThreadId::UI, ...)` silently drops tasks
            // posted before this point). Flip `ready` and take `stashed`
            // under the SAME lock acquisition the launcher-ipc reader task
            // uses to check-then-stash — this is what closes the TOCTOU
            // reagent's review caught in the first version of this fix.
            //
            // SPEC_PILLAR1_STEP4 Phase 3 — this is also the decision point
            // for fast-vs-slow path: if no fast-path snapshot has arrived by
            // the time "main" registers (no launcher connected, or the
            // launcher's own snapshot response hasn't landed yet), the slow
            // path should run — but not from here. `reproject_from_srv`
            // needs "main"'s own confirmed srv `window_id`, which isn't
            // known yet at this point (native browser creation happens well
            // before the frontend loads and calls `register_backend_window`
            // — see `pending_slow_path`'s doc comment for why an earlier
            // version's `windowids[0]` positional guess was wrong). So this
            // only sets `pending_slow_path`; `register_backend_window`
            // (`commands/window/meta.rs`) is what actually triggers it, once
            // it has that id in hand.
            //
            // A stash existing is NOT the same as the fast path having
            // anything useful: `Event::Snapshot` always stashes SOMETHING
            // when it arrives before `ready` (even an empty list, or a list
            // containing only a stale `"main"` entry — the launcher-ipc arm
            // doesn't pre-filter). `has_extra` is what actually distinguishes
            // "fast path found real data" from "otherwise" per the spec's
            // §2.1 step 3-4 — checked live: a fresh launcher (full
            // process-tree kill) sends a real, non-stale `Event::Snapshot`
            // with `window_count=0`, which a naive `stashed.is_some()` check
            // wrongly treated as "fast path succeeded," permanently
            // suppressing the slow path.
            let (action, stashed, stashed_backend_window_ids) = {
                let mut gate = self.state.ui_thread_gate.lock();
                let stashed = gate.stashed.take();
                let stashed_backend_window_ids = gate.stashed_backend_window_ids.take();
                let has_extra = stashed
                    .as_ref()
                    .is_some_and(|windows| windows.iter().any(|w| w.label != "main"));
                (gate.on_main_ready(has_extra), stashed, stashed_backend_window_ids)
            };
            if action == crate::state::MainReadyAction::ReplayFastPath {
                let windows = stashed.unwrap_or_default();
                let backend_window_ids = stashed_backend_window_ids.unwrap_or_default();
                tracing::info!(
                    target: "reproject",
                    window_count = windows.len(),
                    "[reproject] replaying stashed snapshot now that \"main\" has registered"
                );
                crate::commands::window::reproject_from_snapshot_and_stage_closures(
                    &self.state,
                    &windows,
                    &backend_window_ids,
                );
            }
        } else if label.starts_with("window-pool-") {
            crate::commands::window_pool::register_pool_window(&self.state, &label);
        } else if label.starts_with("floating-pool-") {
            crate::commands::window_pool::register_pane_pool_window(&self.state, &label);
        }
    }

    pub(crate) fn do_close(&mut self, browser: Option<&mut Browser>) -> bool {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Close-cascade diagnostics (window-lifecycle-leak retro round 3,
        // 2026-07-05): do_close firing means CEF actually initiated browser
        // destruction — the missing middle link when secondary-window closes
        // leak the browser/renderer.
        dlog(&format!(
            "do_close fired; browser_list.len()={}",
            self.browser_list.len()
        ));

        // Auth popup (OAuth/GIS) tearing down (its own window.close(), or the
        // opener closing it after the postMessage hand-off).
        //
        // Under the shipping **Chrome runtime**, CEF owns the popup's window
        // and closes it itself — there's no AgentMux Views window, so
        // `.window()` is None and this is a clean no-op (confirmed live via the
        // [oauth-popup] trace: step 2 `on_popup_browser_view_created` never
        // fires and the popup closes on its own). The explicit close is the
        // belt-and-suspenders path for an **Alloy/Views-hosted** popup (other
        // runtimes / future) where CEF does NOT auto-close the window and it
        // would otherwise linger blank ("stray Sign In window"; reagent P1 on
        // PR #2545). Either way the pane and main window are untouched — this
        // browser is a distinct popup, not the pane's own frame.
        if let Some(b) = browser {
            let id = b.identifier();
            // CHECK membership only — do NOT remove here. do_close fires before
            // on_before_close for the same browser, and on_before_close relies
            // on the id still being present to tell a popup's own self-close
            // (was_popup=true → no cascade) apart from the PANE closing
            // (was_popup=false → cascade-close remaining popups). Removing here
            // made a self-closing popup look like the pane and force-close a
            // sibling in-progress popup (reagent P1 round 4 on #2545).
            if self.popup_browser_ids.contains(&id) {
                match browser_view_get_for_browser(Some(b)).and_then(|v| v.window()) {
                    Some(mut win) => {
                        tracing::info!(
                            target: "oauth-popup",
                            popup_id = id,
                            "[oauth-popup] step 4/4: do_close — closing the popup's own Views window (Alloy/Views path)",
                        );
                        win.close();
                    }
                    None => tracing::info!(
                        target: "oauth-popup",
                        popup_id = id,
                        "[oauth-popup] step 4/4: do_close — Chrome-runtime popup closes itself; nothing to do",
                    ),
                }
            }
        }

        if self.browser_list.len() == 1 {
            self.is_closing = true;
        }
        // Return false to allow the close.
        false
    }

    /// Intercept `target="_blank"` / `window.open()` from embedded pages so
    /// they don't spawn rogue top-level CEF windows.
    ///
    /// Routing, by who fired the popup and where it points:
    /// * **App UI → external site** (Help pane's GitHub / docs / Discord
    ///   links): open in the **system browser** and cancel. Navigating the app
    ///   window itself to an external origin replaces the AgentMux UI and
    ///   strands it on "Can't reconnect" (`SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17.md`).
    /// * **Browser pane → OAuth authorization popup** (a `window.open`
    ///   popup-disposition request whose URL is an OAuth/OIDC authorize
    ///   endpoint — `is_oauth_authorization_url`): allow CEF to create a REAL
    ///   child popup (return false) so the `window.opener`/`postMessage`/cookie
    ///   handshake completes the sign-in in the pane. Scoped tightly to auth
    ///   URLs so this does NOT re-open the "rogue popup window" bug
    ///   (`SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md`) for
    ///   arbitrary `window.open` popups.
    /// * **Browser pane → any other popup** (non-auth `window.open`): open in
    ///   the **system browser** if external (no rogue in-app window, no
    ///   hijacking the pane's frame), else cancel.
    /// * **Browser pane → `target="_blank"` link, or internal URL**: navigate
    ///   the **current** frame — in a pane, following a link IS the point.
    ///
    /// **The `load_url` call is deferred via `post_task`**, not run inline.
    /// Inline `load_url` caused a UI-thread deadlock on link click:
    /// `on_before_popup` runs while `AgentMuxLifeSpanHandler` holds
    /// `self.inner.lock()` (via the wrap macro). Inline `load_url` starts
    /// a new navigation on the same UI thread, which triggers
    /// `on_loading_state_change` on `AgentMuxLoadHandler`, which also
    /// tries to take `self.inner.lock()` → deadlock. The host hung with
    /// backend heartbeats still running but the whole UI frozen. Posting
    /// the `load_url` as a separate UI task lets the popup handler
    /// return, release the lock, then pick up the load on the next loop
    /// iteration.
    pub(crate) fn on_before_popup(
        &mut self,
        browser: Option<&mut Browser>,
        _frame: Option<&mut Frame>,
        target_url: Option<&CefString>,
        target_disposition: WindowOpenDisposition,
    ) -> bool {
        let url = target_url.map(|s| s.to_string()).unwrap_or_default();
        if url.is_empty() {
            // Nothing useful to navigate to; just cancel the popup.
            return true;
        }

        // A real popup window (`window.open` with popup features) rather than a
        // `target="_blank"` link (which arrives as a tab disposition). OAuth /
        // Google Identity Services sign-in, payment, and similar handshake
        // windows use these — they postMessage a result back to their opener
        // and then call `window.close()` when done.
        let is_popup_window = {
            let d = target_disposition.get_raw();
            d == WindowOpenDisposition::NEW_POPUP.get_raw()
                || d == WindowOpenDisposition::NEW_WINDOW.get_raw()
        };

        let is_external = crate::commands::platform::is_external_http_url(&url);

        if self.is_browser_pane && is_popup_window {
            // **OAuth authorization popup only.** Let CEF create an ACTUAL
            // child popup (return false) so the sign-in completes IN the pane:
            // the popup shares the pane's browser/request context, so
            // `window.opener` points at the pane's page, `postMessage` delivers
            // the credential back to it, and cookies/session are shared — none
            // of which work when the popup is a separate process (the "There
            // was an error logging you in" symptom of routing it to the system
            // browser). The popup's own `window.close()` closes only the popup
            // (its hosting Views window is closed in do_close via
            // popup_browser_ids), not the pane.
            //
            // SECURITY GATE — the native popup is allowed ONLY when BOTH hold:
            //   1. the target host is a **known identity provider**
            //      (`is_known_idp_host`), and
            //   2. the URL is an OAuth/OIDC authorization request
            //      (`is_oauth_authorization_url`).
            //
            // Condition 1 is the real boundary. Browser panes load
            // untrusted/attacker pages; URL-shape heuristics alone let any such
            // page spawn unlimited native phishing popups (reagent P1 on #2545,
            // the popup-explosion class SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_
            // 2026_04_21.md prevents). An attacker can't serve from
            // accounts.google.com / github.com / *.okta.com / …, so gating on a
            // known-IdP host makes the native popup safe — a real sign-in always
            // targets the provider's host. A self-hosted / unlisted IdP simply
            // doesn't get the in-pane popup and falls through to the system
            // browser below.
            let popup_host = crate::commands::platform::url_host(&url);
            let is_trusted_idp = popup_host
                .as_deref()
                .map(crate::commands::platform::is_known_idp_host)
                .unwrap_or(false);
            if is_trusted_idp && crate::commands::platform::is_oauth_authorization_url(&url) {
                tracing::info!(
                    target: "oauth-popup",
                    url = %url,
                    popup_host = ?popup_host,
                    "[oauth-popup] step 1/4: on_before_popup — trusted-IdP OAuth URL, allowing native CEF child popup (returning false)",
                );
                // Count the popup about to be created; the matching number of
                // subsequent on_after_created calls on this handler tag their
                // browser ids for managed close (counter, not a bool, so two
                // popups in flight both get tagged — reagent P2 on #2545).
                // Bounded so a popup CEF never actually creates (resource
                // exhaustion / pane destroyed mid-creation → no on_after_created
                // to decrement) can't leak the counter unboundedly; it's also
                // reset to 0 when the pane closes (on_before_close). A pane's
                // handler only ever creates popups after its pane, so even a
                // stale count would at worst mis-tag another popup (still a
                // popup) — the cap+reset bounds it fully (reagent P2 round 5).
                self.pending_popups = self.pending_popups.saturating_add(1).min(POPUP_PENDING_CAP);
                return false; // allow CEF to create the popup browser
            }

            // Any OTHER pane popup (non-auth window.open): don't create a rogue
            // in-app window and don't hijack the pane's own frame. Open it in
            // the system browser if external; otherwise cancel.
            if is_external {
                match crate::commands::platform::open_url_in_default_browser(&url) {
                    Ok(()) => tracing::info!(url = %url, "non-auth browser-pane popup opened in system browser"),
                    Err(e) => tracing::warn!(url = %url, error = %e, "failed to open pane popup in system browser"),
                }
            }
            return true; // cancel the in-app popup
        }

        // **App UI → external site** (Help pane's "Report Bugs & Issues",
        // docs, Discord, …): open in the SYSTEM browser and cancel. Navigating
        // the app's own window to an external origin replaces the whole
        // AgentMux UI and tears down the host bridge — the window comes back
        // bridge-dead on "Can't reconnect".
        //
        // `open_url_in_default_browser` only spawns a child process (rundll32 /
        // open / xdg-open); it never re-enters CEF or `self.inner`, so calling
        // it inline here (under the handler lock) cannot deadlock the way an
        // inline `load_url` would.
        if !self.is_browser_pane && is_external {
            match crate::commands::platform::open_url_in_default_browser(&url) {
                Ok(()) => tracing::info!(url = %url, "external link opened in system browser"),
                Err(e) => tracing::warn!(
                    url = %url,
                    error = %e,
                    "failed to open external link in system browser",
                ),
            }
            return true; // cancel popup; do NOT navigate the app frame
        }

        if let Some(b) = browser {
            let browser_clone = b.clone();
            let mut task = crate::ui_tasks::DeferredLoadUrlTask::new(
                browser_clone,
                url.clone(),
            );
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
        tracing::info!(
            is_browser_pane = %self.is_browser_pane,
            url = %url,
            "popup intercepted — deferred navigation of current frame",
        );
        true // cancel the top-level popup creation
    }

    /// External-protocol guard (RequestHandler::on_before_browse). Returns 1 to
    /// CANCEL the navigation, 0 to allow.
    ///
    /// Chromium's default handling of a navigation to a scheme it doesn't own
    /// (a non-web external protocol) is to hand it to the OS shell —
    /// `ShellExecute` on Windows — which launches the OS-registered handler for
    /// that scheme. If that handler is an elevated target, Windows raises a
    /// **UAC** prompt. We never want embedded web content — a browser pane
    /// loading arbitrary sites especially — to reach an OS protocol handler
    /// this way (see the report). For panes we cancel any navigation whose
    /// scheme isn't web-ish (`is_disallowed_pane_nav_scheme`). The main app
    /// client is served from loopback http and is left unrestricted so no
    /// internal (devtools/app) navigation regresses.
    pub(crate) fn on_before_browse(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        request: Option<&mut Request>,
        _user_gesture: ::std::os::raw::c_int,
        is_redirect: ::std::os::raw::c_int,
    ) -> ::std::os::raw::c_int {
        if !self.is_browser_pane {
            return 0; // main app client — never gated
        }
        let url = request
            .as_ref()
            .map(|r| CefString::from(&r.url()).to_string())
            .unwrap_or_default();
        if crate::commands::platform::is_disallowed_pane_nav_scheme(&url) {
            tracing::warn!(
                url = %url,
                "on_before_browse: blocked browser-pane navigation to a non-web external scheme (OS-handoff / UAC guard)",
            );
            return 1; // cancel — do not let CEF hand this to the OS shell
        }

        // Track the pane-load watchdog HERE, not from `on_loading_state_change`
        // — `request.url()` is this navigation's own target, independent of
        // whatever the frame's last COMMITTED document was. Using
        // `Frame::url()` instead (an earlier version did) shows the pane's
        // PREVIOUS page in the eventual timeout error page instead of the
        // one actually being navigated to, because a frame that never
        // committed this navigation still reports its old URL right up until
        // the watchdog fires. See `browser_pane::callbacks::arm_pane_load_watchdog`.
        //
        // `is_redirect` gates WHICH update: a fresh navigation (0) gets a new
        // epoch + a full new deadline; a redirect hop of an ALREADY-armed
        // navigation (nonzero) only updates the reported URL, keeping the
        // original deadline. Treating every hop as a fresh arm (an earlier
        // version of this fix did) let a redirect chain reset the 20s timer
        // on each hop, so the actual wait before a timeout page appeared
        // could grow far past the intended bound (reagentx P1 on PR #2593).
        // See `browser_pane::callbacks::update_pane_load_watchdog_url`.
        let is_main_frame = frame.as_ref().map(|f| f.is_main() == 1).unwrap_or(false);
        if is_main_frame {
            if let Some(b) = browser.as_deref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    // Layer 1, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md:
                    // the main frame is (still) loading whether this is a
                    // fresh navigation or a redirect hop — no-ops (via the
                    // insert-already-present check) if a hop finds it
                    // already true. Must run before `url` is moved into the
                    // watchdog calls below.
                    //
                    // Deliberately passes the frame's CURRENT committed URL
                    // (`Frame::url()`), NOT `url` (this navigation's own
                    // pending TARGET, used below for the watchdog) — every
                    // `browser-pane-nav-state` event's `url` field
                    // unconditionally drives the frontend's `UrlConfirmed`
                    // dispatch regardless of this event's actual purpose
                    // being only the loading-spinner signal. Passing the
                    // pending target jumped the address bar to where the
                    // navigation is headed before it commits — and on a
                    // redirect chain, the insert-already-present no-op above
                    // means only the FIRST hop's target would ever be sent
                    // (subsequent hops don't change the tracked state), so
                    // the bar would get stuck on that first intermediate hop
                    // for the whole chain. Passing the current (pre-this-
                    // navigation) URL instead makes this event's UrlConfirmed
                    // dispatch a harmless no-op — reagent P2 on PR #2642.
                    let current_url = frame
                        .as_ref()
                        .map(|f| CefString::from(&f.url()).to_string())
                        .unwrap_or_default();
                    crate::browser_pane::callbacks::set_pane_main_frame_loading(
                        &self.state,
                        &block_id,
                        b,
                        &current_url,
                        true,
                    );
                    if is_redirect != 0 {
                        crate::browser_pane::callbacks::update_pane_load_watchdog_url(
                            &self.state,
                            &block_id,
                            url,
                        );
                    } else {
                        crate::browser_pane::callbacks::arm_pane_load_watchdog(
                            &self.state,
                            &block_id,
                            b.clone(),
                            url,
                        );
                    }
                }
            }
        }
        0
    }

    /// Post `quit_message_loop()` back to the UI thread from a background
    /// thread, once that thread's `backend_close_window` notify work is
    /// done. Never call `quit_message_loop()` directly off the UI thread —
    /// see `ui_tasks::QuitMessageLoopTask`'s doc comment. Fixes the
    /// last-window close race — docs/retro/retro-last-window-close-quit-race-2026-07-16.md.
    fn quit_after_backend_notify() {
        let mut task = crate::ui_tasks::QuitMessageLoopTask::new();
        let posted = post_task(ThreadId::UI, Some(&mut task));
        if posted == 0 {
            // post_task can fail during teardown if the UI thread's message
            // loop already tore down its task queue (see the doc note at
            // lifecycle.rs:416 for the analogous close_browser case). Nothing
            // recoverable to do — the process is already on its way out one
            // way or another; log so a future investigation isn't blind.
            tracing::warn!(target: "wrr", "[wrr] quit_after_backend_notify: post_task(UI) failed — process is likely already exiting");
        }
    }

    pub(crate) fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Phase B.9.3 — diagnostic trace at debug level. Filtered
        // out in production (default RUST_LOG=info). Enable via
        // RUST_LOG="info,wrr-trace=debug" when investigating
        // close-cascade issues.
        tracing::debug!(
            target: "wrr-trace",
            "[trace] on_before_close ENTER; self.browser_list.len()={} is_browser_pane={}",
            self.browser_list.len(), self.is_browser_pane
        );
        dlog(&format!("on_before_close fired; browser_list.len()={}", self.browser_list.len()));

        let Some(mut browser) = browser.cloned() else {
            // CEF can pass None during emergency teardown (e.g. process
            // shutdown while a browser is still closing). Log and bail —
            // a panic here SIGABRTs CrBrowserMain, which the launcher
            // mistakes for a crash and relaunches the app.
            tracing::error!("[on_before_close] browser is None — skipping close logic");
            return;
        };

        // Drop any crash-history entry for this browser — it's closing
        // cleanly so its budget is reset. Without this the map would
        // accumulate one stale entry per closed browser over a session.
        self.crash_history.remove(&browser.identifier());
        self.memory_pause_history.remove(&browser.identifier());
        // Remove this browser's popup tag HERE (not in do_close — do_close only
        // checks membership). `closing_was_popup` then reliably distinguishes a
        // popup's own self-close (true) from the PANE closing (false), which the
        // cascade below depends on.
        let closing_id = browser.identifier();
        let closing_was_popup = self.popup_browser_ids.remove(&closing_id);

        // Cascade-close orphaned OAuth popups when THIS PANE closes. The popup
        // shares the pane's handler, so its browser id lives in
        // popup_browser_ids; if the user closes the pane (or the workspace) while
        // a sign-in popup is still open, nothing else would close it and it would
        // survive as an orphaned top-level window — the exact failure
        // SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md prevents (reagent
        // P1 round 2 on #2545). The closing browser is the PANE (not a popup
        // itself) when its id was NOT in popup_browser_ids; force-close every
        // still-tracked popup, deferred via post_task (never call close_browser
        // inline from on_before_close — it re-enters CEF and hangs the UI thread,
        // the reason ClosePoolBrowserTask exists).
        if !closing_was_popup && self.is_browser_pane && !self.popup_browser_ids.is_empty() {
            let popup_ids: Vec<i32> = self.popup_browser_ids.drain().collect();
            for pid in popup_ids {
                if let Some(popup) = self.browser_list.iter().find(|b| {
                    let mut b = (*b).clone();
                    b.identifier() == pid
                }) {
                    tracing::info!(
                        target: "oauth-popup",
                        pane_closing = true,
                        popup_id = pid,
                        "[oauth-popup] pane closing — force-closing its still-open OAuth popup",
                    );
                    let mut task = super::ClosePoolBrowserTask::new(popup.clone());
                    cef::post_task(cef::ThreadId::UI, Some(&mut task));
                }
            }
            // The pane is gone — no popup can still be pending on its handler.
            // Clears any leaked increment (reagent P2 round 5 counter-leak).
            self.pending_popups = 0;
        }

        // Unregister browser from the reducer's `browsers` map and get its
        // label. Phase H.2.d — legacy `state.browsers.lock().remove` removed;
        // reducer is sole source of truth (see PR #4 commit 2 H.2.c flip).
        // Find-by-identity loop now iterates reducer-backed snapshot via
        // `state.list_browsers()`, then dispatches `UnregisterBrowser`.
        let snapshot = self.state.list_browsers();
        let keys: Vec<&String> = snapshot.iter().map(|(k, _)| k).collect();
        dlog(&format!("browsers map keys: {:?}", keys));
        let label = snapshot
            .iter()
            .find(|(_, b)| {
                let mut b = b.clone();
                b.is_same(Some(&mut browser)) != 0
            })
            .map(|(k, _)| k.clone());
        dlog(&format!("label found: {:?}", label));
        // Pillar 2 Stage 2 (SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md §3.2/§4#3)
        // — `UnregisterBrowser` is quit-relevant, so `reducer::update` already
        // computed `reconcile_quit` under the same lock and surfaced its
        // verdict here. Capture it now (state.browsers doesn't change again
        // before the gate below reads it) instead of re-deriving
        // `count_live_user_windows() == 0` locally — `reconcile_quit` is the
        // single decision authority; this handler is now just an executor.
        let mut request_drain: Option<crate::state::QuitReason> = None;
        if let Some(ref lbl) = label {
            let unregister_dispatch = self.state.host_dispatch(
                crate::reducer::HostCommand::UnregisterBrowser { label: lbl.clone() },
            );
            request_drain = unregister_dispatch.request_drain;
            let remaining = self.state.host_state.lock().browsers.len();
            tracing::info!(
                "Unregistered browser: label={} (remaining: {})",
                lbl,
                remaining
            );

            // Evict this label's HWND from `window_hwnds`. The cache
            // has no other cleanup path, and the resolver's hot-path
            // hits it before walking the registry — without this,
            // a subsequent open of the same label (e.g. main
            // restart) leaves a stale entry that breaks WM_CLOSE
            // routing. See
            // docs/specs/SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md.
            // Windows-only because `AppState::window_hwnds` is itself
            // `#[cfg(target_os = "windows")]` in `state.rs`. Codex P1
            // on PR #1133.
            #[cfg(target_os = "windows")]
            {
                let removed = self.state.window_hwnds.lock().remove(lbl);
                if removed.is_some() {
                    tracing::debug!(
                        target: "win-resolve",
                        label = %lbl,
                        "[win-resolve] evicted on close"
                    );
                }
            }

            // Co-evict the floater's window-placement entry (pane-state
            // reducer). Floaters key `pane_window_states` by window label,
            // and they're NOT in `browser_panes`, so this close hook — the
            // same place `window_hwnds` is evicted — is the correct cleanup
            // site. Gated to `floating-` labels (the only ones that ever
            // hold an entry); the reducer arm is itself idempotent/no-op if
            // absent. See SPEC_PANE_STATE_REDUCER_2026-05-28.md (REVISION
            // 2026-05-29).
            if lbl.starts_with("floating-") {
                self.state.host_dispatch(
                    crate::reducer::HostCommand::EvictFloatingPaneWindowState {
                        label: lbl.to_string(),
                    },
                );

                // Restore the WndProcs the Ctrl+Wheel hook subclassed and drop
                // its bookkeeping. Windows reuses HWND values, so leaving stale
                // entries behind would let a future window inherit a hook whose
                // context points at a dead floater. See floater_wheel.rs.
                #[cfg(target_os = "windows")]
                unsafe {
                    crate::floater_wheel::remove_floater_ctrl_wheel_hook(lbl);
                }
            }
        }

        // A credential-approval subwindow (opened via `open_subwindow`,
        // `initial_view=credential-approval`) closing before the human
        // decided — parent window closing cascades down to it same as any
        // other subwindow. No-op (empty Vec) for every ordinary window
        // close; the registry only ever has entries keyed by an actual
        // approval subwindow's label. See `credential_broker::approval`'s
        // own doc comment — this is the mirror of `browser_pane::auth`'s
        // pane-close cleanup below, just window-close instead of
        // pane-close.
        if let Some(ref lbl) = label {
            let cancelled = crate::credential_broker::approval::cancel_for_window(lbl);
            if !cancelled.is_empty() {
                use cef::ImplAuthCallback;
                tracing::info!(
                    "[credential-broker] approval window {} closed before a decision — \
                     cancelling {} parked auth request(s)",
                    lbl,
                    cancelled.len(),
                );
                for request_id in &cancelled {
                    if let Some(cb) = crate::browser_pane::auth::take(request_id) {
                        cb.cancel();
                    }
                }
            }
        }

        // Pane-specific on_before_close work (drain lifecycle entry) lives
        // in `crate::browser_pane::callbacks` after Phase 4.
        if let Some(ref lbl) = label {
            if lbl.starts_with("browser-pane-") {
                crate::browser_pane::callbacks::on_before_close_browser_pane(&self.state, lbl);
            }
            // Pool-window cleanup — release the respawn semaphore +
            // drop the label from the queue if the window died before
            // promote (renderer crash, OS-level close). Without this
            // the pool would never refill.
            if lbl.starts_with("window-pool-") {
                crate::commands::window_pool::on_pool_window_destroyed(&self.state, lbl);
            } else if lbl.starts_with("floating-pool-") {
                crate::commands::window_pool::on_pane_pool_window_destroyed(&self.state, lbl);
            }
            // Phase B.4 — mirror the close to the launcher. Skip
            // browser-pane child HWNDs (never reported as open).
            // For everything else, send unconditionally: the launcher
            // reducer silently no-ops on unknown labels (codex P2
            // PR #577 round-2 made `WindowClosed` strictly paired
            // with `WindowOpened`), so pre-promote pool deaths and
            // post-pop / pre-validation orphans are filtered there
            // — no host-side guard needed. Pool inventory updates
            // travel via `ReportPoolWindowRemoved` from
            // `on_pool_window_destroyed` and `promote_pool_window`.
            if !lbl.starts_with("browser-pane-") {
                crate::launcher_ipc::report_window_closed(lbl.clone());
                // Phase B.4 follow-up — drift check after the close.
                crate::launcher_ipc::compute_and_report_host_counts(&self.state);
            }
        }

        // Phase B.5 (window_id_map step d) — host no longer mutates
        // `window_id_map`. The launcher's `state.backend_window_ids`
        // (B.5 step a) is the sole authority; we look up the wid via
        // the shadow-first helper before notifying the launcher to
        // drop it.
        //
        // The immediate lookup below is a best-effort first try, kept
        // for the dlog trace and the common case. It is NOT assumed
        // reliable on its own: a window promoted and closed in rapid
        // succession (confirmed in the 2026-07-04 pagefile-test
        // session, docs/retro/retro-window-lifecycle-leak-2026-07-04.md)
        // can reach `on_before_close` before the host→launcher→host
        // `register_backend_window` round trip has populated the
        // shadow map. When the immediate check misses, the retry
        // below — on the same background thread `backend_close_window`
        // already runs on, never the UI thread — gives that race a
        // bounded chance to resolve before we give up. See
        // docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md.
        //
        // IMPORTANT (reagent P1 on PR #1965): `report_backend_window_id_unregistered`
        // is deliberately NOT called here, unconditionally, the way it used
        // to be. That report tells the launcher to drop its own canonical
        // `backend_window_ids[label]` entry and broadcasts
        // `BackendWindowIdUnregistered`, which purges this host's shadow
        // map too (launcher_ipc.rs). Firing it before we know whether the
        // retry below will need that very entry would race the unregister
        // against the pending register — exactly the case the retry
        // exists to recover from. It is now called once the outcome is
        // known, at each of the call sites below (immediate success,
        // retry success, retry exhausted) — never before.
        let backend_window_id = label.as_deref().and_then(|lbl| {
            let wid = self.state.backend_window_id(lbl);
            dlog(&format!("backend_window_id({:?}) => {:?}", lbl, wid));
            wid
        });

        // Pull and remove the closing window's meta; if it's a FullInstance,
        // cascade-close every Subwindow whose parent_instance_id points to it.
        // See `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md` §2.3.
        //
        // Phase B.5 (window_meta step d, refined) — read closing
        // meta via shadow-first helper, drop the host-side cache
        // entry (single canonical mutation site for window_meta
        // post-refinement: insert in on_after_created, remove here).
        let closing_meta = label
            .as_deref()
            .and_then(|lbl| self.state.window_meta(lbl));
        if let Some(lbl) = label.as_deref() {
            self.state.window_meta.lock().remove(lbl);
        }
        if let Some(meta) = &closing_meta {
            if meta.kind == WindowKind::FullInstance {
                let child_labels = self.state.subwindow_children_of(&meta.label);
                for child_label in child_labels {
                    // Phase H.2.b — reducer-aware lookup with fallback.
                    if let Some(mut child) = self.state.get_browser(&child_label) {
                        if let Some(host) = child.host() {
                            tracing::info!(parent = %meta.label, child = %child_label, "[subwindow-cascade] closing sub-window");
                            host.close_browser(1);
                        }
                    }
                }
            }
        }

        // Phase F.6 — narrate the pane-reap step for the launcher's
        // window-cleanup-cascade saga. By the time we reach here, the
        // pane lifecycle drain (`on_before_close_browser_pane` for browser-
        // pane labels) and the subwindow cascade above have run for
        // this label. The saga uses this signal as the Step 1
        // terminal so it can advance to Step 2 (drain-pool decision).
        //
        // Skip for browser-pane labels: the saga is triggered by
        // `Event::WindowClosed`, which only fires for non-pane
        // top-level windows; emitting `PanesReaped` for pane labels
        // would be a stray report (no in-flight saga to consume it).
        // Same gate as `report_window_closed` above — skip
        // browser-pane-* labels (sub-views, not top-level windows).
        // Don't filter window-pool-* here: filtering on prefix would
        // wrongly suppress promoted pool windows (which keep the
        // `window-pool-*` prefix but ARE tracked windows). Stray
        // events for unpromoted-pool drains are emitted but harmless
        // — no F.6 saga is in flight to consume them.
        if let Some(ref lbl) = label {
            if !lbl.starts_with("browser-pane-") {
                crate::launcher_ipc::report_panes_reaped(lbl.clone());
            }
        }

        dlog(&format!("backend_window_id: {:?}", backend_window_id));

        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }

        dlog(&format!("browser_list after remove: {}", self.browser_list.len()));

        // App-exit decision (authoritative): count remaining live USER windows
        // by the per-browser `BrowserKind::is_pool` flag, NOT pool
        // set-membership. The closing browser was already removed from the
        // reducer's `browsers` map above (`UnregisterBrowser`), so this reflects
        // what REMAINS.
        //
        // Why the flag, not `user_visibility_snapshot()`'s pool-SET count
        // (which this used to use, and which is kept below for logging only):
        // the snapshot excludes labels found in `pool.unpromoted ∪ pool.queue`.
        // If a pool window left those SETS without its `is_pool` flag clearing
        // (a failed/partial promote, an out-of-band drop), the snapshot counted
        // it as user-visible while it was really a hidden scratch window —
        // `user_browser_count` never hit 0, `BeginDrain` never fired, and the
        // host never quit. That is the orphaned-process-tree regression
        // (confirmed: 9,483-line orphan host log with no drain marker). The
        // `is_pool` flag is the single source of truth, flipped atomically at
        // promote (`pool.rs`) and read here under one lock, so it can't drift or
        // race a concurrent promote the way the two-set read can.
        //
        // A PROMOTED pool window keeps its `window-pool-*` label but is
        // `is_pool: false`, so it still correctly counts; unpromoted pool
        // windows (`is_pool: true`) and `BrowserKind::Pane` children don't. See
        // SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md §5.1/§10.1.
        let user_browser_count = self.state.count_live_user_windows();

        // Snapshot retained for the diagnostic trace below only (label lists);
        // the gate above is the authoritative `is_pool` count.
        let (browsers_keys, pool_keys) = {
            let (pool_labels, browsers) = self.state.user_visibility_snapshot();
            let keys: Vec<String> = browsers.into_iter().map(|(l, _)| l).collect();
            let pool: Vec<String> = pool_labels.into_iter().collect();
            (keys, pool)
        };

        // Phase B.9.3 diagnostic — fires for every close (incl.
        // pane closes). Demoted to debug for production. Enable
        // via RUST_LOG="info,wrr-trace=debug" to see per-close
        // gate input when investigating close-cascade issues.
        tracing::debug!(
            target: "wrr-trace",
            "[trace] app-exit gate: closing_label={:?} user_count={} is_browser_pane={} browsers={:?} unpromoted_pool={:?}",
            label, user_browser_count, self.is_browser_pane, browsers_keys, pool_keys
        );

        // Phase F.6 — narrate the post-close pool-drain decision for
        // the launcher's window-cleanup-cascade saga. The saga's
        // Step 2 terminal: `was_last == true` → `Event::PoolDrained`
        // (the wrr two-stage cascade below kicked off Stage 1's
        // pool drain); `was_last == false` → `Event::PoolNotLast`
        // (other windows remain; pool stays warm). Both close the
        // saga's `SagaStarted` bracket successfully — the saga's job
        // is to narrate the decision, not enforce a particular
        // outcome.
        //
        // Same skip-pane gate as `report_panes_reaped` above: the
        // saga is triggered by `Event::WindowClosed`, which only
        // fires for non-pane top-level windows. Pane closes don't
        // start a saga, so the report would be a no-op stray on the
        // bus.
        //
        // Computed here (BEFORE the wrr two-stage cascade below) so
        // the same condition the cascade gates on is what gets
        // reported. The boolean flag captures intent — Stage 1 may
        // not have started yet by the time the report is sent, but
        // the decision itself is final.
        if let Some(ref lbl) = label {
            // Same gate as report_panes_reaped above: skip
            // browser-pane-* only. window-pool-* labels (promoted)
            // are tracked windows and need their cleanup events.
            if !lbl.starts_with("browser-pane-") {
                // Pillar 2 Stage 2 — mirrors the actual cascade gate below
                // (`request_drain.is_some()`), not the raw `user_browser_count`
                // reading, so this report and the real decision can't drift
                // apart. `request_drain` is strictly more conservative than
                // `user_browser_count == 0` alone (it's also `None` while a
                // user window-creation is in flight, or once already
                // draining) — see reconcile_quit's should_begin_drain.
                let was_last = request_drain.is_some() && !self.is_browser_pane;
                crate::launcher_ipc::report_pool_drain_decision(lbl.clone(), was_last);
            }
        }

        // ── Phase B.9.3 — two-stage close cascade ─────────────────
        //
        // Stage 1: If user_browser_count just dropped to 0 (last
        // user-visible window closed), POST WM_CLOSE to every pool
        // browser. Async — the message loop processes the closes on
        // subsequent iterations. We do NOT call quit_message_loop
        // here. Calling it from inside on_before_close DEADLOCKS the
        // UI thread (smoke v0.33.498 confirmed: log line "calling
        // quit_message_loop now" was last; loop never returned).
        //
        // Stage 2: When self.browser_list becomes empty AFTER
        // removing this browser (i.e. every browser this handler
        // ever managed has closed), THEN call quit_message_loop.
        // Matches the canonical cefsimple pattern. By then there
        // are no other in-flight CEF lifecycle events to deadlock
        // against. The MAIN client's handler is the only one that
        // owns top-level windows + pool windows, so this fires
        // exactly when the entire app's CEF browser inventory is
        // gone.
        //
        // Cross-platform note: the Stage 1 PostMessage is the
        // Windows path. macOS uses NSWindow.performClose:; Linux
        // uses X11 WM_DELETE_WINDOW. Same async-close-cascade
        // semantics on all platforms; only the OS API differs.
        // Pillar 2 Stage 2 — the decision ("should we drain?") now comes
        // solely from `reconcile_quit` (via the `request_drain` captured
        // above from the `UnregisterBrowser` dispatch); this handler only
        // executes it. `!self.is_browser_pane` stays as a belt-and-suspenders
        // guard (a browser-pane close should never drive top-level app-quit
        // logic), though in practice `request_drain` is already `None` for a
        // pane close — `count_live_user_windows` only counts
        // `BrowserKind::TopLevel{is_pool:false}`, never panes.
        if !self.is_browser_pane {
            if let Some(reason) = request_drain {
                self.begin_drain_and_cascade(reason);
            }
        }

        // Stage 2: every browser this handler ever managed is now
        // gone. Safe to call quit_message_loop — no other CEF
        // lifecycle is in flight that could deadlock with it.
        //
        // 2026-07-16 fix (docs/retro/retro-last-window-close-quit-race-2026-07-16.md):
        // this branch used to call quit_message_loop() and NOTHING ELSE —
        // the backend-notify block below lived only in the `else` arm, so
        // closing the LAST window never even attempted backend_close_window
        // for it. srv never heard about the close, its window/workspace/tab
        // row leaked permanently, and crash-reproject faithfully resurrected
        // it as a "ghost" window on every subsequent launch. The notify
        // logic below now runs UNCONDITIONALLY (last window or not); when
        // this is the last window, quit_message_loop is deferred until that
        // notify work finishes, posted back to the UI thread (it must run
        // there — see the comment above about deadlocking on a nested call).
        let is_last_window = self.browser_list.is_empty() && !self.is_browser_pane;

        // Phase B.7.3.3 — `Event::WindowClosed` +
        // `Event::WindowInstanceReleased` from the launcher
        // drive remaining renderers' InstancePanel atoms via the
        // CEF JS bridge; no sync emit here.

        // Tell the backend to clean up this window's workspace/tabs/shells.
        // This replaces the JavaScript `beforeunload` handler — running it here
        // ensures shells die after the CEF browser is gone (not while it's still
        // alive), so Task Manager keeps them grouped until they exit.
        if let Some(window_id) = backend_window_id {
            let web_endpoint = self.state.backend_endpoints.lock().web_endpoint.clone();
            let auth_key = self.state.auth_key.lock().clone();
            // Safe to report here (reagent P1 on PR #1965): we already
            // resolved the window_id, so there's no pending retry left
            // to race against.
            let unregister_lbl = label.clone();
            dlog(&format!("spawning backend_close_window thread for window_id={}", window_id));
            std::thread::spawn(move || {
                backend_close_window(&web_endpoint, &auth_key, &window_id);
                if let Some(lbl) = unregister_lbl {
                    crate::launcher_ipc::report_backend_window_id_unregistered(lbl);
                }
                if is_last_window {
                    tracing::warn!(target: "wrr", "[wrr] stage 2: backend notified — posting quit_message_loop to UI thread");
                    Self::quit_after_backend_notify();
                }
            });
        } else if let Some(lbl) = label.clone() {
            // The immediate shadow lookup missed. This is expected and
            // permanent for pre-promote pool-window churn (never had a
            // backend_window_id, never will) — but it's also exactly
            // what happens when a window is promoted and closed fast
            // enough that the host->launcher->host register_backend_window
            // round trip hasn't landed yet, confirmed in the 2026-07-04
            // pagefile-test session (docs/retro/retro-window-lifecycle-leak-2026-07-04.md).
            // Retry on this same background thread (never the UI thread)
            // for a bounded window before giving up — closes that race
            // without a larger reconciliation mechanism. See
            // docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md.
            let state = self.state.clone();
            let web_endpoint = self.state.backend_endpoints.lock().web_endpoint.clone();
            let auth_key = self.state.auth_key.lock().clone();
            std::thread::spawn(move || {
                let sleep_fn = |d: std::time::Duration| std::thread::sleep(d);
                match retry_backend_window_id_lookup(
                    BACKEND_WINDOW_ID_RETRY_ATTEMPTS,
                    BACKEND_WINDOW_ID_RETRY_DELAY,
                    || state.backend_window_id(&lbl),
                    sleep_fn,
                ) {
                    Some(window_id) => {
                        dlog(&format!(
                            "backend_window_id({:?}) resolved on retry — spawning backend_close_window",
                            lbl
                        ));
                        backend_close_window(&web_endpoint, &auth_key, &window_id);
                        // Report the unregister only now (reagent P1 on
                        // PR #1965) — the retry just succeeded using
                        // this mapping, so it's safe to tell the
                        // launcher to drop it.
                        crate::launcher_ipc::report_backend_window_id_unregistered(lbl);
                    }
                    None => {
                        let warn = format!(
                            "[on_before_close] no backend window ID registered for label={:?} after {} retries ({}ms) — shells may orphan",
                            lbl,
                            BACKEND_WINDOW_ID_RETRY_ATTEMPTS,
                            BACKEND_WINDOW_ID_RETRY_ATTEMPTS * BACKEND_WINDOW_ID_RETRY_DELAY.as_millis() as u32
                        );
                        dlog(&warn);
                        tracing::warn!("{}", warn);
                        // Retries exhausted — nothing left to protect
                        // against racing; report the unregister so the
                        // launcher's bookkeeping doesn't keep a stale
                        // entry for a label that's now definitely gone.
                        crate::launcher_ipc::report_backend_window_id_unregistered(lbl);
                    }
                }
                if is_last_window {
                    tracing::warn!(target: "wrr", "[wrr] stage 2: backend notify attempt finished — posting quit_message_loop to UI thread");
                    Self::quit_after_backend_notify();
                }
            });
        } else {
            if !self.is_browser_pane {
                let warn = format!(
                    "[on_before_close] no backend window ID registered for label={:?} — shells may orphan",
                    label
                );
                dlog(&warn);
                tracing::warn!("{}", warn);
            }
            // Browser-pane handlers reaching here with label=None are the
            // DESIGNED post-explicit-close state, not an orphan risk: the
            // pane close path (browser_panes::close → take_browser_hwnd)
            // already unregistered the label before CEF's on_before_close
            // fires, and panes never have a backend_window_id to begin
            // with. Warning here would fire once per pane close now that
            // the wrapper teardown actually runs CEF's close pipeline
            // (retro-browser-pane-renderer-leak-2026-07-07) — pure noise.
            //
            // No async work was started above (no backend_window_id, no
            // label to retry against), so if this was the last window,
            // quit_message_loop is safe to call directly, right here on
            // the UI thread — no need to post it.
            if is_last_window {
                tracing::warn!(target: "wrr", "[wrr] stage 2: no backend notify possible (no label) — calling quit_message_loop directly");
                quit_message_loop();
                tracing::warn!(target: "wrr", "[wrr] quit_message_loop returned");
            }
        }

        tracing::debug!(
            target: "wrr-trace",
            "[trace] on_before_close EXIT label={:?} self.browser_list.len()={}",
            label, self.browser_list.len()
        );
    }

    /// Pillar 2 Stage 2 (SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md §3.2) —
    /// the Stage-1 drain-and-cascade executor, extracted verbatim from what
    /// used to be `on_before_close`'s inline `user_browser_count == 0` block
    /// so it's callable from any UI-thread CEF callback that observes
    /// `reconcile_quit`'s decision via `DispatchOutput.request_drain` — not
    /// just the close-edge that happened to trigger it this time. This is
    /// the "action" half of the decision/action split (`reducer/quit.rs:49-54`):
    /// callers must have already confirmed `request_drain.is_some()` (the
    /// DECISION) before calling this (the ACTION) — this function does not
    /// re-check anything, it just executes.
    ///
    /// Only flips `QuitState` and closes the (already-hidden) pool browsers.
    /// Never calls `quit_message_loop()` — that stays Stage 2, gated
    /// separately on `self.browser_list.is_empty()` in `on_before_close`,
    /// since calling it from inside another browser's `on_before_close`
    /// deadlocks the UI thread (confirmed v0.33.498).
    pub(crate) fn begin_drain_and_cascade(&self, reason: crate::state::QuitReason) {
        // Body extracted to `ui_tasks::begin_drain_and_cascade` (Pillar 2
        // sanitize-then-decide Phase 0, §1.G) so the executor is callable from
        // any UI-thread context holding `&Arc<AppState>`, not just this
        // handler. Zero behavior change — the handler only ever touched
        // `self.state`.
        crate::ui_tasks::begin_drain_and_cascade(&self.state, reason);
    }
}

#[cfg(test)]
mod backend_window_id_retry_tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    // No-op sleep — these tests exercise attempt counting and result
    // selection, not real timing, so they run instantly.
    fn no_sleep(_d: Duration) {}

    #[test]
    fn resolves_on_a_later_attempt_within_the_budget() {
        // Simulates the confirmed race: the shadow map is empty on the
        // first couple of checks (registration still in flight) and then
        // populates — exactly what should now recover instead of orphaning
        // the window, per docs/retro/retro-window-lifecycle-leak-2026-07-04.md.
        let calls = RefCell::new(0u32);
        let result = retry_backend_window_id_lookup(5, Duration::from_millis(1), || {
            *calls.borrow_mut() += 1;
            if *calls.borrow() >= 3 {
                Some("wid-123".to_string())
            } else {
                None
            }
        }, no_sleep);
        assert_eq!(result, Some("wid-123".to_string()));
        assert_eq!(*calls.borrow(), 3, "should stop retrying as soon as it resolves");
    }

    #[test]
    fn resolves_on_the_very_last_attempt() {
        let calls = RefCell::new(0u32);
        let result = retry_backend_window_id_lookup(5, Duration::from_millis(1), || {
            *calls.borrow_mut() += 1;
            if *calls.borrow() == 5 {
                Some("wid-last".to_string())
            } else {
                None
            }
        }, no_sleep);
        assert_eq!(result, Some("wid-last".to_string()));
    }

    #[test]
    fn gives_up_after_exhausting_all_attempts() {
        // The permanent, benign case (pre-promote pool-window churn):
        // never had a backend_window_id and never will. Must still give
        // up cleanly after exactly `max_attempts` tries, not loop forever.
        let calls = RefCell::new(0u32);
        let result = retry_backend_window_id_lookup(5, Duration::from_millis(1), || {
            *calls.borrow_mut() += 1;
            None::<String>
        }, no_sleep);
        assert_eq!(result, None);
        assert_eq!(*calls.borrow(), 5);
    }

    #[test]
    fn resolves_immediately_without_needless_extra_attempts() {
        let calls = RefCell::new(0u32);
        let result = retry_backend_window_id_lookup(5, Duration::from_millis(1), || {
            *calls.borrow_mut() += 1;
            Some("wid-fast".to_string())
        }, no_sleep);
        assert_eq!(result, Some("wid-fast".to_string()));
        assert_eq!(*calls.borrow(), 1);
    }
}
