// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tear-off Phase 6 — pre-warmed window pool.
//
// Spec §0 makes a 0 ms first-paint flash mandatory. The cold path
// (open_window_at_position → wait for HWND register → SC_MOVE)
// has an inherent 150-300 ms gap while CEF spawns the renderer
// process and paints first frame. Pre-spawning hidden windows
// eliminates that gap: on tear-off we POP an already-painted
// window from the pool, reposition it under the cursor, show it,
// and emit `pool:promote` so the renderer bootstraps the
// workspace in-place (no reload, no renderer restart).
//
// Pool windows live with URL `?pool=1`; the frontend init flow
// detects this and defers `initHostNewWindow()` until the
// `pool:promote` event arrives with a workspace ID.
//
// Sizing: N=2. One window is "next destination," the other is
// "buffer while respawn completes." With N=1 a back-to-back
// tear-off would cold-path. With N>2 the RAM cost outweighs the
// rare-second-tearoff benefit.
//
// Lifecycle:
// - App startup → spawn N pool windows after primary first-paint.
// - On tear-off → pop, reposition, show, emit promote, enqueue
//   refill. Refills are serialised (single in-flight) so a burst
//   of tear-offs can't spawn unbounded windows.
// - App shutdown → pool windows close cleanly with the rest of
//   the process.

use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashMap;

use crate::state::{AppState, WindowKind, WindowMeta};

/// HWND cache for pool windows. Populated at `on_after_created`
/// (register_pool_window) and consulted at `promote_pool_window` as
/// the source of truth — `BrowserHost::window_handle()` returns null
/// once the page loads even though the underlying Win32 window is
/// alive (verified by `IsWindow` — see
/// `SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md` §4.1 diagnostic run).
///
/// Entries are removed on pool-window destruction
/// (`on_pool_window_destroyed`) so the map can't leak across the
/// process lifetime. The HWND is stored as `usize` so this state is
/// `Send + Sync` without `unsafe`; callers cast back to `HWND` /
/// `*mut c_void` at use site.
#[cfg(target_os = "windows")]
static POOL_HWND_CACHE: std::sync::OnceLock<Mutex<HashMap<String, usize>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
fn pool_hwnd_cache() -> &'static Mutex<HashMap<String, usize>> {
    POOL_HWND_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Global cache of pool windows' CEF Views `Window`, captured at
/// `on_window_created` (UI thread) and consumed by a promote UI task. The
/// macOS/Linux `state.windows` registry is cfg-gated OFF on Windows, and
/// `browser_view.window()` returns None for pool windows post-load
/// (SPEC_POOL_WINDOW_HWND_NULL), so the promote can't otherwise reach the Views
/// `Window` to apply the macOS-parity `set_bounds()`+`show()` fix.
///
/// This is a Send `Mutex` (NOT a thread-local): `on_window_created` runs on the
/// CEF UI thread but the Windows promote runs on the IPC thread, so the cache
/// must cross threads. `cef::Window` is `Send` (same as the macOS `state.windows`
/// registry). The actual `set_bounds()`+`show()` is still performed on the UI
/// thread via a posted task — CEF Views calls are UI-thread-only.
#[cfg(target_os = "windows")]
static POOL_WINDOW_VIEWS: std::sync::OnceLock<Mutex<HashMap<String, cef::Window>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
fn pool_window_views() -> &'static Mutex<HashMap<String, cef::Window>> {
    POOL_WINDOW_VIEWS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cache a pool window's CEF Views `Window` (called from `on_window_created`,
/// where the Window is valid). No-op for non-pool labels.
#[cfg(target_os = "windows")]
pub fn cache_pool_window_view(label: &str, window: &cef::Window) {
    if !label.starts_with("window-pool-") {
        return;
    }
    pool_window_views()
        .lock()
        .unwrap()
        .insert(label.to_string(), window.clone());
}

/// Take (remove + return) a pool window's cached CEF Views `Window`. Called on
/// the UI thread by the promote show task.
#[cfg(target_os = "windows")]
pub fn take_pool_window_view(label: &str) -> Option<cef::Window> {
    pool_window_views().lock().unwrap().remove(label)
}

/// HWND cache for pane pool windows. Separate from `POOL_HWND_CACHE` because
/// pane pool windows are WS_POPUP + WS_EX_TOOLWINDOW (unowned, no taskbar entry)
/// and their outer HWND must be cached before the CEF child browser is created
/// (same reason as `POOL_HWND_CACHE` — `window_handle()` goes null after load).
#[cfg(target_os = "windows")]
static PANE_POOL_HWND_CACHE: std::sync::OnceLock<Mutex<HashMap<String, usize>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
fn pane_pool_hwnd_cache() -> &'static Mutex<HashMap<String, usize>> {
    PANE_POOL_HWND_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Store the outer WS_POPUP HWND for a pane pool window. Called from
/// `floating_pane::CreatePanePoolWindowWin32Task` after `CreateWindowExW`
/// and before `browser_host_create_browser` fires.
#[cfg(target_os = "windows")]
pub(crate) fn cache_pane_pool_hwnd(label: &str, hwnd: usize) {
    pane_pool_hwnd_cache()
        .lock()
        .unwrap()
        .insert(label.to_string(), hwnd);
}

/// Pop (and remove) the outer HWND for a pane pool label (promote or destroy path).
#[cfg(target_os = "windows")]
pub(crate) fn take_pane_pool_hwnd(label: &str) -> Option<usize> {
    pane_pool_hwnd_cache().lock().unwrap().remove(label)
}

/// Clean up after a pane pool window that failed during creation (before it
/// ever became a usable pool slot). Unlike `on_pane_pool_window_destroyed`,
/// this does NOT trigger a refill spawn — creation failures are likely
/// persistent (Win32 error, driver issue) and re-spawning immediately would
/// cause an infinite respawn loop: create → fail → cleanup → spawn → fail.
/// The pool stays empty and the cold path handles the next tear-off request.
#[cfg(target_os = "windows")]
pub(crate) fn cleanup_failed_pane_pool_creation(state: &Arc<AppState>, label: &str) {
    take_pane_pool_hwnd(label);
    state.host_dispatch(
        crate::reducer::HostCommand::PanePoolWindowDestroyedBeforePromote {
            label: label.to_string(),
        },
    );
    // Deliberately skip spawn_pane_pool_window — creation failure may be
    // persistent; caller should dequeue PendingWindowCreation separately.
}

/// Target pool size. See module-level comment for rationale.
pub const POOL_TARGET_SIZE: usize = 2;

/// Pool windows are spawned at this off-screen position so they
/// don't appear on the user's desktop while pre-painting. On
/// promote they're moved to the cursor and shown.
pub(crate) const POOL_OFFSCREEN_X: i32 = -32000;
pub(crate) const POOL_OFFSCREEN_Y: i32 = -32000;
const POOL_WIDTH: i32 = 1200;
const POOL_HEIGHT: i32 = 800;
/// Pixels above the cursor where the title bar sits — matches
/// open_window_at_position so the cursor lands near the top-center
/// of the title bar after promotion.
const TITLE_BAR_OFFSET_PX: i32 = 16;

/// Clamp a window rect so it lies fully within a monitor work area.
///
/// Shrinks `(w, h)` to fit the work area if larger, then shifts the origin so no
/// edge falls outside. The result is guaranteed on-screen — the safety net that
/// prevents a HiDPI coordinate miscalc from stranding a pool-promoted new window
/// off-screen (the 2026-06-21 "blank new window" bug: a window parked at the
/// DPI-scaled `POOL_OFFSCREEN`, e.g. `-25600 = -32000 × 0.8` at 125%). Pure — the
/// work-area rect is passed in — so the placement logic is unit-tested without
/// Win32. See docs/specs/PLAN_POOL_NEW_WINDOW_DPI_POSITIONING_2026_06_21.md.
pub(crate) fn clamp_rect_within(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    wa_x: i32,
    wa_y: i32,
    wa_w: i32,
    wa_h: i32,
) -> (i32, i32, i32, i32) {
    let w = w.clamp(1, wa_w.max(1));
    let h = h.clamp(1, wa_h.max(1));
    // Pull the right/bottom edge inside first, then the left/top edge — order
    // matters so an oversized-or-far rect ends up flush against the work area
    // origin rather than hanging off the far edge.
    let x = x.min(wa_x + wa_w - w).max(wa_x);
    let y = y.min(wa_y + wa_h - h).max(wa_y);
    (x, y, w, h)
}

