// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window lifecycle commands + HWND resolution helpers for the CEF host.
//
// First carve of the `commands/window.rs` modularization
// (docs/analysis/ANALYSIS_LARGE_FILE_MODULARIZATION_CANDIDATES_2026_05_28.md,
// Plan 1). Holds the close path, the per-label top-level-HWND resolver,
// the EnumWindows fallbacks, and the per-label HWND cache capture —
// everything that answers "which OS window does this label map to, and
// how do we close it." The motion / chrome / transparency / meta /
// creation handlers stay in `mod.rs` (later carves).
//
// `resolve_window_hwnd` and `find_all_own_windows` are `pub(super)`: they
// are consumed by the motion / transparency handlers that still live in
// `mod.rs`. `close_window` / `close_window_by_label` are `pub` (dispatched
// by ipc.rs); `find_own_top_level_window` / `capture_hwnd_for_label` are
// `pub(crate)` (browser_pane, client, backend resolve them as
// `commands::window::<name>`). All but the two cross-platform close
// handlers are Windows-only.

use std::sync::Arc;

use crate::state::AppState;

// Both traits are needed by the Windows-only resolver/capture fns below
// (`Browser::host` needs ImplBrowser; `BrowserHost::window_handle` needs
// ImplBrowserHost). cfg-gated so non-Windows builds — where those fns
// compile out — don't see an unused import.
#[cfg(target_os = "windows")]
use cef::{ImplBrowser, ImplBrowserHost};

/// Close the window. Args: optional `{ "label": string }`; defaults to "main".
/// Routes by label via `resolve_window_hwnd` — without that the floater
/// (owned, drawn above its owner in Z-order) would always swallow the
/// close because `find_own_top_level_window` returns the topmost-Z
/// visible top-level of the process.
pub fn close_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_close_window(state, label);
    let _ = (state, label);
    Ok(serde_json::Value::Null)
}

/// Close a specific window by label. Used by the tear-off Phase 4
/// merge path: after the candidate window pulls the dragged tab into
/// its own workspace via MoveTabToWorkspace, the dragged window is
/// empty and should be destroyed. Posts WM_CLOSE on Win32; uses the
/// existing UI-thread close task on other platforms.
pub fn close_window_by_label(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing label".to_string())?
        .to_string();

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        // Route through resolve_window_hwnd so we hit the cached OUTER
        // top-level HWND. The reducer registry returns CEF's inner
        // WS_CHILD for `set_as_child` browsers (and for floaters our
        // outer popup HWND is only in the cache), so going straight to
        // `host.window_handle()` would WM_CLOSE the embedded child —
        // after a redock that leaves the outer popup as an empty shell.
        let hwnd = resolve_window_hwnd(state, &label);
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return Ok(serde_json::Value::Null);
        }
        return Err(format!("no top-level HWND for label {}", label));
    }

    #[cfg(not(target_os = "windows"))]
    {
        crate::ui_tasks::post_close_window(state, &label);
        Ok(serde_json::Value::Null)
    }
}

/// Resolve a top-level HWND for the given label. Prefer the reducer
/// registry (`state.get_browser(label)` → host → window_handle → walk
/// to root) over the process-wide `find_own_top_level_window` fallback.
///
/// Why the label matters: `find_own_top_level_window` does an
/// `EnumWindows` and returns the *first* visible top-level of the
/// current process. Z-order puts **owned** windows ABOVE their owner,
/// so as soon as a floating-pane window exists, every label-less
/// `get/set_window_position` call (e.g. the main window's
/// `useWindowDrag`) accidentally targets the floater — dragging the
/// main window moves the floater instead.
///
/// `GetAncestor(hwnd, GA_ROOT)` guard handles the case where CEF
/// returns the embedded browser's WS_CHILD HWND rather than our
/// outer top-level — without it, `SetWindowPos` would only shift the
/// child within its parent, not move the outer floater.
/// Class name of the floating-pane outer HWND
/// (`agentmux-cef/src/floating_pane.rs::CLASS_NAME`). Kept in sync so
/// `find_main_window` can EnumWindows-skip floaters when CEF Views
/// hides the main window's HWND.
// Use floater_class_name() (not a const) so both files agree on the
// runtime-suffixed name that embeds AGENTMUX_IPC_HASH (I5 invariant).
#[cfg(target_os = "windows")]
#[inline(always)]
fn floating_pane_class_name() -> &'static str {
    crate::floating_pane::floater_class_name()
}

