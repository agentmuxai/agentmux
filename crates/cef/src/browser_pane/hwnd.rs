// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32 HWND-level helpers for browser panes: the subclass on every window of
//! a pane, which reports clicks and focus to the input owner
//! (`crate::input_owner`), and the map from a window to its pane.
//!
//! Everything in this file is Windows-only by gating.

#![cfg(target_os = "windows")]

use std::sync::{Arc, Weak};

/// Map of pane HWND -> original WndProc, so the subclass hook can delegate
/// to the real handler after running its interception logic. The mutex is
/// held only while mutating the map — hooks that read on the UI thread
/// copy out the pointer quickly.
static BROWSER_PANE_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Per-pane context, keyed by the pane's outer HWND. Populated by
/// `install_browser_pane_subclass`. The WndProc hook uses it to emit the
/// `browser-pane-clicked` event on `WM_LBUTTONDOWN` without needing to
/// round-trip through CEF callbacks — only the outer HWND is keyed here;
/// descendants walk up via `GetParent` to find their context.
#[derive(Clone)]
struct BrowserPaneContext {
    state: Weak<crate::state::AppState>,
    block_id: String,
}

static BROWSER_PANE_HWND_CONTEXT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, BrowserPaneContext>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Restore the original WndProc for every HWND (outer + children) that was
/// subclassed by `install_browser_pane_subclass` for the given `block_id`.
///
/// Call this from `on_before_close_browser_pane` **before**
/// `remove_contexts_for_block` — we still need `BROWSER_PANE_HWND_CONTEXT` to
/// locate the outer HWND.
///
/// If an HWND has already been destroyed by the time this runs, the
/// `SetWindowLongPtrW` call is skipped (guarded by `IsWindow`) but the entry
/// is still removed from `BROWSER_PANE_WNDPROCS`. This prevents the closed
/// pane's HWND value from being picked
/// as the "main render widget" by `find_main_render_widget` if the value is
/// later recycled by a new `Chrome_RenderWidgetHostHWND`.
pub fn uninstall_pane_subclass_for_block(block_id: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, IsWindow, SetWindowLongPtrW, GWLP_WNDPROC,
    };

    // Collect outer HWND(s) for this block from BROWSER_PANE_HWND_CONTEXT.
    let outer_hwnds: Vec<usize> = BROWSER_PANE_HWND_CONTEXT
        .lock()
        .ok()
        .map(|m| {
            m.iter()
                .filter(|(_, ctx)| ctx.block_id == block_id)
                .map(|(&hwnd, _)| hwnd)
                .collect()
        })
        .unwrap_or_default();

    if outer_hwnds.is_empty() {
        tracing::debug!(block_id = %block_id, "[pane-unsubclass] no outer HWNDs in context for block — already cleaned up?");
        return;
    }

    // Build the full set of HWNDs to restore:
    //   1. The outer HWND(s) themselves (may already be destroyed — that's fine,
    //      map.remove still drains their BROWSER_PANE_WNDPROCS entry).
    //   2. Live children found via EnumChildWindows (outer alive path).
    //   3. Dead children still in BROWSER_PANE_WNDPROCS (outer destroyed path) —
    //      we can't walk the HWND tree for a dead outer, so we drain every entry
    //      from BROWSER_PANE_WNDPROCS whose HWND is no longer a valid window.
    //      Those dead entries can only belong to the closed pane (no other code
    //      leaves dead HWNDs in the map), so removing them is safe and closes the
    //      recycled-child-HWND gap described in §3 of the analysis.
    let mut candidates: Vec<usize> = outer_hwnds.clone();
    for &outer in &outer_hwnds {
        let outer_ptr = outer as *mut std::ffi::c_void;
        if unsafe { IsWindow(outer_ptr) } == 0 {
            // Outer is gone — collect all BROWSER_PANE_WNDPROCS keys that are
            // also dead; they must be children of this (now-destroyed) pane.
            if let Ok(map) = BROWSER_PANE_WNDPROCS.lock() {
                for &hwnd in map.keys() {
                    if unsafe { IsWindow(hwnd as *mut std::ffi::c_void) } == 0 {
                        candidates.push(hwnd);
                    }
                }
            }
            continue;
        }
        unsafe extern "system" fn collect_children(
            child: *mut std::ffi::c_void,
            lparam: isize,
        ) -> i32 {
            let acc = &mut *(lparam as *mut Vec<usize>);
            acc.push(child as usize);
            1
        }
        unsafe {
            EnumChildWindows(
                outer_ptr,
                Some(collect_children),
                &mut candidates as *mut Vec<usize> as isize,
            );
        }
    }

    let mut restored = 0usize;
    if let Ok(mut map) = BROWSER_PANE_WNDPROCS.lock() {
        for hwnd in candidates {
            if let Some(orig) = map.remove(&hwnd) {
                let hwnd_ptr = hwnd as *mut std::ffi::c_void;
                unsafe {
                    if IsWindow(hwnd_ptr) != 0 {
                        SetWindowLongPtrW(hwnd_ptr, GWLP_WNDPROC, orig);
                    }
                }
                restored += 1;
            }
        }
    }

    tracing::info!(
        block_id = %block_id,
        restored = restored,
        "[pane-unsubclass] restored WndProcs for {} HWNDs on pane close",
        restored,
    );
}