#[cfg(test)]
mod clamp_tests {
    use super::clamp_rect_within;

    #[test]
    fn brings_offscreen_rect_onscreen() {
        // The Window 3 case: scaled POOL_OFFSCREEN with a broken size.
        let (x, y, w, h) = clamp_rect_within(-25600, -25600, 159, 27, 0, 0, 1920, 1040);
        assert!(x >= 0 && y >= 0 && x + w <= 1920 && y + h <= 1040, "got {:?}", (x, y, w, h));
    }

    #[test]
    fn shrinks_oversized_and_keeps_inside() {
        let (x, y, w, h) = clamp_rect_within(1800, 1000, 1200, 800, 0, 0, 1920, 1040);
        assert!(w <= 1920 && h <= 1040 && x >= 0 && y >= 0 && x + w <= 1920 && y + h <= 1040);
    }

    #[test]
    fn leaves_valid_rect_unchanged() {
        assert_eq!(clamp_rect_within(421, 246, 960, 640, 0, 0, 1920, 1040), (421, 246, 960, 640));
    }

    #[test]
    fn respects_negative_monitor_origin() {
        // Secondary monitor to the left of the primary (negative virtual coords).
        let (x, _y, w, _h) = clamp_rect_within(-1900, 100, 960, 640, -1920, 0, 1920, 1080);
        assert!(x >= -1920 && x + w <= 0, "rect must stay on the left monitor: {:?}", (x, w));
    }
}

// Tab-anchor placement: PR #730 hardcoded FIRST_TAB_INSET_X /
// TAB_STRIP_TOP_OFFSET_PX as best-effort window-chrome offsets.
// Smoke on v0.33.704 showed they were inaccurate (new window's
// first tab not landing where the dragged tab was). The frontend
// now measures the source window's chrome dynamically and computes
// the new window's outer top-left position itself; backend just
// uses the supplied anchor verbatim. The constants are gone.

/// Spawn a single pool window. Called at startup (N times) and
/// after each promote (1 refill). Idempotent against the
/// in-flight semaphore — concurrent calls collapse to one spawn
/// in flight at a time.
pub fn spawn_pool_window(state: &Arc<AppState>) {
    // Phase B.9.3 — if the host has decided to quit (last user-
    // visible window closed, draining pool), skip refill. Without
    // this guard, every pool close triggers a refill, keeping
    // state.browsers non-empty forever and quit_message_loop's
    // QuitWhenIdle never reaches idle.
    // PR #5 H.5 — early-out before dispatching to the reducer. The
    // reducer's PoolWindowSpawnStart arm ALSO checks `quit_state !=
    // Running` and would no-op, but the early-out keeps the warn-log
    // shape identical to pre-PR for diagnostic continuity.
    if state.is_quitting() {
        tracing::warn!(
            target: "wrr",
            "[wrr] spawn_pool_window skipped — quit_state != Running (drain mode)"
        );
        return;
    }

    // PR #6 H.7 — refuse pool refill while any pane is mid-close. Pool
    // windows are CEF top-levels just like user-visible ones; the v146
    // deadlock fires regardless of whether the new window is on-screen.
    // See `commands/window.rs::open_window_with_kind` for rationale.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] spawn_pool_window deferred — pane is mid-close (H.7 invariant)"
        );
        return;
    }

    // PR #6 codex P1 — capacity check. The semaphore (PoolWindowSpawnStart)
    // only single-flights; it does not enforce the target size. Without
    // this guard, the new H.7 always-on-pane-close kick (in
    // `BrowserPaneManager::close` / `drain_closed_label`) would add a
    // pool window on every pane close once no spawn is in flight, growing
    // `pool.queue` indefinitely past `POOL_TARGET_SIZE`. The legacy
    // callers (`mark_pool_window_renderer_ready`,
    // `on_pool_window_destroyed`) already gate on the same check; this
    // moves it inside spawn_pool_window so every entry point is covered.
    if state.pool_queue_size() >= POOL_TARGET_SIZE {
        tracing::debug!(
            target: "dnd:tearoff:pool",
            current = %state.pool_queue_size(),
            target = %POOL_TARGET_SIZE,
            "[pool] spawn skipped — pool already at target size"
        );
        return;
    }

    let window_id = uuid::Uuid::new_v4();
    // Use the `window-pool-` prefix so existing `is_instance_label`
    // checks (tear_off_hook.rs, app-init.ts) pass naturally — they
    // accept anything starting with `window-`. After promotion the
    // label stays the same; the reducer's `pool.unpromoted` is the
    // authoritative pool-vs-promoted distinction (cleared on
    // promote — `pool.queue` is populated only after the
    // renderer-ready handshake, so it's not reliable as the
    // distinguisher during the ~100ms spawn → ready gap).
    let label = format!("window-pool-{}", window_id.simple());

    // PR #5 H.4 — atomic single-flight + label-into-unpromoted via the
    // reducer. `pool_spawn_proceeding=false` means another spawn was
    // already in flight (or quit_state != Running) and we should skip
    // — the in-flight spawn will catch up to TARGET_SIZE.
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PoolWindowSpawnStart { label: label.clone() },
    );
    if !dispatch.pool_spawn_proceeding {
        tracing::debug!(
            target: "dnd:tearoff:pool",
            "[pool] spawn skipped — respawn already in flight or pool draining"
        );
        return;
    }

    // Phase B.4 follow-up — mirror the pool inventory in the launcher
    // and check pool drift. We use the pool-only variant
    // (`report_host_pool_count`) rather than the full
    // `report_host_counts` because `spawn_pool_window` is invoked
    // from the refill chain inside `on_pool_window_destroyed`, which
    // runs during `on_before_close` BEFORE the matching
    // `ReportWindowClosed` is sent for the closing window. A
    // full-counts snapshot at this moment would see browsers
    // shrunk (closing window already removed) but the launcher
    // mirror still holding it (close not yet reported), producing
    // transient false windows-drift on every normal
    // promoted-window close that triggers a refill. Pool count IS
    // stable at this moment (the new label was just added), so
    // checking pool alone preserves the "check every transition"
    // guarantee for the dimension that actually changed. (codex
    // P2 PR #578 rounds 2 + 3.)
    crate::launcher_ipc::report_pool_window_added(label.clone());
    {
        // Pool inventory (unpromoted ∪ queue), not unpromoted-only:
        // the launcher's `state.pool` mirror is built from
        // ReportPoolWindowAdded/Removed/Promoted events; the host's
        // unpromoted→queue transition emits NO event, so the
        // launcher retains queued labels in its pool set. Reporting
        // unpromoted.len() under-counts and triggers spurious pool
        // drift while a warm slot is queued. Atomic snapshot —
        // single host_state lock.
        let pool_count = {
            let st = state.host_state.lock();
            (st.pool.unpromoted.len() + st.pool.queue.len()) as u32
        };
        crate::launcher_ipc::report_host_pool_count(pool_count);
    }

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    // The `pool=1` flag tells the frontend to skip its standard
    // workspace init and wait for a `pool:promote` event.
    let url = match super::window::resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}&pool=1",
                base_url, separator, ipc_port, ipc_token, label
            )
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                label = %label,
                "[pool] frontend assets unavailable — pool warmup will load static error page (tear-off targets will surface the install-broken notice instead of crash-looping)",
            );
            super::window::assets_missing_data_url(&e)
        }
    };

    // Phase B.5 (window_meta step d) — combined pre-create handoff.
    // Pool windows graduate to tear-off destinations, which are
    // FullInstance from the user's perspective.
    //
    // Phase F.1 — routed through the host reducer.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind: WindowKind::FullInstance,
                parent_instance_id: None,
            },
        },
    );

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] spawning pool window"
    );

    // Spawn at off-screen coords. The window is technically
    // visible (frameless) but well outside any monitor bounds, so
    // the user never sees it; CEF still paints it because Windows
    // considers it a normal HWND.
    crate::ui_tasks::post_create_window(
        state,
        &url,
        &label,
        POOL_OFFSCREEN_X,
        POOL_OFFSCREEN_Y,
        POOL_WIDTH,
        POOL_HEIGHT,
        true,
    );

    // The window registers in `state.browsers` via on_after_created
    // asynchronously. We don't add to `window_pool` here — the
    // register completion handler does that (see register_pool_window
    // below) so a window only enters the pool after it's actually
    // alive.
}