/// X-coordinate threshold (in screen px) below which a top-level window is
/// treated as an off-screen warm-pool member, not a real user window.
///
/// Unpromoted pool windows are created at `POOL_OFFSCREEN_X` = -32000
/// (`window_pool.rs`) and stay there until promoted — but they remain
/// `IsWindowVisible`, so the EnumWindows-based fallbacks below would
/// otherwise enumerate one and bind `"main"` to a window the user can't
/// see. Drags and closes then act on the hidden pool window instead of the
/// visible one (root cause of the "window won't drag" regression: every
/// `set_window_position` faithfully moved an off-screen pool window). A
/// failed drag can nudge the parked window a few hundred px before the bind
/// is fixed, so we test against a generous threshold rather than the exact
/// parking coordinate. No real monitor origin is anywhere near -20000
/// (even a 4K monitor left of primary sits at ~-3840).
#[cfg(target_os = "windows")]
const OFFSCREEN_POOL_THRESHOLD_X: i32 = -20000;

/// True if `hwnd` is parked at the warm-pool's off-screen position (see
/// `OFFSCREEN_POOL_THRESHOLD_X`). Used by both `find_main_window` and the
/// `capture_hwnd_for_label` fallback to refuse binding an on-screen label
/// to a hidden pool window. On `GetWindowRect` failure we return `false`
/// (can't prove it's a pool window — don't skip).
#[cfg(target_os = "windows")]
unsafe fn is_offscreen_pool_window(hwnd: *mut std::ffi::c_void) -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut rect: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return false;
    }
    rect.left < OFFSCREEN_POOL_THRESHOLD_X
}

/// Like `find_own_top_level_window` but skips floating-pane windows.
/// Used when the label points at the main window but the reducer-
/// registry path failed (CEF Views' `BrowserHost::window_handle()`
/// returns NULL on Win32 for Views-based browsers). Without the
/// skip, the floater (owned, drawn ABOVE its owner) would be
/// enumerated first and we'd target it instead.
#[cfg(target_os = "windows")]
pub(super) unsafe fn find_main_window() -> *mut std::ffi::c_void {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let _pid = GetCurrentProcessId();
    let mut result: *mut std::ffi::c_void = std::ptr::null_mut();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid != GetCurrentProcessId() {
            return 1;
        }
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        // Read the window class name; skip if it matches the floating-
        // pane class. `GetClassNameW` writes up to `cchClassMaxCount`
        // UTF-16 code units (excluding the null terminator).
        let mut buf: [u16; 64] = [0; 64];
        let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            let class = String::from_utf16_lossy(&buf[..len as usize]);
            if class == floating_pane_class_name() {
                return 1;
            }
        }
        // Skip unpromoted warm-pool windows parked off-screen. They share
        // the main window's `Chrome_WidgetWin_1` class (so the class check
        // above can't catch them) and are `IsWindowVisible`, but they are
        // NOT the real promoted main window — binding to one breaks
        // drag/close. See `is_offscreen_pool_window`.
        if is_offscreen_pool_window(hwnd) {
            return 1;
        }
        let result_ptr = lparam as *mut *mut std::ffi::c_void;
        *result_ptr = hwnd;
        0
    }

    EnumWindows(Some(enum_callback), &mut result as *mut _ as isize);
    result
}