/// Return the outer HWND for every pane currently tracked in
/// `BROWSER_PANE_HWND_CONTEXT`, regardless of whether that pane is still
/// registered in `state.browsers`. Used by `MainFocusReclaimTask` as a
/// defence-in-depth supplement to the `state.list_browsers` source so that
/// panes which have been `BrowserUnregistered` but whose HWNDs are still live
/// (deferred CEF teardown window) are still excluded from
/// `find_main_render_widget`.
pub fn pane_outer_hwnds_from_context() -> Vec<*mut std::ffi::c_void> {
    BROWSER_PANE_HWND_CONTEXT
        .lock()
        .ok()
        .map(|m| m.keys().map(|&h| h as *mut std::ffi::c_void).collect())
        .unwrap_or_default()
}

/// Remove every `BROWSER_PANE_HWND_CONTEXT` entry whose context refers to the given
/// `block_id`. Called from `on_before_close_browser_pane` so the map doesn't grow
/// unbounded as panes are opened and closed over the session. Keyed by
/// block_id (not HWND) because the close path has the label/block_id
/// immediately but not the HWND — by the time CEF fires on_before_close,
/// the browser's HWND may already be invalid.
pub fn remove_contexts_for_block(block_id: &str) {
    if let Ok(mut map) = BROWSER_PANE_HWND_CONTEXT.lock() {
        let before = map.len();
        map.retain(|_hwnd, ctx| ctx.block_id != block_id);
        let removed = before - map.len();
        if removed > 0 {
            tracing::info!(
                block_id = %block_id,
                removed = removed,
                remaining = map.len(),
                "[pane-hwnd] cleaned up hwnd context entries",
            );
        }
    }
}

/// Walk from `hwnd` up the parent chain to the pane it belongs to. Chromium's
/// child windows aren't in the map; the pane's outer window is. A bound of 8 is
/// plenty: Chromium's pane hierarchy is 2-3 deep.
fn find_context(mut hwnd: *mut std::ffi::c_void) -> Option<BrowserPaneContext> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
    for _ in 0..8 {
        if let Ok(map) = BROWSER_PANE_HWND_CONTEXT.lock() {
            if let Some(ctx) = map.get(&(hwnd as usize)) {
                return Some(ctx.clone());
            }
        }
        let parent = unsafe { GetParent(hwnd) };
        if parent.is_null() || parent == hwnd {
            return None;
        }
        hwnd = parent;
    }
    None
}

/// The block id of the pane `hwnd` is a window of, if any.
pub fn pane_block_for_hwnd(hwnd: *mut std::ffi::c_void) -> Option<String> {
    find_context(hwnd).map(|ctx| ctx.block_id)
}

