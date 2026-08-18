// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane-specific CEF callback bodies.
//!
//! Extracted from `client.rs` in Phase 4 of the modularization split
//! (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). `AgentMuxHandler`
//! still owns the CEF callback plumbing; this module holds the pane-branch
//! bodies so pane-specific logic lives in one place instead of threaded
//! through `if self.is_browser_pane` branches in `client.rs`.
//!
//! Notable: this is where `install_browser_pane_focus_redirect` actually gets wired
//! in. Before this phase the function existed in `browser_pane::hwnd` but had zero
//! callers (see `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #5). Now
//! `on_after_created_browser_pane` and `on_load_end_browser_pane` both reinstall the focus
//! subclass — required because Chromium recreates the
//! `Chrome_RenderWidgetHostHWND` child on every navigation, stranding the
//! old subclass on a destroyed HWND.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use cef::*;

use crate::state::AppState;

/// How long a browser pane's top-level navigation may sit in CEF's real
/// "still loading" state before we give up on it ourselves and show a
/// synthetic `ERR_CONNECTION_TIMED_OUT` page. Chromium's actual TCP
/// connect-timeout ceiling (`net::TransportConnectJob::ConnectionTimeout()`)
/// is 4 minutes — the SAME code Chrome itself runs, so there is no CEF
/// setting that makes "our" timeout longer than Chrome's; they share it.
/// This is a deliberate product choice to bound a floating pane's UX well
/// below that ceiling instead of leaving it blank for up to 4 minutes on a
/// silently-dropped connection.
const PANE_LOAD_WATCHDOG_TIMEOUT_MS: i64 = 20_000;

/// Monotonic arm counter for the pane load watchdog — mirrors
/// `client::navigation`'s `PAINT_GATE_NEXT_EPOCH` pattern. Each arm
/// (navigation start) gets a fresh epoch; the delayed watchdog task captures
/// it and only fires if it's still current when the deadline elapses, so a
/// navigation that finishes — or a NEW navigation that starts — before the
/// deadline can't have a stale watchdog fire over unrelated content.
static PANE_LOAD_WATCHDOG_NEXT_EPOCH: AtomicU64 = AtomicU64::new(0);

wrap_task! {
    struct PaneLoadWatchdogTask {
        state: Arc<AppState>,
        block_id: String,
        epoch: u64,
    }
    impl Task { fn execute(&self) {
        fire_pane_load_watchdog(&self.state, &self.block_id, self.epoch);
    }}
}

/// Arm a FRESH pane load watchdog — new epoch, new `PANE_LOAD_WATCHDOG_TIMEOUT_MS`
/// deadline — for a main-frame browser-pane navigation that's about to
/// start. Called from `client::lifecycle::on_before_browse` for a genuinely
/// new navigation (`is_redirect == 0`), NOT from the loading-state-change
/// handler — `on_before_browse` hands us the navigation's own target URL via
/// CEF's `Request` object, which is the only reliable source for "what is
/// this navigation actually trying to reach." (`Frame::url()` reflects the
/// last COMMITTED document; for a navigation that's still pending — and a
/// watchdog exists purely to handle the case where a navigation stays
/// pending forever — that's still the PREVIOUS page.) `url` is stored
/// alongside the arm so `fire_pane_load_watchdog` never has to re-derive it.
///
/// A redirect hop of an ALREADY-armed navigation must NOT call this — see
/// `update_pane_load_watchdog_url` for why.
pub(crate) fn arm_pane_load_watchdog(state: &Arc<AppState>, block_id: &str, browser: Browser, url: String) {
    let epoch = PANE_LOAD_WATCHDOG_NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    state
        .browser_pane_load_watchdog
        .lock()
        .insert(block_id.to_string(), (Instant::now(), epoch, browser, url));
    let mut task = PaneLoadWatchdogTask::new(state.clone(), block_id.to_string(), epoch);
    post_delayed_task(ThreadId::UI, Some(&mut task), PANE_LOAD_WATCHDOG_TIMEOUT_MS);
}