#[cfg(target_os = "windows")]
pub(crate) unsafe fn resolve_window_hwnd(state: &Arc<AppState>, label: &str) -> *mut std::ffi::c_void {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, GA_ROOT};

    if !label.is_empty() {
        // 1. Consult the authoritative per-label HWND cache FIRST —
        //    populated by `capture_hwnd_for_label` for main / pool /
        //    tear-off windows AND by `floating_pane.rs` for our own
        //    owned-popup floaters at create time. The cache stores
        //    the actual outer top-level HWND we want to act on.
        //    We MUST NOT walk GA_ROOT on cache hits: the cached value
        //    is already the top-level we want; walking up could land
        //    on a different owner (e.g. main, for floaters owned by
        //    main) and cause `close_window_by_label` to post WM_CLOSE
        //    to the wrong window.
        //
        // Liveness guard: `capture_hwnd_for_label` has no eviction
        // path, and CEF Views can swap the outer HWND on window
        // recreate. A cache hit pointing at a dead HWND silently
        // posts WM_CLOSE into the void (broken title-bar close
        // button observed v0.39.1 → SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md).
        // Validate via IsWindow before trusting; on stale, evict
        // and fall through to the registry/EnumWindows paths.
        let cached = state.window_hwnds.lock().get(label).copied();
        if let Some(raw_isize) = cached {
            let raw = raw_isize as *mut std::ffi::c_void;
            if !raw.is_null() {
                if IsWindow(raw) != 0 {
                    tracing::info!(
                        target: "win-resolve",
                        label = %label,
                        cache_hwnd = ?raw,
                        "[win-resolve] resolved via window_hwnds cache"
                    );
                    return raw;
                }
                tracing::warn!(
                    target: "win-resolve",
                    label = %label,
                    stale_hwnd = ?raw,
                    "[win-resolve] cache hit was stale (IsWindow=false); evicting"
                );
                state.window_hwnds.lock().remove(label);
            }
        }

        // 2. Fall back to the CEF reducer registry. This returns
        //    `host.window_handle()` which on Win32 is usually a
        //    WS_CHILD inner HWND for `set_as_child` browsers — so we
        //    DO need GA_ROOT here to walk up to the top-level. Used
        //    when capture_hwnd_for_label hasn't run yet (very early
        //    startup) for non-floater labels.
        if let Some(browser) = state.get_browser(label) {
            if let Some(host) = browser.host() {
                let raw = host.window_handle().0 as *mut std::ffi::c_void;
                if !raw.is_null() {
                    let root = GetAncestor(raw, GA_ROOT);
                    let resolved = if root.is_null() { raw } else { root };
                    tracing::info!(
                        target: "win-resolve",
                        label = %label,
                        host_hwnd = ?raw,
                        root_hwnd = ?resolved,
                        "[win-resolve] resolved via reducer registry + GA_ROOT"
                    );
                    return resolved;
                }
            }
        }

        tracing::warn!(
            target: "win-resolve",
            label = %label,
            "[win-resolve] cache + reducer-registry both empty — using class-aware EnumWindows fallback"
        );
    }

    // 3. EnumWindows last resort. CEF Views (main window) hides its
    //    HWND behind a Views container, so this branch fires for
    //    "main" before the user has triggered the init-status path
    //    (e.g. cold-boot drag attempts). Z-order returns the floater
    //    first (owned windows draw ABOVE their owner), so for "main"
    //    we use a class-aware enumerator that skips the floating-pane
    //    window class — deterministic regardless of Z-order. For
    //    non-"main" labels with neither cache nor registry entry,
    //    plain `find_own_top_level_window` is the best we can do.
    let fallback = if label == "main" {
        find_main_window()
    } else {
        find_own_top_level_window()
    };
    tracing::info!(
        target: "win-resolve",
        label = %label,
        fallback_hwnd = ?fallback,
        "[win-resolve] class-aware EnumWindows fallback"
    );
    fallback
}

/// Find ALL visible top-level windows belonging to this process.
#[cfg(target_os = "windows")]
pub(super) fn find_all_own_windows() -> Vec<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let mut results: Vec<*mut std::ffi::c_void> = Vec::new();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            let results = &mut *(lparam as *mut Vec<*mut std::ffi::c_void>);
            results.push(hwnd);
        }
        1 // Continue
    }

    unsafe {
        EnumWindows(Some(enum_callback), &mut results as *mut _ as isize);
    }
    results
}

/// Find the top-level window belonging to this process.
/// In CEF Views mode, browser.host().window_handle() returns NULL,
/// so we enumerate windows and find ours by process ID.
///
/// ⚠️ **LABEL-LESS LAST RESORT — never call this when a window label is
/// available.** It returns the process's FIRST visible top-level, and owned
/// floater popups draw ABOVE their owner in Z-order, so once any floater
/// exists this returns the *floater*, not main. That is the root of the
/// recurring "wrong window" bug class (#1165, #1166, the 2026-05-30
/// browser-pane parent bug). Any handler that has a `label` must resolve via
/// [`resolve_window_hwnd`] instead. The only legitimate callers are the
/// non-"main" fallback inside `resolve_window_hwnd` and genuinely label-less
/// paths (the floater's own owner at create, the deprecated `move_window_by`).
/// See P1 in docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md.
#[cfg(target_os = "windows")]
pub(crate) unsafe fn find_own_top_level_window() -> *mut std::ffi::c_void {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let pid = GetCurrentProcessId();
    let mut result: *mut std::ffi::c_void = std::ptr::null_mut();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            // Store the HWND in the pointer passed via lparam
            let result_ptr = lparam as *mut *mut std::ffi::c_void;
            *result_ptr = hwnd;
            return 0; // Stop enumeration
        }
        1 // Continue
    }

    let _ = pid; // Used inside callback via GetCurrentProcessId()
    EnumWindows(
        Some(enum_callback),
        &mut result as *mut _ as isize,
    );
    result
}