/// Called from `AgentMuxWindowDelegate::on_window_created` for pool windows
/// (Windows only). Applies `WS_EX_TOOLWINDOW` and caches the HWND at the
/// earliest reliable point — the CEF `CefWindow::GetWindowHandle()` is
/// non-null here, whereas `BrowserHost::window_handle()` may return null
/// by the time `on_after_created` fires (post-page-load CEF behaviour,
/// diagnosed 2026-05-06; see `POOL_HWND_CACHE` doc above).
///
/// `register_pool_window` still runs later as belt-and-suspenders, but the
/// taskbar hide and HWND cache are already correct by then.
#[cfg(target_os = "windows")]
pub fn init_pool_window_hwnd(label: &str, raw_hwnd: *mut std::ffi::c_void) {
    pool_hwnd_cache()
        .lock()
        .unwrap()
        .insert(label.to_string(), raw_hwnd as usize);
    set_taskbar_hidden(raw_hwnd, true);
    tracing::debug!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] HWND cached + taskbar hidden at on_window_created (early path)"
    );
}

/// Called from on_after_created when a pool window's browser is
/// registered. Logs + applies WS_EX_TOOLWINDOW so the off-screen
/// pool window doesn't show up in the taskbar / Alt+Tab. The
/// promote path (`promote_pool_window`) clears it again so the
/// torn-off window IS taskbar-visible.
///
/// Queue insertion still waits for `mark_pool_window_renderer_ready`
/// (frontend-side handshake) — without that gate emit_event_to_window
/// could race the renderer's listener install and drop the promote
/// signal.
pub fn register_pool_window(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("window-pool-") {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        // Look up the HWND under a short browsers lock; release
        // before any Win32 FFI. The HWND should exist by the time
        // on_after_created fires; if it doesn't, log and bail —
        // the pool entry is harmless until promote re-checks.
        use cef::{ImplBrowser, ImplBrowserHost};
        // Phase H.2.b — reducer-aware lookup with fallback.
        let raw_hwnd: Option<*mut std::ffi::c_void> = state
            .get_browser(label)
            .and_then(|browser| {
                browser.host().and_then(|host| {
                    let h = host.window_handle();
                    if h.0.is_null() {
                        None
                    } else {
                        Some(h.0 as *mut std::ffi::c_void)
                    }
                })
            });
        if let Some(hwnd) = raw_hwnd {
            // Cache the HWND for promote-time use. CEF's
            // `BrowserHost::window_handle()` returns null after the
            // page loads (verified diagnostic run 2026-05-06), but
            // the underlying Win32 window is still alive. The cache
            // is the only reliable source for the HWND at promote.
            pool_hwnd_cache()
                .lock()
                .unwrap()
                .insert(label.to_string(), hwnd as usize);
            set_taskbar_hidden(hwnd, true);
        } else {
            tracing::warn!(
                target: "dnd:tearoff:pool",
                label = %label,
                "[pool] HWND null at register time — taskbar hide skipped, cache not populated"
            );
        }
    }
    tracing::debug!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] browser registered, awaiting renderer-ready signal"
    );
}

/// Toggle WS_EX_TOOLWINDOW on a window's extended style so it
/// appears (or doesn't) in the taskbar / Alt+Tab. We use this so
/// pre-warmed pool windows stay invisible to the user, then
/// re-enter the taskbar when promoted to a real torn-off window.
///
/// Per Win32 docs, changing the ex-style after creation only
/// updates the taskbar reliably if the window is hidden + reshown.
/// We do that hide/show cycle here with SWP_FRAMECHANGED so
/// the change takes effect even if SW_HIDE was already implicit.
#[cfg(target_os = "windows")]
fn set_taskbar_hidden(hwnd: *mut std::ffi::c_void, hidden: bool) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            GWL_EXSTYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        };
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if hidden {
            ex |= WS_EX_TOOLWINDOW as isize;
            ex &= !(WS_EX_APPWINDOW as isize);
        } else {
            ex &= !(WS_EX_TOOLWINDOW as isize);
            ex |= WS_EX_APPWINDOW as isize;
        }
        // Hide → write style → show forces the shell to re-evaluate
        // the taskbar entry. Without the hide/show pair Windows often
        // keeps the original taskbar state on style change.
        let _ = ShowWindow(hwnd, SW_HIDE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        let _ = SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        // Don't re-show pool windows — they should stay hidden until
        // promote. Re-show only when transitioning OUT of toolwindow.
        if !hidden {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
    }
}

/// Called when a pool window is destroyed before it ever became
/// renderer-ready (renderer crash mid-init, user closed at OS level,
/// etc.). Without this clearing path the respawn semaphore would
/// stay locked forever and the pool would never refill.
pub fn on_pool_window_destroyed(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("window-pool-") {
        return;
    }
    // Drop the cached HWND so the map can't grow unbounded across the
    // process lifetime. Idempotent — fine if the entry isn't present
    // (e.g. a window destroyed before register_pool_window populated
    // the cache).
    #[cfg(target_os = "windows")]
    {
        pool_hwnd_cache().lock().unwrap().remove(label);
        // Likewise drop the cached CEF Views Window. Only `take_pool_window_view`
        // (on promote) otherwise removes entries, so a pool window that dies
        // *before* promote would leak its cached `cef::Window` for the process
        // lifetime. Idempotent — `take_*` already removed it if this is a
        // post-promote close. (reagent P2 PR #1654.) Windows-only: the cache
        // and `take_pool_window_view` are `#[cfg(target_os = "windows")]`.
        let _ = take_pool_window_view(label);
    }
    // PR #5 H.4 — atomic remove-from-{unpromoted,queue} + clear
    // respawn semaphore via reducer. The dispatch returns:
    //   - `pool_destroyed_was_unpromoted`: distinguishes pre-promote
    //     death (this fn owns the launcher mirror update) from
    //     post-promote close (`on_before_close`'s window-close path
    //     owns it). Without this gate, post-promote closes would
    //     fire `ReportHostCounts` here (browsers already shrunk,
    //     mirror hasn't seen the matching `ReportWindowClosed` yet),
    //     causing a guaranteed transient windows-drift alert on
    //     every normal promoted-window close. (codex P2 PR #578 round-1.)
    //   - `pool_size_after`: queue length after removal — caller
    //     decides refill against POOL_TARGET_SIZE.
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PoolWindowDestroyedBeforePromote {
            label: label.to_string(),
        },
    );

    if dispatch.pool_destroyed_was_unpromoted {
        crate::launcher_ipc::report_pool_window_removed(label.to_string());
        crate::launcher_ipc::compute_and_report_host_counts(state);
    }

    let needs_refill = dispatch
        .pool_size_after
        .map(|n| n < POOL_TARGET_SIZE)
        .unwrap_or(false);
    tracing::warn!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] pool window destroyed before promote — releasing semaphore + refilling"
    );
    if needs_refill {
        spawn_pool_window(state);
    }
}

/// Called from the `pool_window_ready` IPC handler — fired by the
/// frontend's awaitPoolPromote AFTER its `pool:promote` listener
/// is installed. NOW it's safe to enqueue this window for
/// promotion.
pub fn mark_pool_window_renderer_ready(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("window-pool-") {
        return;
    }

    // PR #5 H.4 — atomic move-from-unpromoted-to-queue + clear respawn
    // semaphore via reducer. Idempotent against duplicate frontend
    // signals (re-mount, hot reload). `pool_size_after` is the queue
    // length after the move; caller refills if below target.
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PoolWindowReady { label: label.to_string() },
    );
    let pool_size = dispatch.pool_size_after.unwrap_or(0);

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        pool_size = %pool_size,
        "[pool] pool window renderer ready, enqueued"
    );

    if pool_size < POOL_TARGET_SIZE {
        spawn_pool_window(state);
    }
}