/// Update the STORED target URL for an already-armed watchdog, without
/// touching its epoch or deadline. Called from `on_before_browse` for
/// REDIRECT hops (`is_redirect != 0`) of a navigation that's already armed.
///
/// Calling `arm_pane_load_watchdog` instead (an earlier version of this fix
/// did) hands out a fresh `PANE_LOAD_WATCHDOG_TIMEOUT_MS` deadline on EVERY
/// hop of a redirect chain, so a site that redirects a handful of times
/// before its final hop hangs could push the actual wait arbitrarily far
/// past the intended 20s bound before the watchdog ever fires — defeating
/// its whole purpose (reagentx P1 on PR #2593). The error page should still
/// report the LATEST hop being attempted if the watchdog does eventually
/// fire, though, so the URL itself is still updated — just not the timer.
///
/// No-ops if nothing is currently armed for `block_id` — e.g. a redirect
/// notification arriving after the watchdog already fired/disarmed. Whichever
/// of those already ran owns the outcome; there's nothing to update.
pub(crate) fn update_pane_load_watchdog_url(state: &Arc<AppState>, block_id: &str, url: String) {
    if let Some(entry) = state.browser_pane_load_watchdog.lock().get_mut(block_id) {
        entry.3 = url;
    }
}

/// Runs on the CEF UI thread when a pane's load-watchdog deadline elapses.
/// No-ops if the navigation already finished (disarmed by
/// `on_loading_state_change_browser_pane`) or a newer navigation re-armed
/// the same pane (epoch mismatch) — both mean this specific timeout is
/// stale and must not act.
fn fire_pane_load_watchdog(state: &Arc<AppState>, block_id: &str, epoch: u64) {
    let entry = state.browser_pane_load_watchdog.lock().remove(block_id);
    let Some((armed_at, armed_epoch, mut browser, target_url)) = entry else { return };
    if armed_epoch != epoch {
        // A newer navigation re-armed this pane after this timeout was
        // scheduled — put the newer entry back (we shouldn't have taken it)
        // and let its OWN watchdog, or normal load completion, own the
        // outcome instead of forcing a timeout over content that isn't even
        // the navigation this timeout was scheduled for.
        state
            .browser_pane_load_watchdog
            .lock()
            .insert(block_id.to_string(), (armed_at, armed_epoch, browser, target_url));
        return;
    }
    let Some(mut frame) = browser.main_frame() else { return };
    tracing::warn!(
        "[pane-load-watchdog] navigation still loading after {}ms, forcing timeout: block_id={} url={:?}",
        PANE_LOAD_WATCHDOG_TIMEOUT_MS,
        block_id,
        target_url,
    );
    crate::browser_pane::trace::pane_trace(
        block_id,
        "load-watchdog-timeout",
        &format!("url={target_url} after {}ms", PANE_LOAD_WATCHDOG_TIMEOUT_MS),
    );
    crate::client::navigation::show_load_error_page(
        &mut frame,
        &target_url,
        sys::cef_errorcode_t::ERR_CONNECTION_TIMED_OUT as i32,
        "ERR_CONNECTION_TIMED_OUT",
        true,
    );
}

/// Set (or clear) the main-frame-loading tracker for `block_id` and, if the
/// tracked state actually changed, emit a `browser-pane-nav-state` event
/// carrying the corrected `is_loading` for the frontend spinner. `url`
/// is supplied by the caller rather than re-derived here because the
/// correct source differs by call site: `on_before_browse` must pass the
/// navigation's own REQUEST target (the frame hasn't committed yet, so
/// `Frame::url()` there would still report the previous page — same
/// reasoning as `arm_pane_load_watchdog`'s doc comment), while
/// `on_load_end_browser_pane`/`on_load_error` pass the frame's now-current
/// (committed or failed) URL.
///
/// No epoch/generation guard is needed here (unlike
/// `browser_pane_load_watchdog`'s epoch): a `false` call only ever
/// originates from `on_load_end_browser_pane` or a main-frame `on_load_error`
/// that already survived the `ERR_ABORTED` filter — i.e. a navigation that
/// was genuinely superseded by a newer one is reported as `ERR_ABORTED` and
/// filtered out before reaching this function, so a real (non-aborted)
/// completion/error for `block_id` can't race a newer navigation that's
/// already `true` in the tracker.
///
/// See `docs/specs/SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md`
/// layer 1.
pub(crate) fn set_pane_main_frame_loading(state: &Arc<AppState>, block_id: &str, url: &str, loading: bool) {
    let changed = {
        let mut set = state.browser_pane_main_frame_loading.lock();
        if loading {
            set.insert(block_id.to_string())
        } else {
            set.remove(block_id)
        }
    };
    if !changed {
        return; // already in the target state — no redundant emit
    }
    tracing::info!(
        "[browser-pane:diag][{}] main-frame-loading={} url={:?}",
        block_id.chars().take(7).collect::<String>(),
        loading,
        url,
    );
    crate::events::emit_event_from_state(
        state,
        "browser-pane-nav-state",
        &serde_json::json!({
            "block_id": block_id,
            "url": url,
            "is_loading": loading,
        }),
    );
}