/// Capture and store the HWND for `label` in `AppState::window_hwnds`.
///
/// Called from `set_window_init_status` once the frontend signals "ready".
/// Two-pass approach:
/// 1. Fast path: `browser.host().window_handle()` — may be non-NULL by this
///    point even in CEF Views mode (window fully shown).
/// 2. Fallback: enumerate all process-owned visible HWNDs and pick the one
///    not yet registered in `window_hwnds`. Reliable because windows are
///    opened sequentially (pool windows are hidden before promotion).
#[cfg(target_os = "windows")]
pub(crate) fn capture_hwnd_for_label(state: &Arc<AppState>, label: &str) {
    // If a known-good outer HWND was already inserted by the creator
    // of this window (e.g. `floating_pane.rs::create_owned_popup`
    // registers the outer floater HWND it built via CreateWindowExW),
    // DO NOT overwrite it. The fast path below uses
    // `host.window_handle()` which for `set_as_child` browsers returns
    // the CEF inner WS_CHILD HWND — replacing the outer HWND with the
    // child here breaks any IPC that needs to act on the actual
    // top-level (SetWindowPos drags the child within the parent,
    // PostMessage(WM_CLOSE) destroys the child only, etc.).
    if state.window_hwnds.lock().contains_key(label) {
        tracing::debug!(
            "[opacity] capture_hwnd_for_label: label={} already registered, preserving",
            label
        );
        return;
    }
    // Fast path.
    if let Some(mut browser) = state.get_browser(label) {
        if let Some(host) = browser.host() {
            let hwnd = host.window_handle();
            if !hwnd.0.is_null() {
                // `host.window_handle()` can be the CEF inner WS_CHILD (the
                // main window's Views child, or any `set_as_child` browser),
                // but the cache MUST hold the OUTER top-level frame. The cache-
                // first `resolve_window_hwnd` returns cache hits verbatim — it
                // deliberately does NOT walk GA_ROOT on a hit (that would be
                // wrong for owned floaters, whose GA_ROOT is their owner). So
                // if we stash a child here, `set_window_position` drags the
                // child within its parent (the main-window title-bar drag looks
                // dead) and the redock Z-order walk in `resolve_window_at_cursor`
                // — which matches real top-level frames against this map — never
                // finds the main frame (no redock ghost / no drop target).
                // Walk to GA_ROOT once at capture so the "cache holds the outer
                // frame" invariant matches what floater-create pre-registers.
                let raw = hwnd.0 as isize;
                #[cfg(target_os = "windows")]
                let raw = unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
                    let root = GetAncestor(hwnd.0 as *mut std::ffi::c_void, GA_ROOT);
                    if root.is_null() { raw } else { root as isize }
                };
                state.window_hwnds.lock().insert(label.to_string(), raw);
                tracing::debug!("[opacity] captured hwnd fast-path label={} hwnd={:#x} (outer via GA_ROOT)", label, raw);
                return;
            }
        }
    }
    // Fallback: pick the first eligible visible HWND not already mapped.
    //
    // For on-screen labels (`main`, tear-off `window-*`, promoted pool
    // windows) we MUST skip windows still parked at the warm-pool's
    // off-screen position: those are unpromoted pool members that are
    // `IsWindowVisible` but invisible to the user. Grabbing one binds the
    // label to a hidden window, so drag/close act on the wrong window
    // (root cause of the "window won't drag" regression). When capturing a
    // pool label itself the window legitimately IS off-screen, so the skip
    // is disabled for `window-pool-*` labels.
    let capturing_pool_label = label.starts_with("window-pool-");
    let known: std::collections::HashSet<isize> = state.window_hwnds.lock().values().cloned().collect();
    for hwnd_raw in find_all_own_windows() {
        let raw = hwnd_raw as isize;
        if known.contains(&raw) {
            continue;
        }
        if !capturing_pool_label && unsafe { is_offscreen_pool_window(hwnd_raw) } {
            tracing::debug!(
                "[opacity] capture_hwnd_for_label: skipping off-screen pool window {:#x} for label={}",
                raw, label
            );
            continue;
        }
        state.window_hwnds.lock().insert(label.to_string(), raw);
        tracing::debug!("[opacity] captured hwnd fallback label={} hwnd={:#x}", label, raw);
        return;
    }
    tracing::warn!("[opacity] capture_hwnd_for_label: no available HWND for label={}", label);
}