/// Initialize the pool after primary-window first paint. Spawns
/// `POOL_TARGET_SIZE` windows. Called once per app run from
/// `on_after_created` for the "main" label.
///
/// Phase 7: now cross-platform. Pool windows are spawned off-screen at
/// (-32000, -32000) DIP — invisible on macOS Cocoa (accepts any coordinate)
/// and X11 (large-negative coords are in the virtual screen but off all
/// monitors). Windows uses the same off-screen trick but with physical pixels.
pub fn init_pool(state: &Arc<AppState>) {
    // PR #5 H.4 — read pool size via reducer-aware helper.
    let current = state.pool_queue_size();
    if current >= POOL_TARGET_SIZE {
        return;
    }
    // First spawn — the rest are kicked off chain-style by
    // mark_pool_window_renderer_ready when each pool window's renderer
    // reports ready. This sequencing keeps spawns serialised (one CEF
    // window at a time) and avoids spawn pressure spikes at startup.
    spawn_pool_window(state);
}

/// Promote a pool window for tear-off. Pops a label, sends a
/// move-and-show task to the CEF UI thread, and emits
/// `pool:promote` to the renderer with the workspace ID. Returns
/// the promoted window's label so the caller can chain SC_MOVE
/// against it. Returns None if the pool is empty (caller should
/// fall back to the cold path).
///
/// Called from the IPC handler `tear_off_pool_promote`.
#[cfg(target_os = "windows")]
pub fn promote_pool_window(
    state: &Arc<AppState>,
    workspace_id: &str,
    screen_x: i32,
    screen_y: i32,
    width: Option<i32>,
    height: Option<i32>,
    tab_anchor_x: Option<i32>,
    tab_anchor_y: Option<i32>,
    initial_view: Option<String>,
    initial_meta: Option<String>,
) -> Option<String> {
    // PR #5 H.4 — atomic pop+remove via reducer. The dispatch pops
    // the front of the pool queue, removes the label from
    // unpromoted, and clears `is_pool` on the corresponding
    // BrowserHandle, all under one host_state lock. Returns None if
    // the queue is empty (cold-path fallback).
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PopAndPromoteFrontPoolWindow,
    );
    let label = dispatch.promoted_pool_label?;

    // Phase B.4 follow-up — pool inventory shrinks unconditionally on
    // pop. The user-visible WindowOpened report is deferred until
    // after HWND validation succeeds (codex P1 PR #577 round-1):
    // emitting it before validation would record a `WindowOpened`
    // for a label that may never become a real visible window in
    // the failure path (HWND lookup returns None, function early-
    // returns after refill), permanently desyncing the mirror.
    crate::launcher_ipc::report_pool_window_removed(label.clone());

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        workspace_id = %workspace_id,
        screen_x = %screen_x,
        screen_y = %screen_y,
        "[pool] promoting pool window"
    );

    // Resolve the HWND under a SHORT lock — drop the browsers mutex
    // before any Win32 call so we don't hold a global state lock
    // across FFI into the OS UI subsystem.
    //
    // Each None-returning step is a state-inconsistency bug, not an
    // expected failure — log per-step at ERROR so an operator can
    // tell which invariant broke.
    use cef::{ImplBrowser, ImplBrowserHost};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
    // Resolve the HWND. CEF's `BrowserHost::window_handle()` returns
    // null after the page loads on Views-based browsers, even though
    // the underlying Win32 window is alive (verified 2026-05-06,
    // SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md). Use the cache we
    // populated at `register_pool_window`; the CEF path is kept as a
    // first-try in case some future CEF version starts returning the
    // HWND consistently again.
    let raw_hwnd: Option<*mut std::ffi::c_void> = match state.get_browser(&label) {
        None => {
            tracing::error!(
                target: "dnd:tearoff:pool",
                label = %label,
                "[pool] promoted label not in browsers map (state inconsistency)"
            );
            None
        }
        Some(browser) => match browser.host() {
            None => {
                tracing::error!(
                    target: "dnd:tearoff:pool",
                    label = %label,
                    "[pool] browser has no host (state inconsistency)"
                );
                None
            }
            Some(host) => {
                let cef_hwnd = host.window_handle().0;
                if !cef_hwnd.is_null() {
                    Some(cef_hwnd as *mut std::ffi::c_void)
                } else {
                    // CEF lost the reference — fall back to cache.
                    let cached = pool_hwnd_cache().lock().unwrap().get(&label).copied();
                    match cached {
                        None => {
                            tracing::error!(
                                target: "dnd:tearoff:pool",
                                label = %label,
                                "[pool] CEF HWND null AND no cache entry (state inconsistency)"
                            );
                            None
                        }
                        Some(h) => {
                            // Verify the cached HWND is still a live
                            // OS window. If the OS has reclaimed it,
                            // the slot is genuinely dead; refuse the
                            // promote and fall back to cold-path.
                            let alive = unsafe { IsWindow(h as HWND) } != 0;
                            if alive {
                                tracing::debug!(
                                    target: "dnd:tearoff:pool",
                                    label = %label,
                                    hwnd = format!("0x{:x}", h),
                                    "[pool] using cached HWND (CEF returned null)"
                                );
                                Some(h as *mut std::ffi::c_void)
                            } else {
                                tracing::error!(
                                    target: "dnd:tearoff:pool",
                                    label = %label,
                                    hwnd = format!("0x{:x}", h),
                                    "[pool] cached HWND no longer a live window"
                                );
                                None
                            }
                        }
                    }
                }
            }
        },
    };

    // Pool-slot leak guard: if HWND lookup fails after we've already
    // popped the label, capacity permanently shrinks unless we refill.
    //
    // Orphan cleanup (B.5c smoke test caught this): the popped label is
    // still in `state.browsers` but is no longer in `unpromoted_pool_labels`
    // (we removed it at the top of this fn) and never became a real
    // user-visible window (`report_window_opened` is gated on the
    // post-validation success path). Without explicit cleanup the
    // host's `compute_and_report_host_counts` filter
    // (`browsers - panes - unpromoted`) counts the orphan as a window,
    // producing persistent windows-drift against the launcher mirror
    // (which correctly never received a `WindowOpened` for it).
    //
    // `cleanup_failed_promote_orphan` is responsible for ALL recovery
    // including pool refill — see its contract. We deliberately do NOT
    // call `spawn_pool_window` here since the cleanup helper either
    // (a) issues `close_browser` → `on_before_close` → `on_pool_window_destroyed`
    // already triggers refill, or (b) does direct cleanup + refill
    // itself. Calling refill from both paths produces double refill.
    // (codex P1 PR #582 round-1.)
    let raw_hwnd = match raw_hwnd {
        Some(h) => h,
        None => {
            cleanup_failed_promote_orphan(state, &label);
            return None;
        }
    };

    // Register the promoted window's outer top-level HWND under its label in the
    // chrome-resolution cache (`window_hwnds`) so the new window's title-bar
    // DRAG / CLOSE / MINIMIZE / MAXIMIZE act on THIS window. Pool windows
    // otherwise never land in `window_hwnds`: `capture_hwnd_for_label` reads
    // `host.window_handle()`, which is null for pool windows
    // (SPEC_POOL_WINDOW_HWND_NULL), so `resolve_window_hwnd` fell back to
    // `find_own_top_level_window()` (the MAIN window) — which is why dragging the
    // new window's header moved the original, and close/maximize would hit the
    // wrong window. `raw_hwnd` is the CefWindow top-level handle (already the
    // outer HWND), so store it directly; this also overwrites any stale entry
    // pointing at the off-screen pre-promote pool window.
    state
        .window_hwnds
        .lock()
        .insert(label.clone(), raw_hwnd as isize);

    // Phase F.5 — explicit promote signal sent BETWEEN the matching
    // `report_pool_window_removed` (above) and `report_window_opened`
    // (next). The launcher's pool-respawn saga starts on the
    // resulting `Event::PoolWindowPromoted` and bracket the
    // subsequent refill in `SagaStarted`/`SagaCompleted` so the
    // renderer can buffer "you got a tear-off + the pool is
    // refilling" atomically. Sent only on the validated-HWND path
    // (mirrors `report_window_opened`'s contract); pre-promote
    // destroy paths emit only `report_pool_window_removed` with no
    // promote signal so the saga doesn't fire on non-promote drains.
    crate::launcher_ipc::report_pool_window_promoted(label.clone());

    // HWND validated — the label IS becoming a real user-visible
    // window. NOW report the open to the launcher mirror so a
    // failure path above can't leave the mirror with a phantom
    // entry. (codex P1 PR #577 round-1.)
    crate::launcher_ipc::report_window_opened(
        label.clone(),
        agentmux_common::ipc::WindowKind::FullInstance,
        None,
    );

    // PR #664 codex P2 — explicit AUTHORITATIVE HWND link for
    // promoted pool windows. Pool windows skip the explicit
    // report_hwnd_opened branch in `client.rs::on_after_created`
    // (gated on `!label.starts_with("window-pool-")`) because their
    // initial registration happens before promotion. The launcher's
    // drain-on-WindowOpened fallback (in `handle_report_window_opened`)
    // would only link a recent pending HWND if one happened to be in
    // the 2s window — pre-promote pool windows are usually older than
    // that, leaving the mirror permanently hwnd=None. This explicit
    // link guarantees the mirror tracks the HWND so WRR
    // visibility/foreground/orphan-destroy drift detection works for
    // every torn-off window. The accompanying repair logic in
    // `apply_hwnd_opened` corrects any wrong drain-pick.
    crate::launcher_ipc::report_hwnd_opened(
        raw_hwnd as u64,
        "Chrome_WidgetWin_1".to_string(),
        label.clone(),
        Some(label.clone()),
    );

    // Phase B.4 follow-up — drift check after the atomic
    // pool→windows transition.
    crate::launcher_ipc::compute_and_report_host_counts(state);

    // Compute position outside the unsafe block — these are pure
    // arithmetic, no FFI needed. Don't clamp with .max(0): Windows'
    // virtual screen space is signed (secondary monitors to the left
    // of or above the primary have negative coords), and clamping
    // would push tear-offs onto the primary monitor when the user
    // grabbed from a secondary.
    // Use the source window's dimensions when provided (tear-off
    // UX: new window matches the frame the user dragged from). Fall
    // back to the pool default otherwise.
    //
    // DPI conversion (codex P2 / reagent P1 PR #727): the frontend
    // sends `window.outerWidth/Height` in CSS/DIP pixels but Win32
    // `SetWindowPos` expects PHYSICAL pixels. Use the DESTINATION
    // monitor's DPI (the one under cursor at the drop point), NOT
    // the pool HWND's current monitor — pool windows live at
    // POOL_OFFSCREEN_X/Y which is typically the primary monitor, so
    // on mixed-DPI multi-monitor the pool HWND's DPI doesn't match
    // the user's actual drop target.
    let dpi_scale: f32 = unsafe {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{
            MonitorFromPoint, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        let pt = POINT { x: screen_x, y: screen_y };
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x: u32 = 0;
        let mut dpi_y: u32 = 0;
        let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        if hr != 0 || dpi_x == 0 { 1.0 } else { dpi_x as f32 / 96.0 }
    };
    let to_physical = |dip: i32| -> i32 { (dip as f32 * dpi_scale).round() as i32 };
    let win_w_dip = width.unwrap_or(POOL_WIDTH);
    let win_h_dip = height.unwrap_or(POOL_HEIGHT);
    let win_w = if width.is_some() { to_physical(win_w_dip) } else { win_w_dip };
    let win_h = if height.is_some() { to_physical(win_h_dip) } else { win_h_dip };

    // Position the new window. With tab anchor (frontend captured the
    // grab offset within the source tab and converted to a screen
    // point): place the new window's first tab top-left at the anchor
    // so the cursor stays on the same visual element across the
    // handoff. Without anchor: cursor-centered title bar (legacy).
    //
    // Position units (codex P1 PR #730 round 1): the anchor and
    // screen_x/y arrive in the SAME unit as the legacy `screen_x -
    // win_w / 2` math (the unit CEF reports for `window.screenX`).
    // The inset constants (`FIRST_TAB_INSET_X`, `TAB_STRIP_TOP_OFFSET_PX`)
    // are LOGICAL/DIP pixels, but they're SMALL (8 / 16) and the
    // cold-path drag.rs uses them raw too — converting them here in
    // ONE path would make the warm and cold paths land at different
    // offsets on HiDPI. Keep both paths consistent by using the
    // constants raw. The width/height conversion above is independent
    // and addresses the SetWindowPos-wants-physical-pixels constraint
    // for SIZE; that conversion stays.
    // No `.max(0)` clamp on the anchor branch (codex P2 PR #730 round
    // 2): on multi-monitor setups where a secondary display is to the
    // left of or above the primary, screen coords can legitimately be
    // negative, and clamping to 0 would yank the window back onto the
    // primary monitor. The legacy fallback also doesn't clamp.
    //
    // Anchor semantics (refined post-PR #730 smoke): tab_anchor_{x,y}
    // is now the OUTER TOP-LEFT of the new window, not the screen
    // position of the grabbed tab. Frontend computes
    //   anchor = cursor_screen - grab_offset - source_chrome_inset
    // so its hardcoded chrome inset (was FIRST_TAB_INSET_X /
    // TAB_STRIP_TOP_OFFSET_PX) is gone — frontend measures the source
    // window's actual chrome dynamically. Backend just places the
    // window at anchor with no further offset.
    let (pos_x, pos_y) = match (tab_anchor_x, tab_anchor_y) {
        (Some(ax), Some(ay)) => (ax, ay),
        _ => (
            screen_x - win_w / 2,
            screen_y - TITLE_BAR_OFFSET_PX,
        ),
    };

    // Safety net (PLAN_POOL_NEW_WINDOW_DPI_POSITIONING_2026_06_21): clamp the
    // target rect to the DESTINATION monitor's work area so a HiDPI coordinate
    // miscalc can't strand the promoted window off-screen — the "blank new
    // window" bug where the window rendered but sat at the DPI-scaled
    // POOL_OFFSCREEN. The anchor/origin picks the monitor. Use the PHYSICAL work
    // area: pos/size here and SetWindowPos below are physical pixels, so the DIP
    // variant would over-constrain on HiDPI (reagent P1 #1652).
    let (pos_x, pos_y, win_w, win_h) =
        match crate::app::get_monitor_work_area_physical(pos_x, pos_y) {
            Some((wa_x, wa_y, wa_w, wa_h)) => {
                clamp_rect_within(pos_x, pos_y, win_w, win_h, wa_x, wa_y, wa_w, wa_h)
            }
            None => (pos_x, pos_y, win_w, win_h),
        };

    // FIX — Stage 0 (RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21,
    // cef#3638): position the window at its final ON-SCREEN rect while it is still
    // HIDDEN, then perform the FIRST show THERE.
    //
    // Pool windows are spawned visible-but-OFF-SCREEN, then immediately
    // SW_HIDE'd (set_taskbar_hidden(true)) in init_pool_window_hwnd, so by
    // promote time the raw HWND is hidden. The previous order re-showed it
    // first — via set_taskbar_hidden(false)'s internal SW_SHOWNA — at the
    // OFF-SCREEN pool position, and THEN moved it on-screen. On Windows that binds the browser
    // compositor's visibility/surface state to the off-screen show, and the
    // subsequent move+resize never re-syncs it, so the promoted window paints
    // BLANK despite a valid DOM. Showing for the first time at the final on-screen
    // position gives Chromium the genuine hidden->visible transition it needs.
    // (macOS is unaffected — occlusion-driven visibility / IOSurface.)
    //
    // SWP_NOZORDER is intentionally *not* set on the placement move — tear-off
    // needs the window at the top of the Z-order for the SC_MOVE mouse-capture
    // handshake.
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, SetWindowPos, ShowWindow, HWND_TOP, SW_SHOW, SWP_NOACTIVATE,
        };

        // 1. Position to the final on-screen rect WHILE HIDDEN (no show yet).
        let pos_ok = SetWindowPos(raw_hwnd, HWND_TOP, pos_x, pos_y, win_w, win_h, 0);
        if pos_ok == 0 {
            // Non-fatal: the SC_MOVE handshake still runs; the user may see a
            // misplaced window. Log so we can detect a pattern.
            let err = windows_sys::Win32::Foundation::GetLastError();
            tracing::error!(
                target: "dnd:tearoff:pool",
                label = %label,
                last_err = %err,
                "[pool] SetWindowPos failed"
            );
        }

        // 2. Apply the taskbar (APPWINDOW) style AND perform the genuine first
        //    show: set_taskbar_hidden(false)'s SW_HIDE -> style -> SW_SHOWNA cycle
        //    is now the first hidden->visible transition, at the on-screen rect
        //    set in step 1 (set_taskbar_hidden's inner SetWindowPos is NOMOVE/
        //    NOSIZE, so it preserves that position).
        set_taskbar_hidden(raw_hwnd, false);

        // 3. Ensure shown + activated (SW_SHOWNA above does not activate).
        let _ = ShowWindow(raw_hwnd, SW_SHOW);

        // 4. Re-assert the rect after show + off-screen telemetry (#1652).
        //    SWP_NOACTIVATE so we don't steal focus again; HWND_TOP keeps it frontmost.
        let _ = SetWindowPos(raw_hwnd, HWND_TOP, pos_x, pos_y, win_w, win_h, SWP_NOACTIVATE);
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(raw_hwnd, &mut r) != 0 {
            // Physical work area: GetWindowRect is physical px (reagent P2 #1652).
            if let Some((wx, wy, ww, wh)) =
                crate::app::get_monitor_work_area_physical(pos_x, pos_y)
            {
                let onscreen =
                    r.right > wx && r.left < wx + ww && r.bottom > wy && r.top < wy + wh;
                if !onscreen {
                    tracing::error!(
                        target: "pool:new-window",
                        label = %label,
                        left = r.left,
                        top = r.top,
                        "[pool] promoted window still off-screen after post-show re-assert"
                    );
                }
            }
        }

        // Pool windows skip the cascade-hook install in on_after_created
        // (gated on !label.starts_with("window-pool-")) because they are
        // hidden off-screen at creation time. Install it here after the
        // window is visible and promoted so floaters torn from this window
        // follow its minimize/restore/destroy.
        crate::client::install_main_window_floater_cascade_hook(raw_hwnd);
    }

    // macOS-PARITY VISIBILITY FIX (the load-bearing one): drive the CEF Views
    // `Window` set_bounds() + show() exactly as the macOS/Linux promote does — but
    // POSTED TO THE UI THREAD, because CEF Views calls are UI-thread-only and this
    // promote runs on the IPC thread (the previous in-line attempt no-op'd: the
    // thread-local cache + show ran on the wrong thread). The Window was cached at
    // on_window_created (browser_view.window() is None for pool windows post-load
    // on Windows). The Win32 SetWindowPos + ShowWindow above position/show the raw
    // HWND but do NOT flip the browser's view-hierarchy/compositor visibility —
    // only the Views Window show() does, which is the one thing macOS does and
    // Windows didn't, and is why macOS renders and Windows paints blank. set_bounds
    // is DIP, so convert the physical rect we positioned the HWND at.
    {
        let scale = crate::app::dpi_scale_at(pos_x, pos_y);
        let to_dip = |v: i32| (v as f32 / scale).round() as i32;
        crate::ui_tasks::post_promote_pool_window_views_show(
            state,
            &label,
            to_dip(pos_x),
            to_dip(pos_y),
            to_dip(win_w),
            to_dip(win_h),
        );
    }

    // Phase B.7.3.3 — the launcher's typed events drive the
    // InstancePanel atoms via the CEF JS bridge. No sync emit here.

    // Now tell the pool window's renderer to bootstrap the workspace.
    crate::events::emit_event_to_window(
        state,
        &label,
        "pool:promote",
        &serde_json::json!({
            "workspaceId": workspace_id,
            "initialView": initial_view,
            "initialMeta": initial_meta,
        }),
    );

    // Refill the pool in the background.
    spawn_pool_window(state);

    Some(label)
}