/// Current value of the main-frame-loading tracker for `block_id`. Read by
/// `on_loading_state_change_browser_pane` so its own emitted `is_loading`
/// field (that event also legitimately carries `can_go_back`/
/// `can_go_forward`, so it can't simply stop emitting `is_loading`
/// altogether) reflects this corrected, main-frame-scoped signal instead of
/// forwarding CEF's raw frame-blind parameter a second time.
pub(crate) fn is_pane_main_frame_loading(state: &Arc<AppState>, block_id: &str) -> bool {
    state.browser_pane_main_frame_loading.lock().contains(block_id)
}

/// Called from `AgentMuxHandler::on_after_created` when the browser being
/// registered is a pane (label prefix `browser-pane-*`).
///
/// Responsibilities:
/// 1. Raise the pane's outer HWND to the top of its parent's Z-order so
///    mouse-wheel events reach the pane renderer rather than main's.
/// 2. Install the WM_SETFOCUS redirect subclass on the pane's HWND tree so
///    Chromium's internal focus-steals on page load don't yank keyboard
///    focus away from the main window.
pub fn on_after_created_browser_pane(state: &Arc<AppState>, browser: &Browser) {
    #[cfg(target_os = "windows")]
    {
        if let Some(host) = browser.host() {
            let wh = host.window_handle();
            if !wh.0.is_null() {
                let hwnd = wh.0 as *mut std::ffi::c_void;

                // Z-order: bring pane above main's widget.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                        hwnd as _,
                        std::ptr::null_mut(), // HWND_TOP
                        0, 0, 0, 0,
                        0x0001 | 0x0002 | 0x0010, // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
                    );
                }
                tracing::info!("[pane-zorder] raised pane to top of Z-order");

                // Subclass the pane HWND + its descendants so WM_SETFOCUS
                // from Chromium gets redirected to the parent. The state
                // and block_id let the subclass emit `browser-pane-clicked`
                // directly on WM_LBUTTONDOWN without relying on CEF focus
                // callbacks (which don't fire for clicks inside an already-
                // focused pane).
                let block_id = resolve_pane_block_id(state, browser).unwrap_or_default();
                unsafe {
                    crate::browser_pane::hwnd::install_browser_pane_focus_redirect(
                        hwnd,
                        state.clone(),
                        block_id,
                    );
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // On macOS/Linux, the focus redirect subclass used on Windows doesn't
        // exist. CEF's on_after_created fires from inside add_overlay_view
        // (when the BrowserView is added to the widget hierarchy) and gives
        // focus to the new pane browser. creation_views::create_browser_pane_view
        // immediately returns focus to the main window after add_overlay_view
        // returns — this callback is an additional safety net for any navigation-
        // driven focus steals that bypass that initial return (e.g. cross-origin
        // redirects that recreate the renderer, hitting on_after_created again).
        //
        // Only refocus if we can identify the parent window; don't do a blanket
        // "focus main" that would disrupt multi-window setups.
        if let Some(block_id) = resolve_pane_block_id(state, browser) {
            let parent_window_label = state
                .browser_pane_overlays
                .lock()
                .iter()
                .find(|(lbl, _)| {
                    // label format: browser-pane-<block_id>-<seq>
                    lbl.starts_with(&format!("browser-pane-{}-", block_id))
                })
                .map(|(_, (win_lbl, _))| win_lbl.clone());

            if let Some(win_label) = parent_window_label {
                if let Some(main_browser) = state.get_browser(&win_label) {
                    if let Some(mut host) = main_browser.host() {
                        host.set_focus(1);
                        tracing::info!(
                            block_id = %block_id, window_label = %win_label,
                            "[browser-pane] macOS/Linux on_after_created: returned focus to main window"
                        );
                    }
                }
            }
        }
    }
}

/// Called from `AgentMuxHandler::on_before_close` after the browser has
/// been removed from `state.browsers` and the label has been identified
/// as a pane label (prefix `browser-pane-*`).
///
/// On Linux/macOS, runs the deferred `OverlayController::destroy()` for
/// any controller that `detach_browser_pane_view` stashed — see the long
/// comment on `state.pending_overlay_destroy` for why destroy can't run
/// synchronously with the close request. Drains the reducer entry next so
/// a re-create with the same block_id gets a fresh Live state. Idempotent
/// — if the explicit `close()` path already drained the reducer, the
/// drain is a no-op; if no controller was stashed (Windows or already
/// destroyed), the destroy step is a no-op.
pub fn on_before_close_browser_pane(state: &Arc<AppState>, label: &str) {
    // Step 1 (Linux/macOS): destroy the deferred OverlayController.
    // Safe now because the Browser is fully torn down and Chromium has
    // drained any queued tasks holding `WeakPtr<View>` to its BrowserView.
    #[cfg(not(target_os = "windows"))]
    {
        let stashed = state.pending_overlay_destroy.lock().remove(label);
        if let Some(controller) = stashed {
            controller.destroy();
            tracing::info!(
                label = %label,
                "[browser-pane] views: deferred OverlayController destroyed at on_before_close"
            );
        }
    }

    // Step 2: drain the reducer's pane entry (idempotent).
    state.browser_panes.drain_closed_label(state, label);

    // Labels are `browser-pane-<uuid>-<seq>`; strip prefix + trailing `-<seq>`
    // to recover the block_id.
    let block_id = label
        .strip_prefix("browser-pane-")
        .and_then(|rest| rest.rfind('-').map(|dash| &rest[..dash]));

    // Cross-platform: drop this pane's zoom-factor entry so
    // `browser_pane_zoom` doesn't grow unboundedly as panes are opened and
    // closed over a session. Same for the stashed context-menu frame
    // (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md) — also holds a
    // cloned `Frame` that must not outlive the closing browser.
    if let Some(block_id) = block_id {
        state.browser_pane_zoom.lock().remove(block_id);
        state.browser_pane_context_menu_frame.lock().remove(block_id);
        // Drop the main-frame-loading tracker entry too — otherwise a pane
        // closed mid-navigation leaves a stale `true` behind, and a
        // "redock" recreate reusing the same block_id (see
        // `browser_panes/mod.rs`'s redock comment) would inherit it and
        // start with a spinner that never got a matching false transition.
        state.browser_pane_main_frame_loading.lock().remove(block_id);

        // If this pane closes mid-navigation, its load watchdog is still
        // armed holding a cloned `Browser`. Without this removal,
        // `fire_pane_load_watchdog` fires up to PANE_LOAD_WATCHDOG_TIMEOUT_MS
        // later and calls `main_frame()`/`load_url()` on a browser that's
        // already torn down (reagentx P1 on PR #2593).
        if state.browser_pane_load_watchdog.lock().remove(block_id).is_some() {
            tracing::debug!(
                block_id = %block_id,
                "[pane-load-watchdog] pane closed mid-navigation, watchdog disarmed"
            );
        }
    }

    // Windows only:
    //   1. Restore WndProcs for all subclassed HWNDs (must run first, before
    //      remove_contexts_for_block wipes the outer-HWND lookup).
    //   2. Wipe BROWSER_PANE_HWND_CONTEXT entries for the block.
    #[cfg(target_os = "windows")]
    {
        if let Some(block_id) = block_id {
            crate::browser_pane::hwnd::uninstall_focus_redirect_for_block(block_id);
            crate::browser_pane::hwnd::remove_contexts_for_block(block_id);
        }
    }
}

/// Called from `AgentMuxHandler::on_load_start` for a browser pane's MAIN
/// frame only (caller already filtered on `frame.is_main()`), i.e. the
/// moment a navigation actually COMMITS and starts loading its content —
/// disarms the pane-load-watchdog right here, rather than waiting for
/// `on_loading_state_change_browser_pane`'s `!is_loading` (which waits for
/// EVERY subresource too). Once the target document has committed, the
/// watchdog's whole purpose — catching a navigation that never resolves,
/// leaving the pane blank — no longer applies: the user is now looking at
/// real, committed content, even if a slow image/script/iframe keeps CEF's
/// `is_loading` true past the 20s deadline. Firing the watchdog after this
/// point would replace an already-successfully-loaded page with a synthetic
/// `ERR_CONNECTION_TIMED_OUT` (reagentx P1 on PR #2593, second pass; also
/// flagged inline by Codex on the same PR).
///
/// No-ops if nothing is armed for this block_id — e.g. a client-side (SPA)
/// route change, which doesn't re-arm the watchdog in the first place
/// (`arm_pane_load_watchdog` is only called from `on_before_browse` for a
/// genuine top-level navigation), or a commit arriving after the watchdog
/// already fired/disarmed via some other path.
pub fn on_load_start_browser_pane(state: &Arc<AppState>, browser: &Browser) {
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        if state.browser_pane_load_watchdog.lock().remove(&block_id).is_some() {
            crate::browser_pane::trace::pane_trace(
                &block_id,
                "load-watchdog-disarmed",
                "main frame committed — target document has content, watchdog no longer needed",
            );
        }
    }
}

