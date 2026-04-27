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
    let label = format!("pool-{}", window_id.simple());

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
/// registered. Marks the pool slot as available.
pub fn register_pool_window(state: &Arc<AppState>, label: &str) {
    if !label.starts_with("pool-") {
        return;
    }
    state.window_pool.lock().push_back(label.to_string());
    state
        .window_pool_respawn_in_flight
        .store(false, Ordering::Release);

    let pool_size = state.window_pool.lock().len();
    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        pool_size = %pool_size,
        "[pool] pool window registered, ready"
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

    // Reposition + show via Win32 SetWindowPos. The window's HWND
    // is in state.browsers; we run this synchronously since
    // SetWindowPos is thread-safe (per the codebase convention in
    // ui_tasks.rs).
    unsafe {
        use cef::{ImplBrowser, ImplBrowserHost};
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOP, SWP_NOZORDER, SW_SHOW,
        };

        let browsers = state.browsers.lock();
        let browser = browsers.get(&label)?;
        let host = browser.host()?;
        let hwnd = host.window_handle();
        if hwnd.0.is_null() {
            tracing::warn!(
                target: "dnd:tearoff:pool",
                label = %label,
                "[pool] promoted window has null HWND — refusing"
            );
            return None;
        }
        let raw_hwnd = hwnd.0 as HWND;
        // Center the window so the cursor lands near the top-center
        // of the title bar (matches open_window_at_position).
        let pos_x = (screen_x - POOL_WIDTH / 2).max(0);
        let pos_y = (screen_y - 16).max(0);
        SetWindowPos(
            raw_hwnd,
            HWND_TOP,
            pos_x,
            pos_y,
            POOL_WIDTH,
            POOL_HEIGHT,
            SWP_NOZORDER,
        );
        let _ = ShowWindow(raw_hwnd, SW_SHOW);
    }

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
