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

/// Target pool size. See module-level comment for rationale.
pub const POOL_TARGET_SIZE: usize = 2;

/// Pool windows are spawned at this off-screen position so they
/// don't appear on the user's desktop while pre-painting. On
/// promote they're moved to the cursor and shown.
const POOL_OFFSCREEN_X: i32 = -32000;
const POOL_OFFSCREEN_Y: i32 = -32000;
const POOL_WIDTH: i32 = 1200;
const POOL_HEIGHT: i32 = 800;
/// Pixels above the cursor where the title bar sits — matches
/// open_window_at_position so the cursor lands near the top-center
/// of the title bar after promotion.
const TITLE_BAR_OFFSET_PX: i32 = 16;

/// Tab-anchor placement constants. When the frontend supplies
/// `tabAnchorX/Y` (the screen point where the user grabbed the tab),
/// the new window is positioned so its FIRST TAB's top-left lands at
/// that anchor — cursor stays on the same visual element across the
/// handoff. Spec: SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md §4.4.
///
/// `FIRST_TAB_INSET_X` is the left-edge gap from the window's outer
/// frame to where the first tab actually starts (tab strip leading
/// padding). `TAB_STRIP_TOP_OFFSET_PX` is the vertical distance from
/// the window's outer top to the top of the tab strip — title bar
/// sits above it.
pub(super) const FIRST_TAB_INSET_X: i32 = 8;
pub(super) const TAB_STRIP_TOP_OFFSET_PX: i32 = TITLE_BAR_OFFSET_PX;

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
    let base_url = super::window::resolve_frontend_base_url(ipc_port);
    let separator = if base_url.contains('?') { "&" } else { "?" };
    // The `pool=1` flag tells the frontend to skip its standard
    // workspace init and wait for a `pool:promote` event.
    let url = format!(
        "{}{}ipc_port={}&ipc_token={}&windowLabel={}&pool=1",
        base_url, separator, ipc_port, ipc_token, label
    );

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
/// Windows-only: promote_pool_window is a no-op on non-Windows
/// platforms (Phase 7 will add equivalents). Spawning hidden pool
/// windows that can never be consumed would just waste renderer
/// processes, so we skip the whole init off-Win32.
pub fn init_pool(state: &Arc<AppState>) {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        return;
    }
    #[cfg(target_os = "windows")]
    {
        // PR #5 H.4 — read pool size via reducer-aware helper.
        let current = state.pool_queue_size();
        if current >= POOL_TARGET_SIZE {
            return;
        }
        // First spawn — the rest are kicked off chain-style by
        // register_pool_window when each new pool window registers.
        // This sequencing keeps spawns serialised (one CEF window at
        // a time) and avoids spawn pressure spikes at startup.
        spawn_pool_window(state);
    }
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
    let (pos_x, pos_y) = match (tab_anchor_x, tab_anchor_y) {
        (Some(ax), Some(ay)) => (
            ax - FIRST_TAB_INSET_X,
            ay - TAB_STRIP_TOP_OFFSET_PX,
        ),
        _ => (
            screen_x - win_w / 2,
            screen_y - TITLE_BAR_OFFSET_PX,
        ),
    };

    // Take the window out of WS_EX_TOOLWINDOW so the promoted window
    // appears in the taskbar / Alt+Tab like any other AgentMux
    // instance. Must run BEFORE the position/show below; otherwise
    // the taskbar entry won't appear until the next style refresh.
    set_taskbar_hidden(raw_hwnd, false);

    // Reposition + raise to top + show. SWP_NOZORDER is intentionally
    // *not* set — for tear-off we need the new window at the top of
    // the Z-order so the subsequent SC_MOVE handshake routes the
    // mouse-capture correctly. With SWP_NOZORDER set, HWND_TOP would
    // be silently ignored.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOP, SW_SHOW,
        };
        let pos_ok = SetWindowPos(
            raw_hwnd,
            HWND_TOP,
            pos_x,
            pos_y,
            win_w,
            win_h,
            0, // no flags — apply move + size + Z-order all
        );
        if pos_ok == 0 {
            // Non-fatal: the SC_MOVE handshake will still try to
            // run, but the user may see a misplaced window. Log
            // for diagnostics so we can detect a pattern.
            let err = windows_sys::Win32::Foundation::GetLastError();
            tracing::error!(
                target: "dnd:tearoff:pool",
                label = %label,
                last_err = %err,
                "[pool] SetWindowPos failed"
            );
        }
        let _ = ShowWindow(raw_hwnd, SW_SHOW);
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

#[cfg(not(target_os = "windows"))]
pub fn promote_pool_window(
    _state: &Arc<AppState>,
    _workspace_id: &str,
    _screen_x: i32,
    _screen_y: i32,
    _width: Option<i32>,
    _height: Option<i32>,
    _tab_anchor_x: Option<i32>,
    _tab_anchor_y: Option<i32>,
) -> Option<String> {
    // Non-Windows: pool isn't built yet (Phase 7). Caller falls
    // back to the cold path.
    None
}
