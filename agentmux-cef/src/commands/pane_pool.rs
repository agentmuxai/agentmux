// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Pane pool — pre-warmed frameless `floating-pool-{uuid}` windows used to
// eliminate first-paint flash on floating-pane tear-off, the same problem
// `window_pool.rs` solves for full top-level `?pool=1` windows. Split out of
// that file (which originally covered both subsystems) because the two pools
// are independently lifecycled: separate HWND caches, separate target sizes,
// separate spawn/promote/evict/destroy paths, and the pane pool additionally
// owns a Win32 `WS_POPUP` + `WS_EX_TOOLWINDOW` HWND-reuse promote path
// (`promote_pane_pool_window`) that has no top-level-pool analogue.
//
// `window_pool.rs` re-exports this module (`pub use super::pane_pool::*;`) so
// the existing `window_pool::…` call sites elsewhere in the crate keep working
// unchanged.

use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashMap;

use crate::state::{AppState, WindowKind};
// `Task`/`WrapTask`/`ImplTask` are what `cef::wrap_task!` expands into — needed
// in scope for the `DestroyPanePoolHwndTask` definition below (B.5 Part 1, issue
// #2218). Named imports rather than `use cef::*` (the pattern browser_panes.rs
// uses) to avoid widening this namespace.
#[cfg(target_os = "windows")]
use cef::{rc::Rc, ImplTask, Task, WrapTask};

// Off-screen spawn position shared with the top-level window pool — pane-pool
// windows are parked at the same coordinates while pre-painting.
use super::window_pool::{POOL_OFFSCREEN_X, POOL_OFFSCREEN_Y};

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

// ── Pane pool (floating-pool-{uuid}, frameless=true) ─────────────────────

/// Target size for the pane pool. One pre-warmed window covers the common
/// burst; two would add ~75 MB RSS with minimal additional benefit.
pub const PANE_POOL_TARGET_SIZE: usize = 1;
pub(crate) const PANE_POOL_WIDTH: i32 = 900;
pub(crate) const PANE_POOL_HEIGHT: i32 = 600;

/// Evict one idle (renderer-ready, never-promoted) pane-pool window on a
/// memory-pressure transition into Warn/Critical (issue #2218, B.5 Part 1).
/// Until this existed, the only pressure reaction anywhere in this crate was
/// `spawn_pool_window`/`spawn_pane_pool_window` refusing to grow the pool —
/// nothing ever shrank an already-warm one, so commit charge only ever
/// ratcheted up during a session.
///
/// Deliberately reuses the plain, already-reliable owned-popup `DestroyWindow`
/// mechanism (the same one `floating_pane.rs`'s failure-cleanup path uses on
/// its own outer HWND) rather than routing through `ui_tasks::post_close_window`
/// / `CloseWindowTask` — that function's `main`/`window-*`/`floating-*`
/// branching carries a lot of history (Discussion #1680: orphaned-process-tree
/// regressions, srv-notify races) built for a *user* closing a *visible*
/// window, none of which applies to the host quietly reclaiming a pool window
/// the user never saw. Pane-pool windows are unowned top-level `WS_POPUP`s the
/// app itself created, not `WS_CHILD` of `main` like browser panes (which
/// need the wrapper-reparent dance) — a direct `DestroyWindow` on the thread
/// that created it is the same, already-proven-safe shape *for the HWND*.
///
/// The queue claim itself is atomic: `HostCommand::PopFrontPanePoolWindowForEviction`
/// pops-and-clears the front label under the same `host_state` mutex every
/// other pool mutation goes through, so this can never race a concurrent
/// `PopAndPromoteFrontPanePoolWindow` (a real user tear-off) for the same
/// label — a prior version peeked `queue.front()` non-destructively and
/// separately mutated, leaving a window where eviction could destroy a
/// window the user had just promoted (reagent P2, round 2).
///
/// The Browser itself still needs arming first, though: `close_browser(1)`
/// must run on the CEF UI thread — calling it directly from this function
/// (which runs on the mem-heartbeat background thread) risks a UI-thread
/// hang (reagent P1, round 2). So both `close_browser(1)` and the native
/// `DestroyWindow` are done inside the SAME posted `DestroyPanePoolHwndTask`,
/// on `cef::ThreadId::UI`, mirroring `CloseWindowTask`'s round-5 sequencing
/// exactly (`ui_tasks/window.rs`) — `close_browser(1)` first is what drives
/// `on_before_close` -> reducer `UnregisterBrowser`, which cleans the
/// `state.browsers` map entry; that's a SEPARATE piece of state from
/// `pane_pool.queue`/`unpromoted` (already cleared by the atomic pop above).
///
/// Returns `true` iff a label was popped and a destroy was posted.
#[cfg(target_os = "windows")]
pub fn evict_idle_pane_pool_window(state: &Arc<AppState>) -> bool {
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PopFrontPanePoolWindowForEviction,
    );
    let Some(label) = dispatch.evicted_pane_pool_label else {
        return false;
    };
    let Some(hwnd) = take_pane_pool_hwnd(&label) else {
        tracing::warn!(
            target: "pool:pane",
            label = %label,
            "[pane-pool] eviction: no cached HWND for queued label — pool short by one"
        );
        return false;
    };
    let mut task = DestroyPanePoolHwndTask::new(Arc::clone(state), label.clone(), hwnd as isize);
    let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
    if posted == 0 {
        tracing::error!(
            target: "pool:pane",
            label = %label,
            hwnd,
            "[pane-pool] eviction: post_task(destroy) failed — window will leak"
        );
        return false;
    }
    tracing::info!(
        target: "pool:pane",
        label = %label,
        "[pane-pool] evicted idle pool window under memory pressure"
    );
    true
}