/// Phase B.5c follow-up — clean up an orphan pool window left behind
/// when `promote_pool_window`'s HWND validation fails. Without this,
/// the popped label sits in `state.browsers` but is no longer in
/// `unpromoted_pool_labels` (promote removed it) and never became a
/// real user window (no `WindowOpened` was reported). Host's
/// `compute_and_report_host_counts` then counts it as a window, while
/// the launcher mirror correctly does not — persistent off-by-one
/// drift. (Caught by B.4b drift detection during B.5c smoke test on
/// v0.33.461.)
///
/// Contract: this fn is responsible for ALL recovery including pool
/// refill. The caller MUST NOT call `spawn_pool_window` itself —
/// double refill would overshoot `POOL_TARGET_SIZE` and waste
/// renderer capacity. (codex P1 PR #582 round-1.)
///
/// Two paths:
///
/// * **Graceful path** (browser+host alive in CEF): issue
///   `close_browser(1)` and let CEF's `on_before_close` fire. That
///   path runs the standard cleanup chain — drops from
///   `state.browsers` + `window_meta`, calls
///   `on_pool_window_destroyed` (which itself triggers refill via
///   `spawn_pool_window` when pool size is below target), and
///   triggers `compute_and_report_host_counts` from `client.rs`.
/// * **Direct path** (browser or host already gone): `on_before_close`
///   won't fire so we do its job inline — drop from browsers +
///   window_meta, send `report_window_closed` (silent no-op in
///   launcher reducer for an unknown label), spawn the refill
///   ourselves, and emit a drift count snapshot so the orphan's
///   removal is observable on the same tick. (reagent P2 PR #582
///   round-1 — original direct path skipped the snapshot.)
#[cfg(target_os = "windows")]
fn cleanup_failed_promote_orphan(state: &Arc<AppState>, label: &str) {
    use cef::{ImplBrowser, ImplBrowserHost};
    // Phase H.2.b — reducer-aware lookup with fallback.
    let mut browser_clone = state.get_browser(label);
    if let Some(ref mut browser) = browser_clone {
        if let Some(host) = browser.host() {
            // force_close = 1: don't run beforeunload, we know
            // this window never reached a useful state.
            host.close_browser(1);
            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %label,
                "[pool] orphan close_browser issued — on_before_close will run cleanup + refill"
            );
            return;
        }
    }
    // Browser or host already gone — do `on_before_close`'s job
    // inline since CEF won't fire it for this label.
    // Phase H.2.d — legacy `state.browsers.lock().remove` removed;
    // reducer's UnregisterBrowser is sole canonical mutation site.
    state.host_dispatch(
        crate::reducer::HostCommand::UnregisterBrowser {
            label: label.to_string(),
        },
    );
    state.window_meta.lock().remove(label);
    crate::launcher_ipc::report_window_closed(label.to_string());
    // Refill (graceful path gets this via on_pool_window_destroyed).
    spawn_pool_window(state);
    // Emit a count snapshot now so the orphan's removal is
    // observable in the launcher's drift stream on the same tick.
    crate::launcher_ipc::compute_and_report_host_counts(state);
    tracing::warn!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] orphan browser already gone — cleaned host state directly + refilled"
    );
}