/// Subclass a browser pane's outer HWND (and every descendant HWND Chromium
/// has already created) so a click (`WM_LBUTTONDOWN`) and focus arriving
/// (`WM_SETFOCUS`) reach the input owner. The input owner decides whether focus
/// that a page took on its own (page load, JS `window.focus()`) is put back;
/// the message itself always reaches Chromium, so Chromium never believes it is
/// blurred while it holds Win32 focus.
///
/// Wired in by `browser_pane::callbacks::on_after_created_browser_pane` at create time and
/// by `browser_pane::callbacks::on_load_end_browser_pane` after every navigation — Chromium
/// recreates the `Chrome_RenderWidgetHostHWND` on every page load, so the
/// subclass has to follow along or it ends up stranded on a destroyed HWND.
pub unsafe fn install_browser_pane_subclass(
    hwnd: *mut std::ffi::c_void,
    state: Arc<crate::state::AppState>,
    block_id: String,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_SETFOCUS,
    };

    // Register context so the WndProc's WM_LBUTTONDOWN handler can emit
    // the click event without going through CEF callbacks (which only
    // fire on Windows-level focus CHANGE — clicks inside an already-
    // focused pane wouldn't produce a CEF focus callback at all).
    if let Ok(mut map) = BROWSER_PANE_HWND_CONTEXT.lock() {
        map.insert(hwnd as usize, BrowserPaneContext {
            state: Arc::downgrade(&state),
            block_id: block_id.clone(),
        });
    }

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        // Diagnostic: surface mouse-wheel and key events so we can tell
        // whether they reach the pane HWND at all when the user reports
        // scrolling/typing breakage.
        const WM_MOUSEWHEEL: u32 = 0x020A;
        const WM_MOUSEHWHEEL: u32 = 0x020E;
        const WM_KEYDOWN: u32 = 0x0100;
        const WM_CHAR: u32 = 0x0102;
        // Ctrl+Wheel over a browser pane's HWND is delivered here, not to any
        // DOM listener — AppZoomHandler (app.tsx) is pure JS/DOM and cannot
        // see it. Left unhandled (the pre-existing behavior: log only, fall
        // through), Windows/Chromium's default handling is CEF's own native
        // page zoom, which is scoped per-host via a RequestContext-shared
        // HostZoomMap and therefore visually affects every browser pane on
        // the same host at once (see the module doc on
        // AppState::browser_pane_zoom). Handled here, we consume the message
        // (return 0, never call the original WndProc) and apply zoom
        // ourselves via CSS injection instead — see
        // BrowserPaneManager::zoom_in/zoom_out.
        //
        // KNOWN LIMITATION: this interception is Windows-only (this whole
        // file is `#![cfg(target_os = "windows")]`-gated, see the top of the
        // file). No macOS/Linux equivalent hook exists yet, so on those
        // platforms Ctrl+Wheel over a browser pane still falls through to
        // CEF's native, HostZoomMap-shared zoom (the original shared-zoom
        // bug this feature fixes). Tracked as a follow-up, not silently
        // dropped — see the PR description / REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md.
        if msg == WM_MOUSEWHEEL {
            let ctrl_held = (wparam & 0x0008) != 0; // MK_CONTROL, low word of wParam
            if ctrl_held {
                // High word of wParam, signed: positive = wheel rotated
                // forward/away from the user (the conventional "zoom in"
                // direction), negative = toward the user ("zoom out").
                let raw_delta = (wparam >> 16) as u16 as i16;
                if let Some(ctx) = find_context(hwnd) {
                    if let Some(state) = ctx.state.upgrade() {
                        if raw_delta > 0 {
                            state.browser_panes.zoom_in(&ctx.block_id, &state);
                        } else if raw_delta < 0 {
                            state.browser_panes.zoom_out(&ctx.block_id, &state);
                        }
                    }
                }
                return 0;
            }
        }

        match msg {
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                tracing::info!("[pane-wndproc] mouse-wheel hwnd={:p} msg=0x{:x}", hwnd, msg);
            }
            WM_KEYDOWN | WM_CHAR => {
                tracing::info!("[pane-wndproc] key msg=0x{:x} wparam={}", msg, wparam);
            }
            WM_KILLFOCUS => {
                tracing::info!("[pane-wndproc] WM_KILLFOCUS hwnd={:p}", hwnd);
            }
            _ => {}
        }

        // A click inside the pane is the user choosing its page: the input
        // owner selects it, so the focus Chromium takes next is the one asked
        // for. The DOM-level mousedown never fires for it: the pane's window
        // takes the click at the Win32 level.
        if msg == WM_LBUTTONDOWN {
            if let Some(ctx) = find_context(hwnd) {
                if let Some(state) = ctx.state.upgrade() {
                    crate::input_owner::windows::pane_pointer_down(&state, &ctx.block_id);
                }
            }
            // Emit the click event directly from the WndProc. We can't
            // rely on CEF's FocusHandler::on_set_focus to emit, because
            // CEF only fires that callback when Windows-level focus
            // *changes* — clicks inside a pane that already has keyboard
            // focus (the user clicked another DOM pane, then clicked
            // back into this pane content) produce WM_LBUTTONDOWN but
            // no CEF focus callback.
            if let Some(ctx) = find_context(hwnd) {
                if let Some(state) = ctx.state.upgrade() {
                    let block_id_short: String = ctx.block_id.chars().take(7).collect();
                    tracing::info!(
                        "[browser-pane:diag][{}] emit-clicked",
                        block_id_short,
                    );
                    // Route to the pane's ACTUAL owning window, not "main" —
                    // a pane torn off into its own floating window has its
                    // own JS context; `emit_event_from_state`'s "main"/
                    // first-available fallback delivered this to the wrong
                    // window (or none) for floating panes, so neither pane
                    // selection nor (later) the outside-click-dismiss bridge
                    // built on this event worked there (reagentx P1 on PR
                    // #2597). Same fix as `browser-pane-shortcut`
                    // (handlers.rs) and `browser-pane-context-menu`
                    // (context_menu.rs).
                    match state.browser_pane_window_label(&ctx.block_id) {
                        Some(window_label) => {
                            crate::events::emit_event_to_window(
                                &state,
                                &window_label,
                                "browser-pane-clicked",
                                &serde_json::json!({ "block_id": ctx.block_id }),
                            );
                        }
                        None => {
                            tracing::warn!(
                                "[pane-wndproc] WM_LBUTTONDOWN — no owning window label for block_id={}",
                                ctx.block_id
                            );
                        }
                    }
                } else {
                    tracing::warn!("[pane-wndproc] WM_LBUTTONDOWN — state dropped, skipping emit");
                }
            } else {
                tracing::warn!("[pane-wndproc] WM_LBUTTONDOWN — no pane context for hwnd {:p}", hwnd);
            }
        }

        let original = BROWSER_PANE_WNDPROCS
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        let result = if original != 0 {
            let proc_fn: unsafe extern "system" fn(
                *mut std::ffi::c_void, u32, usize, isize,
            ) -> isize = std::mem::transmute(original);
            CallWindowProcW(Some(proc_fn), hwnd, msg, wparam, lparam)
        } else {
            0
        };

        // Focus arrived in the pane. Chromium has seen it (above); the input
        // owner decides whether it stays (spec I7: never swallowed).
        if msg == WM_SETFOCUS {
            if let Some(ctx) = find_context(hwnd) {
                if let Some(state) = ctx.state.upgrade() {
                    crate::input_owner::windows::pane_focus_gained(&state, &ctx.block_id);
                }
            }
        }
        result
    }

    // Subclass the outer HWND — but only once. Re-calling SetWindowLongPtrW
    // would replace our hook with itself and poison BROWSER_PANE_WNDPROCS.
    let already_hooked = BROWSER_PANE_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if !already_hooked {
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if original != 0 {
            if let Ok(mut map) = BROWSER_PANE_WNDPROCS.lock() {
                map.insert(hwnd as usize, original);
            }
            tracing::info!("[pane-subclass] installed on pane HWND {:p} block={}", hwnd, block_id);
        }
    }

    // Chromium creates inner HWNDs (widget + render) below the outer HWND.
    // Mouse input reaches the deepest descendant, so we must walk the whole
    // tree and subclass every one.
    unsafe extern "system" fn enum_children(
        child: *mut std::ffi::c_void,
        _lparam: isize,
    ) -> i32 {
        let already = BROWSER_PANE_WNDPROCS
            .lock()
            .ok()
            .map(|m| m.contains_key(&(child as usize)))
            .unwrap_or(false);
        if already {
            return 1;
        }
        let orig = SetWindowLongPtrW(child, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if orig != 0 {
            if let Ok(mut map) = BROWSER_PANE_WNDPROCS.lock() {
                map.insert(child as usize, orig);
            }
            let mut class_buf = [0u16; 64];
            let n = windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                child, class_buf.as_mut_ptr(), class_buf.len() as i32,
            );
            let class_name = String::from_utf16_lossy(&class_buf[..n as usize]);
            let block = pane_block_for_hwnd(child).unwrap_or_default();
            tracing::info!("[pane-subclass] subclassed child HWND {:p} class={} block={}", child, class_name, block);
        }
        1 // continue
    }
    windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        hwnd, Some(enum_children), 0,
    );
}