#[cfg(not(target_os = "windows"))]
pub fn evict_idle_pane_pool_window(_state: &Arc<AppState>) -> bool {
    // No live-verified reliable destroy path on macOS/Linux for a pane-pool
    // window yet (mirrors demote_promoted_pool_window/park_and_blank_window's
    // existing Windows-only scope in this file) — matches the pressure
    // spawn-refusal guards above, which already degrade to a no-op question
    // that never arises off-Windows (memory_pressure::current_level() has no
    // Windows-only gate, but there's nothing to evict if nothing is ever
    // trimmed here).
    false
}

#[cfg(target_os = "windows")]
cef::wrap_task! {
    struct DestroyPanePoolHwndTask {
        state: Arc<AppState>,
        label: String,
        hwnd: isize,
    }

    impl Task {
        fn execute(&self) {
            // Arm the browser destruction BEFORE the native DestroyWindow —
            // the same "close_browser(1) first" sequencing CloseWindowTask's
            // round 5 uses (ui_tasks/window.rs), and for the same reason:
            // this is what drives on_before_close -> reducer
            // UnregisterBrowser, which cleans the state.browsers map entry.
            // Must run here, on the CEF UI thread this task is posted to
            // (cef::ThreadId::UI) — calling close_browser off-thread risks a
            // UI-thread hang (reagent P1, round 2).
            {
                use cef::{ImplBrowser, ImplBrowserHost};
                if let Some(mut browser) = self.state.get_browser(&self.label) {
                    if let Some(host) = browser.host() {
                        tracing::debug!(
                            target: "pool:pane",
                            label = %self.label,
                            "[pane-pool] eviction: close_browser(1) to arm destruction"
                        );
                        host.close_browser(1);
                    }
                } else {
                    tracing::warn!(
                        target: "pool:pane",
                        label = %self.label,
                        "[pane-pool] eviction: no Browser found for label — state.browsers entry may already be stale"
                    );
                }
            }
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
                DestroyWindow(self.hwnd as *mut std::ffi::c_void);
            }
        }
    }
}

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
    // Commit-pressure guard — see the matching comment in spawn_pool_window.
    if crate::memory_pressure::current_level() != crate::memory_pressure::PressureLevel::Normal {
        tracing::debug!(
            target: "pool:pane",
            level = crate::memory_pressure::current_level().as_str(),
            "[pane-pool] spawn skipped — commit pressure"
        );
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
        let relabel = state.host_dispatch(crate::reducer::HostCommand::RelabelBrowser {
            old_label: label.clone(),
            new_label: new_label.clone(),
        });
        if !relabel.relabel_ok {
            // The browser vanished between promote and relabel (concurrent
            // close-before-promote race). Bail rather than re-key the AppState
            // maps under `new_label` while `browsers` still holds `old_label`
            // — that split would make the emit below resolve no browser and
            // leave dangling AppState entries. Clean up under the old label.
            tracing::warn!(
                target: "pool:pane",
                old_label = %label,
                new_label = %new_label,
                "[relabel] relabel failed (browser gone?) — aborting promotion, orphan cleanup"
            );
            cleanup_failed_pane_promote_orphan(state, &label);
            return None;
        }
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

        // Re-point the Ctrl+Wheel hook at the NEW label. Promotion relabels the
        // browser and bootstraps the renderer via `pool:pane-promote` +
        // `history.replaceState` — there is NO navigation, so `on_load_end`
        // never fires again and the hook's context would keep emitting to the
        // dead `floating-pool-*` label. Every Ctrl+Wheel in a pool-promoted
        // floater would then be discarded, and close-time cleanup (keyed on the
        // new label) would not find the context either.
        //
        // This REWRITES the existing context rather than installing a second
        // one. Contexts are keyed by HWND and `find_context` walks UP from the
        // deepest descendant, so a fresh entry on the outer popup would still
        // lose to the stale entry on the browser HWND beneath it.
        // (codex P1 on PR #2884.)
        #[cfg(target_os = "windows")]
        crate::floater_wheel::relabel_floater_ctrl_wheel_hook(&label, &new_label);

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