/// Phase 7 — macOS / Linux pool promotion via CEF Views set_bounds().
///
/// Pops a pool window, repositions it from its off-screen holding position
/// to the tear-off destination using CEF Views Window::set_bounds() (DIP
/// coordinates; no Win32 DPI scaling needed), and emits pool:promote so the
/// renderer attaches the new workspace. Falls back gracefully (returns None)
/// if the pool is empty or the window was destroyed before promotion.
#[cfg(not(target_os = "windows"))]
pub fn promote_pool_window(
    state: &Arc<AppState>,
    workspace_id: &str,
    screen_x: i32,
    screen_y: i32,
    width: Option<i32>,
    height: Option<i32>,
    tab_anchor_x: Option<i32>,
    tab_anchor_y: Option<i32>,
    _initial_view: Option<String>,
    _initial_meta: Option<String>,
) -> Option<String> {
    // Atomic pop from the pool queue via reducer. Returns None if empty
    // — caller falls back to cold path.
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PopAndPromoteFrontPoolWindow,
    );
    let label = dispatch.promoted_pool_label?;

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        workspace_id = %workspace_id,
        screen_x,
        screen_y,
        "[pool] promoting pool window (non-Windows)"
    );

    // Validate browser is still alive. On non-Windows we don't cache a
    // native window handle; CEF state presence is the liveness check.
    if state.get_browser(&label).is_none() {
        tracing::warn!(
            target: "dnd:tearoff:pool",
            label = %label,
            "[pool] promoted browser not found in state — orphan cleanup"
        );
        cleanup_failed_promote_orphan_cross_platform(state, &label);
        return None;
    }

    // Compute window position. Tab anchor (outer top-left of new window so
    // the cursor lands on the dragged tab) takes priority; otherwise place
    // the window at the cursor.
    let x = tab_anchor_x.unwrap_or(screen_x);
    let y = tab_anchor_y.unwrap_or(screen_y);
    let w = width.unwrap_or(POOL_WIDTH);
    let h = height.unwrap_or(POOL_HEIGHT);

    // Reposition + emit pool:promote on the CEF UI thread.
    crate::ui_tasks::post_promote_pool_window(state, &label, workspace_id, x, y, w, h);

    // Refill the pool asynchronously.
    spawn_pool_window(state);

    Some(label)
}