/// Called from `AgentMuxHandler::on_load_end` when `is_browser_pane` is true.
///
/// Chromium creates a fresh `Chrome_RenderWidgetHostHWND` on every
/// navigation. The subclass installed at `on_after_created` is on the
/// OLD widget HWND, which was destroyed during navigation — so without
/// reinstalling here, keyboard focus steals by the new page bypass our
/// redirect and end up stuck on the pane.
///
/// Does NOT force focus back to main. `WM_MOUSEWHEEL` is routed to the
/// focused HWND; stealing focus away from the pane breaks scrolling.
/// The FocusHandler cancel + WndProc redirect already keep focus off
/// the pane during the *initial* navigation focus steal.
pub fn on_load_end_browser_pane(state: &Arc<AppState>, browser: &Browser) {
    tracing::info!("[pane-load-end] pane page loaded; reinstalling focus subclass");
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        crate::browser_pane::trace::pane_trace(&block_id, "load-end", &format!("url={url}"));

        // Main-frame load actually finished — clear the loading-spinner
        // tracker (layer 1, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md).
        // `on_load_end_browser_pane` is only ever called for the main frame
        // (filtered at the `client::navigation::on_load_end` call site).
        set_pane_main_frame_loading(state, &block_id, &url, false);

        // Every navigation replaces the page's own DOM/inline-style state,
        // so any CSS `zoom` injected before this load is gone with it --
        // re-apply this pane's stored factor (no-op if it's never been
        // zoomed away from the 1.0 default). See BrowserPaneManager::
        // reapply_zoom's own doc comment for why this is CSS injection and
        // not Chromium's native per-host zoom.
        state.browser_panes.reapply_zoom(&block_id, state);
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(host) = browser.host() {
            let wh = host.window_handle();
            if !wh.0.is_null() {
                let block_id = resolve_pane_block_id(state, browser).unwrap_or_default();
                unsafe {
                    crate::browser_pane::hwnd::install_browser_pane_focus_redirect(
                        wh.0 as *mut std::ffi::c_void,
                        state.clone(),
                        block_id,
                    );
                }
            }
        }
    }

    // URL-only event emit at load_end so the address bar catches redirects
    // that resolve during frame load (e.g. google.com → www.google.com).
    // `can_go_back` / `can_go_forward` are intentionally not read here —
    // `on_load_end` fires before the navigation controller commits the
    // history entry, so calling `browser.can_go_back()` from this hook
    // can return the pre-navigation state. Those flags flow through the
    // dedicated `on_loading_state_change_browser_pane` callback below, which CEF
    // provides with correct values as direct parameters.
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-nav-state url={:?} url_only=true",
            block_id_short, url,
        );
        crate::events::emit_event_from_state(
            state,
            "browser-pane-nav-state",
            &serde_json::json!({
                "block_id": block_id,
                "url": url,
                // can_* omitted on purpose — frontend treats missing
                // fields as "no change" and keeps the last values from
                // on_loading_state_change.
                "url_only": true,
            }),
        );
    } else {
        tracing::warn!("[pane-load-end] couldn't resolve block_id for nav-state emit");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = browser;
    }
}

