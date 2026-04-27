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

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::state::{AppState, WindowKind, WindowMeta};

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

/// Spawn a single pool window. Called at startup (N times) and
/// after each promote (1 refill). Idempotent against the
/// in-flight semaphore — concurrent calls collapse to one spawn
/// in flight at a time.
pub fn spawn_pool_window(state: &Arc<AppState>) {
    // Single-flight: skip if a respawn is already pending. The
    // pending one will catch up to TARGET_SIZE; we don't need
    // to stack spawns.
    if state
        .window_pool_respawn_in_flight
        .swap(true, Ordering::AcqRel)
    {
        tracing::debug!(
            target: "dnd:tearoff:pool",
            "[pool] spawn skipped — respawn already in flight"
        );
        return;
    }

    let window_id = uuid::Uuid::new_v4();
    // Use the `window-pool-` prefix so existing `is_instance_label`
    // checks (tear_off_hook.rs, app-init.ts) pass naturally — they
    // accept anything starting with `window-`. After promotion the
    // label stays the same; "is in window_pool queue" is the
    // authoritative pool-vs-promoted distinction.
    let label = format!("window-pool-{}", window_id.simple());

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

    // Mark as full instance — the window will graduate to a
    // tear-off destination, which is just another instance window
    // from the user's perspective.
    state.window_meta.lock().insert(
        label.clone(),
        WindowMeta {
            label: label.clone(),
            kind: WindowKind::FullInstance,
            parent_instance_id: None,
        },
    );

    state
        .pending_window_labels
        .lock()
        .push_back(label.clone());

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
/// registered. Logs only — the actual queue insertion happens via
/// `mark_pool_window_renderer_ready` once the frontend reports its
/// `pool:promote` listener is installed. Without that gate we'd
/// race emit_event_to_window against the renderer's listener
/// install, dropping the promote signal and stranding the window
/// in pool mode.
pub fn register_pool_window(_state: &Arc<AppState>, label: &str) {
    if !label.starts_with("window-pool-") {
        return;
    }
    tracing::debug!(
        target: "dnd:tearoff:pool",
        label = %label,
        "[pool] browser registered, awaiting renderer-ready signal"
    );
}

/// Called from the `pool_window_ready` IPC handler — fired by the
/// frontend's awaitPoolPromote AFTER its `pool:promote` listener
/// is installed. NOW it's safe to enqueue this window for
/// promotion.
pub fn mark_pool_window_renderer_ready(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("window-pool-") {
        return;
    }
    // Single lock acquisition for both push_back and len. Avoids the
    // window where another thread could push between our two locks
    // and skew pool_size, plus saves one mutex round-trip on the hot
    // post-creation path.
    let pool_size = {
        let mut pool = state.window_pool.lock();
        pool.push_back(label.to_string());
        pool.len()
    };
    state
        .window_pool_respawn_in_flight
        .store(false, Ordering::Release);

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        pool_size = %pool_size,
        "[pool] pool window renderer ready, enqueued"
    );

    // Top up to target if we're below.
    if pool_size < POOL_TARGET_SIZE {
        spawn_pool_window(state);
    }
}

/// Initialize the pool after primary-window first paint. Spawns
/// `POOL_TARGET_SIZE` windows. Called once per app run from
/// `on_after_created` for the "main" label.
pub fn init_pool(state: &Arc<AppState>) {
    let current = state.window_pool.lock().len();
    if current >= POOL_TARGET_SIZE {
        return;
    }
    // First spawn — the rest are kicked off chain-style by
    // register_pool_window when each new pool window registers.
    // This sequencing keeps spawns serialised (one CEF window at
    // a time) and avoids spawn pressure spikes at startup.
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
) -> Option<String> {
    let label = state.window_pool.lock().pop_front()?;

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
    let raw_hwnd: Option<*mut std::ffi::c_void> = {
        let browsers = state.browsers.lock();
        match browsers.get(&label) {
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
                    let h = host.window_handle();
                    if h.0.is_null() {
                        tracing::error!(
                            target: "dnd:tearoff:pool",
                            label = %label,
                            "[pool] host window handle is null (state inconsistency)"
                        );
                        None
                    } else {
                        Some(h.0 as *mut std::ffi::c_void)
                    }
                }
            },
        }
    };

    // Pool-slot leak guard: if HWND
    // lookup fails after we've already popped the label, capacity
    // permanently shrinks unless we refill. The orphan window (if
    // any) is logged above and abandoned.
    let raw_hwnd = match raw_hwnd {
        Some(h) => h,
        None => {
            spawn_pool_window(state);
            return None;
        }
    };

    // Compute position outside the unsafe block — these are pure
    // arithmetic, no FFI needed. Don't clamp with .max(0): Windows'
    // virtual screen space is signed (secondary monitors to the left
    // of or above the primary have negative coords), and clamping
    // would push tear-offs onto the primary monitor when the user
    // grabbed from a secondary.
    let pos_x = screen_x - POOL_WIDTH / 2;
    let pos_y = screen_y - TITLE_BAR_OFFSET_PX;

    // Reposition + raise to top + show. SWP_NOZORDER is intentionally
    // *not* set — for tear-off we need the new window at the top of
    // the Z-order so the subsequent SC_MOVE handshake routes the
    // mouse-capture correctly. With SWP_NOZORDER set, HWND_TOP would
    // be silently ignored.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOP, SW_SHOW,
        };
        SetWindowPos(
            raw_hwnd,
            HWND_TOP,
            pos_x,
            pos_y,
            POOL_WIDTH,
            POOL_HEIGHT,
            0, // no flags — apply move + size + Z-order all
        );
        let _ = ShowWindow(raw_hwnd, SW_SHOW);
    }

    // Register the promoted window in the instance registry +
    // broadcast the count change. Cold-path open_window_at_position
    // does this same step (drag.rs:~390); without it warm-path
    // tear-offs would bypass instance tracking entirely — get_instance
    // _number would fall back to 1 for the new window and other
    // windows would keep a stale count, throwing off the InstancePanel
    // and any other consumer of windowCountAtom.
    {
        let mut reg = state.window_instance_registry.lock();
        let num = reg.register(&label);
        tracing::info!(
            target: "dnd:tearoff:pool",
            label = %label,
            instance = %num,
            "[pool] promoted window registered as instance"
        );
    }
    let count = state.window_instance_registry.lock().count();
    crate::events::emit_event_all_windows(
        state,
        "window-instances-changed",
        &serde_json::json!(count),
    );

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

#[cfg(not(target_os = "windows"))]
pub fn promote_pool_window(
    _state: &Arc<AppState>,
    _workspace_id: &str,
    _screen_x: i32,
    _screen_y: i32,
) -> Option<String> {
    // Non-Windows: pool isn't built yet (Phase 7). Caller falls
    // back to the cold path.
    None
}