/// Orphan cleanup for non-Windows promote failures.
///
/// Called only when `state.get_browser(label).is_none()` (the browser has
/// already left state before we could promote it), so there is no graceful
/// `close_browser` path — `on_before_close` will not fire. Do its job inline.
#[cfg(not(target_os = "windows"))]
fn cleanup_failed_promote_orphan_cross_platform(state: &Arc<AppState>, label: &str) {
    state.host_dispatch(
        crate::reducer::HostCommand::UnregisterBrowser {
            label: label.to_string(),
        },
    );
    state.window_meta.lock().remove(label);
    crate::launcher_ipc::report_window_closed(label.to_string());
    spawn_pool_window(state);
    crate::launcher_ipc::compute_and_report_host_counts(state);
    tracing::warn!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] orphan browser already gone — cleaned host state directly + refilled (non-Windows)"
    );
}

/// Pop a pool window for a new top-level window (Cmd+N, File → New Window).
///
/// Reuses the tab tear-off pool. No workspace_id — the frontend creates a fresh
/// workspace via the absence of `?workspaceId=` in the URL, which causes
/// `initHostNewWindow` to take the "create fresh workspace" branch.
///
/// macOS / Linux: emits `pool:new-window` (no workspaceId).
/// Windows: delegates to `promote_pool_window` with `workspace_id=""`. The
/// frontend's `awaitPoolPromote` receives `pool:promote { workspaceId: "" }` and
/// sets `?workspaceId=` (empty string) in the URL; `initHostNewWindow` reads the
/// param as falsy and falls through to the fresh-workspace path. Position is
/// passed as the tab anchor so it feeds directly to `SetWindowPos` without the
/// cursor-centering offset math. Width/height pass as `None` so the function
/// uses the hardcoded `POOL_WIDTH/POOL_HEIGHT` defaults (1200×800) — this
/// avoids double-applying the DIP→physical DPI conversion: `get_offset_position`
/// and `get_secondary_window_size` on Windows already return physical pixels
/// (from `GetWindowRect`/`GetMonitorInfoW`), but `promote_pool_window` only
/// runs `to_physical()` when `width.is_some()`. Passing `None` skips it.
pub fn promote_pool_window_for_new_window(
    state: &Arc<AppState>,
    pos_x: i32,
    pos_y: i32,
    width: i32,
    height: i32,
    initial_view: Option<String>,
    initial_meta: Option<String>,
) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        // Width/height intentionally passed as None (use POOL_WIDTH/POOL_HEIGHT defaults)
        // to avoid double-DPI-converting the already-physical pixels from GetWindowRect.
        // pos_x/pos_y passed as the tab anchor so SetWindowPos places the window there
        // directly without the cursor-centering arithmetic.
        let _ = (width, height); // used only on non-Windows path
        return promote_pool_window(
            state,
            "",           // empty workspace_id → frontend creates fresh workspace
            pos_x,
            pos_y,
            None,         // width: skip DPI conversion, use POOL_WIDTH default
            None,         // height: skip DPI conversion, use POOL_HEIGHT default
            Some(pos_x),  // tab_anchor_x: physical px placed directly in SetWindowPos
            Some(pos_y),  // tab_anchor_y
            initial_view,
            initial_meta,
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        let dispatch = state.host_dispatch(
            crate::reducer::HostCommand::PopAndPromoteFrontPoolWindow,
        );
        let label = dispatch.promoted_pool_label?;

        tracing::info!(
            target: "pool:new-window",
            label = %label,
            pos_x,
            pos_y,
            width,
            height,
            "[pool:new-window] promoting pool window for new window"
        );

        if state.get_browser(&label).is_none() {
            tracing::warn!(
                target: "pool:new-window",
                label = %label,
                "[pool:new-window] browser not in state — orphan cleanup"
            );
            cleanup_failed_promote_orphan_cross_platform(state, &label);
            return None;
        }

        crate::ui_tasks::post_promote_pool_window_for_new_window(
            state, &label, pos_x, pos_y, width, height, initial_view, initial_meta,
        );
        spawn_pool_window(state);

        Some(label)
    }
}

// ── Pane pool (floating-pool-{uuid}, frameless=true) ─────────────────────

/// Target size for the pane pool. One pre-warmed window covers the common
/// burst; two would add ~75 MB RSS with minimal additional benefit.
pub const PANE_POOL_TARGET_SIZE: usize = 1;
pub(crate) const PANE_POOL_WIDTH: i32 = 900;
pub(crate) const PANE_POOL_HEIGHT: i32 = 600;

/// Spawn a single pre-warmed frameless pane window. Follows the same
/// single-flight + refill chain pattern as `spawn_pool_window`.
pub fn spawn_pane_pool_window(state: &Arc<AppState>) {
    // PR #1612 landed (merged 2026-06-20) — Windows promote_pane_pool_window
    // now uses the WS_POPUP+WS_EX_TOOLWINDOW HWND approach and returns Some
    // instead of None. The early-return guard that blocked spawning on Windows
    // was not removed in that PR; removing it here enables the pool on Windows.
    if state.is_quitting() {
        return;
    }
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[pane-pool] spawn_pane_pool_window deferred — pane is mid-close (H.7 invariant)"
        );
        return;
    }
    if state.pane_pool_queue_size() >= PANE_POOL_TARGET_SIZE {
        tracing::debug!(
            target: "pool:pane",
            current = %state.pane_pool_queue_size(),
            target = %PANE_POOL_TARGET_SIZE,
            "[pane-pool] spawn skipped — pool at target size"
        );
        return;
    }

    let window_id = uuid::Uuid::new_v4();
    let label = format!("floating-pool-{}", window_id.simple());

    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PanePoolWindowSpawnStart { label: label.clone() },
    );
    if !dispatch.pane_pool_spawn_proceeding {
        return;
    }

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let url = match super::window::resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let sep = if base_url.contains('?') { "&" } else { "?" };
            format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}&pane-pool=1",
                base_url, sep, ipc_port, ipc_token, label
            )
        }
        Err(e) => {
            tracing::error!(error = %e, label = %label, "[pane-pool] frontend assets unavailable");
            super::window::assets_missing_data_url(&e)
        }
    };

    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind: WindowKind::FullInstance,
                parent_instance_id: None,
            },
        },
    );

    tracing::info!(target: "pool:pane", label = %label, "[pane-pool] spawning pane pool window");

    // On Windows: create a WS_POPUP + WS_EX_TOOLWINDOW window (same type as the
    // cold-path `post_create_floating_window`) so promote can reuse the same Win32
    // HWND without re-creating the window at tear-off time.
    // On non-Windows: use CEF Views frameless window.
    #[cfg(target_os = "windows")]
    {
        crate::floating_pane::post_create_pane_pool_window_win32(state, &label, &url);
    }
    #[cfg(not(target_os = "windows"))]
    {
        crate::ui_tasks::post_create_window(
            state,
            &url,
            &label,
            POOL_OFFSCREEN_X,
            POOL_OFFSCREEN_Y,
            PANE_POOL_WIDTH,
            PANE_POOL_HEIGHT,
            true,
        );
    }
}

/// Called once at startup (when main window registers) to seed the pane pool.
pub fn init_pane_pool(state: &Arc<AppState>) {
    if state.pane_pool_queue_size() >= PANE_POOL_TARGET_SIZE {
        return;
    }
    spawn_pane_pool_window(state);
}