/// Pane-specific `on_loading_state_change` body. Called from
/// `AgentMuxHandler::on_loading_state_change` when `is_browser_pane == true`.
///
/// CEF invokes `on_loading_state_change` whenever the navigation controller's
/// history state changes — navigation start, navigation commit, and after
/// back/forward. `can_go_back` / `can_go_forward` are provided as direct
/// parameters (not queried after the fact), so they're guaranteed to reflect
/// the real committed state rather than the pre-commit race window — those
/// are legitimately browser-level (not frame-specific) and still forwarded
/// verbatim.
///
/// `is_loading`, by contrast, is CEF's aggregate loading state across the
/// WHOLE frame tree (this callback carries no frame parameter at all — it
/// structurally can't distinguish which frame's load it's reporting), so a
/// sub-frame/subresource load (an ad iframe, a chat widget, an analytics
/// beacon) can flip it long after the main document is done — the loading
/// spinner used to trust it directly, which produced repeated
/// show/hide/show flicker (see
/// `docs/specs/SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md`
/// cause 1; supersedes SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md
/// §4.1's "forwarded verbatim" design, which never investigated sub-frame
/// aggregation). The EMITTED `is_loading` field now instead reads
/// `is_pane_main_frame_loading` — the dedicated, main-frame-only tracker
/// maintained by `set_pane_main_frame_loading` from `on_before_browse` /
/// `on_load_end_browser_pane` / a main-frame `on_load_error`. The raw
/// parameter is still used for the watchdog disarm below, unchanged — that
/// behavior isn't part of this fix.
pub fn on_loading_state_change_browser_pane(
    state: &Arc<AppState>,
    browser: &Browser,
    is_loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
) {
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        if !is_loading {
            // Navigation settled — either it committed normally or CEF's own
            // `on_load_error` already showed a real error page. Either way
            // the watchdog no longer applies to it. (Arming happens earlier,
            // in `client::lifecycle::on_before_browse` — see
            // `arm_pane_load_watchdog`'s doc comment for why it can't happen
            // here.) Deliberately keyed on the RAW CEF `is_loading` param,
            // not `is_pane_main_frame_loading` — the watchdog's own
            // arm/disarm behavior is unchanged by this fix.
            state.browser_pane_load_watchdog.lock().remove(&block_id);
        }

        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        let corrected_is_loading = is_pane_main_frame_loading(state, &block_id);
        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-nav-state url={:?} url_only=false is_loading={} (raw_cef_is_loading={}) can_back={} can_forward={}",
            block_id_short, url, corrected_is_loading, is_loading, can_go_back, can_go_forward,
        );
        crate::events::emit_event_from_state(
            state,
            "browser-pane-nav-state",
            &serde_json::json!({
                "block_id": block_id,
                "url": url,
                "can_go_back": can_go_back,
                "can_go_forward": can_go_forward,
                "is_loading": corrected_is_loading,
            }),
        );
    } else {
        tracing::warn!("[pane-loading-state] couldn't resolve block_id for nav-state emit");
    }
}

/// Resolve the `block_id` for a pane browser. Panes are registered in
/// `state.browsers` under labels like `browser-pane-<uuid>-<seq>`. Find the
/// label whose browser handle matches the given one by `is_same`, then
/// strip the prefix and the trailing `-<seq>` to recover the uuid.
pub(crate) fn resolve_pane_block_id(state: &Arc<AppState>, browser: &Browser) -> Option<String> {
    // Phase H.2.b — reducer-aware iteration with fallback.
    state
        .list_browsers()
        .into_iter()
        .find(|(_k, b)| {
            let mut b_clone = b.clone();
            let mut browser_clone: cef::Browser = browser.clone();
            b_clone.is_same(Some(&mut browser_clone)) != 0
        })
        .and_then(|(label, _)| {
            let rest = label.strip_prefix("browser-pane-")?;
            let dash = rest.rfind('-')?;
            Some(rest[..dash].to_string())
        })
}