/// Called from `on_after_created` for `floating-pool-*` labels.
/// Logs the registration; queue insertion waits for the frontend's
/// `pane_pool_window_ready` IPC so we don't race the listener install.
pub fn register_pane_pool_window(_state: &Arc<AppState>, label: &str) {
    if !label.starts_with("floating-pool-") {
        return;
    }
    tracing::debug!(
        target: "pool:pane",
        label = %label,
        "[pane-pool] browser registered, awaiting renderer-ready signal"
    );
}

/// Called by the `pane_pool_window_ready` IPC command when the frontend
/// has installed its `pool:pane-promote` listener and is ready to receive.
pub fn mark_pane_pool_window_renderer_ready(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("floating-pool-") {
        return;
    }
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PanePoolWindowReady { label: label.to_string() },
    );
    let pool_size = dispatch.pane_pool_size_after.unwrap_or(0);
    tracing::info!(
        target: "pool:pane",
        label = %label,
        pool_size,
        "[pane-pool] renderer ready, enqueued"
    );
    if pool_size < PANE_POOL_TARGET_SIZE {
        spawn_pane_pool_window(state);
    }
}

/// Called from `on_before_close` for `floating-pool-*` labels.
pub fn on_pane_pool_window_destroyed(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("floating-pool-") {
        return;
    }
    // Drop the cached outer HWND so the map can't grow unbounded. Idempotent —
    // fine if the entry is already absent (double-destroy or failure path).
    #[cfg(target_os = "windows")]
    {
        pane_pool_hwnd_cache().lock().unwrap().remove(label);
    }
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PanePoolWindowDestroyedBeforePromote {
            label: label.to_string(),
        },
    );
    let needs_refill = dispatch
        .pane_pool_size_after
        .map(|n| n < PANE_POOL_TARGET_SIZE)
        .unwrap_or(false);
    tracing::warn!(
        target: "pool:pane",
        label = %label,
        "[pane-pool] pool window destroyed before promote"
    );
    if needs_refill {
        spawn_pane_pool_window(state);
    }
}

/// Orphan cleanup for failed pane pool promotes.
fn cleanup_failed_pane_promote_orphan(state: &Arc<AppState>, label: &str) {
    state.host_dispatch(
        crate::reducer::HostCommand::UnregisterBrowser {
            label: label.to_string(),
        },
    );
    state.window_meta.lock().remove(label);
    spawn_pane_pool_window(state);
    tracing::warn!(
        target: "pool:pane",
        label = %label,
        "[pane-pool] orphan cleaned up — refilled"
    );
}

/// Pop a pane pool window and show it as a floating pane at the drop target.
///
/// `parent_hwnd`: the FullInstance HWND that triggered the tear-off (for cascade
/// hook binding on Windows). Pass 0 on non-Windows — it is unused there.
///
/// macOS / Linux: CEF Views `set_bounds` + `show` + `pool:pane-promote`.
/// Windows: Win32 `SetWindowPos` + `ShowWindow` on the outer WS_POPUP HWND
/// cached by `CreatePanePoolWindowWin32Task`, then `pool:pane-promote` event.
pub fn promote_pane_pool_window(
    state: &Arc<AppState>,
    pane_id: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent_hwnd: isize,
) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsWindow, SetWindowPos, ShowWindow, HWND_TOP, SW_SHOWNORMAL,
        };

        let dispatch = state.host_dispatch(
            crate::reducer::HostCommand::PopAndPromoteFrontPanePoolWindow,
        );
        let label = dispatch.promoted_pane_pool_label?;

        tracing::info!(
            target: "pool:pane",
            label = %label,
            pane_id = %pane_id,
            workspace_id = %workspace_id,
            x, y, width, height,
            "[pane-pool] promoting pane pool window (Windows)"
        );

        let outer_hwnd = match take_pane_pool_hwnd(&label) {
            Some(h) => h,
            None => {
                tracing::error!(
                    target: "pool:pane",
                    label = %label,
                    "[pane-pool] no cached HWND for promoted label — orphan cleanup"
                );
                cleanup_failed_pane_promote_orphan(state, &label);
                return None;
            }
        };

        // Verify the HWND is still alive before touching it.
        let alive = unsafe { IsWindow(outer_hwnd as HWND) } != 0;
        if !alive {
            tracing::error!(
                target: "pool:pane",
                label = %label,
                hwnd = format!("0x{:x}", outer_hwnd),
                "[pane-pool] cached HWND is no longer a live OS window — orphan cleanup"
            );
            cleanup_failed_pane_promote_orphan(state, &label);
            return None;
        }

        // `width` and `height` arrive pre-DPI-converted (physical pixels) from
        // `open_floating_pane_window`'s Windows block. `x` and `y` are in the
        // same coordinate space as `CreateWindowExW` (physical screen coords).
        unsafe {
            let ok = SetWindowPos(outer_hwnd as HWND, HWND_TOP, x, y, width, height, 0);
            if ok == 0 {
                let err = windows_sys::Win32::Foundation::GetLastError();
                tracing::error!(
                    target: "pool:pane",
                    label = %label,
                    last_err = %err,
                    "[pane-pool] SetWindowPos failed"
                );
            }
            let _ = ShowWindow(outer_hwnd as HWND, SW_SHOWNORMAL);
        }

        // Rename `floating-pool-<uuid>` → `floating-<uuid>` so the promoted
        // pane is counted as a real user floater rather than filtered out as a
        // warm pool window (SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30). Re-key
        // every per-label store BEFORE the emit below — `emit_event_to_window`
        // resolves the target via `get_browser(label)`, which after the reducer
        // re-key lives under `new_label`.
        let new_label = label.replacen("floating-pool-", "floating-", 1);
        // Reducer state: `browsers` (+ the duplicated label field) and, if
        // present, `window_opacities` / `pane_window_states`.
        let _ = state.host_dispatch(crate::reducer::HostCommand::RelabelBrowser {
            old_label: label.clone(),
            new_label: new_label.clone(),
        });
        // AppState label maps (not reducer state).
        {
            let mut hwnds = state.window_hwnds.lock();
            if let Some(h) = hwnds.remove(&label) {
                hwnds.insert(new_label.clone(), h);
            }
        }
        {
            let mut meta = state.window_meta.lock();
            if let Some(mut m) = meta.remove(&label) {
                m.label = new_label.clone();
                meta.insert(new_label.clone(), m);
            }
        }

        // Bind this floater to the source window's cascade hook. Deferred from
        // spawn time because the parent HWND is only known at tear-off.
        crate::floating_pane::register_floater_hwnd(
            new_label.clone(),
            outer_hwnd as isize,
            parent_hwnd,
        );

        // Tell the renderer to bootstrap pane + workspace. Carry the new window
        // label so `awaitPanePoolPromote` rewrites its `?windowLabel=` param —
        // otherwise the renderer keeps addressing the host by the dead pool
        // label (spec §5.2).
        crate::events::emit_event_to_window(
            state,
            &new_label,
            "pool:pane-promote",
            &serde_json::json!({
                "paneId": pane_id,
                "workspaceId": workspace_id,
                "windowLabel": new_label,
            }),
        );

        spawn_pane_pool_window(state);

        return Some(new_label);
    }

    #[cfg(not(target_os = "windows"))]
    {
        // NOTE: the `floating-pool-*` → `floating-*` relabel
        // (SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30) is currently applied on
        // the Windows path only — that's where the bug was reported and where
        // it can be runtime-verified. The equivalent re-key + payload
        // `windowLabel` should be applied here (via `post_promote_pane_pool_window`)
        // as a follow-up on macOS/Linux.
        let _ = parent_hwnd; // unused on non-Windows
        let dispatch = state.host_dispatch(
            crate::reducer::HostCommand::PopAndPromoteFrontPanePoolWindow,
        );
        let label = dispatch.promoted_pane_pool_label?;

        tracing::info!(
            target: "pool:pane",
            label = %label,
            pane_id = %pane_id,
            workspace_id = %workspace_id,
            x, y, width, height,
            "[pane-pool] promoting pane pool window"
        );

        if state.get_browser(&label).is_none() {
            tracing::warn!(
                target: "pool:pane",
                label = %label,
                "[pane-pool] browser not in state — orphan cleanup"
            );
            cleanup_failed_pane_promote_orphan(state, &label);
            return None;
        }

        crate::ui_tasks::post_promote_pane_pool_window(
            state, &label, pane_id, workspace_id, x, y, width, height,
        );
        spawn_pane_pool_window(state);

        Some(label)
    }
}
