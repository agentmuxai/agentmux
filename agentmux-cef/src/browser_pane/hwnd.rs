// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32 HWND-level helpers for browser panes: the `WM_SETFOCUS` redirect
//! subclass and the focus-bypass flag.
//!
//! Moved out of `client.rs` during Phase 2 of the pane modularization split
//! (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). `client.rs`
//! still uses `ALLOW_PANE_FOCUS_ONCE` at a distance (nothing there imports
//! the function directly), but `install_browser_pane_focus_redirect` is the home
//! for pane-focused Win32 subclass logic and future phases can wire it up
//! to pane `on_after_created` / `on_load_end` without touching `client.rs`.
//!
//! Everything in this file is Windows-only by gating.

#![cfg(target_os = "windows")]

use std::sync::{Arc, Weak};

/// Map of pane HWND -> original WndProc, so the subclass hook can delegate
/// to the real handler after running its interception logic. The mutex is
/// held only while mutating the map — hooks that read on the UI thread
/// copy out the pointer quickly.
static PANE_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Per-pane context, keyed by the pane's outer HWND. Populated by
/// `install_browser_pane_focus_redirect`. The WndProc hook uses it to emit the
/// `browser-pane-clicked` event on `WM_LBUTTONDOWN` without needing to
/// round-trip through CEF callbacks — only the outer HWND is keyed here;
/// descendants walk up via `GetParent` to find their context.
#[derive(Clone)]
struct PaneContext {
    state: Weak<crate::state::AppState>,
    block_id: String,
}

static PANE_HWND_CONTEXT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, PaneContext>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Remove every `PANE_HWND_CONTEXT` entry whose context refers to the given
/// `block_id`. Called from `on_before_close_pane` so the map doesn't grow
/// unbounded as panes are opened and closed over the session. Keyed by
/// block_id (not HWND) because the close path has the label/block_id
/// immediately but not the HWND — by the time CEF fires on_before_close,
/// the browser's HWND may already be invalid.
pub fn remove_contexts_for_block(block_id: &str) {
    if let Ok(mut map) = PANE_HWND_CONTEXT.lock() {
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

/// When `true`, the next `WM_SETFOCUS` delivered to a subclassed pane HWND
/// is allowed through instead of being redirected back to the parent.
///
/// The frontend's `giveFocus()` -> `browser_pane_focus` IPC sets this flag
/// before calling `SetFocus` on the pane, so user-initiated focus works
/// even though Chromium's internal focus-steal on navigation is blocked.
pub static ALLOW_PANE_FOCUS_ONCE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Last-redirect timestamp per root HWND, used by
/// `should_redirect_pane_focus_to_root` to rate-limit programmatic focus
/// storms (setInterval-driven `window.focus()`, OAuth redirector pages,
/// DOM mutation observers re-focusing on every change). Keyed by the root
/// HWND cast to `usize`. Entries are overwritten on each pass and never
/// explicitly removed — the map is bounded by the count of distinct
/// top-level AgentMux windows seen in a session, which is small.
static PANE_REDIRECT_LAST_AT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, std::time::Instant>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Returns `true` iff the pane WM_SETFOCUS subclass should redirect to
/// `root` via `SetFocus(root)`. Two guards, both motivated by the
/// 2026-05-02 multi-window freeze investigation
/// (`docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md`):
///
/// 1. **Cross-window refusal.** If a *different* top-level HWND currently
///    owns OS foreground (per `GetForegroundWindow()`), refuse to redirect.
///    Same-thread `SetFocus` on a top-level HWND triggers `WM_ACTIVATE`,
///    so redirecting here would steal foreground from the AgentMux window
///    the user is interacting with. With two windows whose pane content
///    both call `window.focus()` programmatically, the redirect itself
///    drives a foreground ping-pong and the host UI thread saturates.
///
/// 2. **Per-root rate limit.** Even within the user's active window, cap
///    redirects at once per 100 ms per root. Tight focus storms from page
///    content can otherwise pile WM_SETFOCUS / WM_ACTIVATE chains onto
///    the UI thread faster than they drain.
///
/// When this returns `false`, the pane WM_SETFOCUS handler still consumes
/// the message (returns 0) — the pane simply doesn't get focus and the
/// previous focus owner is undisturbed.
unsafe fn should_redirect_pane_focus_to_root(root: *mut std::ffi::c_void) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let current_fg = GetForegroundWindow();
    if !current_fg.is_null() && current_fg != root {
        return false;
    }

    let key = root as usize;
    let now = std::time::Instant::now();
    if let Ok(mut map) = PANE_REDIRECT_LAST_AT.lock() {
        if let Some(last) = map.get(&key) {
            if now.duration_since(*last) < std::time::Duration::from_millis(100) {
                return false;
            }
        }
        map.insert(key, now);
    }
    true
}

/// Subclass a browser pane's outer HWND (and every descendant HWND Chromium
/// has already created) so `WM_SETFOCUS` is redirected back to the parent
/// top-level window unless the focus change is user-initiated (see
/// `ALLOW_PANE_FOCUS_ONCE`).
///
/// Without this, Chromium's internal SetFocus on the pane HWND (page load,
/// JS `window.focus()`, etc.) steals the Windows-level keyboard focus —
/// subsequent keystrokes go to the pane's renderer instead of the main
/// window, so terminals, URL bars, and other inputs in the main UI stop
/// responding.
///
/// Wired in by `pane::callbacks::on_after_created_pane` at create time and
/// by `pane::callbacks::on_load_end_pane` after every navigation — Chromium
/// recreates the `Chrome_RenderWidgetHostHWND` on every page load, so the
/// subclass has to follow along or it ends up stranded on a destroyed HWND.
pub unsafe fn install_browser_pane_focus_redirect(
    hwnd: *mut std::ffi::c_void,
    state: Arc<crate::state::AppState>,
    block_id: String,
) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GetAncestor, SetWindowLongPtrW, GA_ROOT, GWLP_WNDPROC,
        WM_SETFOCUS, WM_KILLFOCUS, WM_LBUTTONDOWN,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

    // Register context so the WndProc's WM_LBUTTONDOWN handler can emit
    // the click event without going through CEF callbacks (which only
    // fire on Windows-level focus CHANGE — clicks inside an already-
    // focused pane wouldn't produce a CEF focus callback at all).
    if let Ok(mut map) = PANE_HWND_CONTEXT.lock() {
        map.insert(hwnd as usize, PaneContext {
            state: Arc::downgrade(&state),
            block_id: block_id.clone(),
        });
    }

    /// Walk from `hwnd` up the parent chain looking for a registered pane
    /// context. Child HWNDs (Chrome_WidgetWin_1, Chrome_RenderWidgetHostHWND)
    /// aren't themselves in the map — the outer pane HWND is. Safety bound
    /// of 8 jumps is plenty; Chromium's pane hierarchy is only 2-3 deep.
    unsafe fn find_context(mut hwnd: *mut std::ffi::c_void) -> Option<PaneContext> {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
        for _ in 0..8 {
            if let Ok(map) = PANE_HWND_CONTEXT.lock() {
                if let Some(ctx) = map.get(&(hwnd as usize)) {
                    return Some(ctx.clone());
                }
            }
            let parent = GetParent(hwnd);
            if parent.is_null() || parent == hwnd {
                return None;
            }
            hwnd = parent;
        }
        None
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

        // A click inside the pane HWND is the explicit "user wants to
        // interact with the embedded page" signal. Chromium's own handler
        // will call SetFocus(pane) next — arm ALLOW_PANE_FOCUS_ONCE so
        // the WM_SETFOCUS branch below doesn't redirect it. Without this,
        // clicks on the pane never transfer keyboard focus to Chromium
        // (cursor works, typing goes nowhere — reported by user after
        // the onMouseEnter→onMouseDown switch broke hover-focus-grab but
        // the DOM-level mousedown never fires because the pane HWND
        // intercepts the click at Win32 level, not DOM level).
        if msg == WM_LBUTTONDOWN {
            ALLOW_PANE_FOCUS_ONCE.store(true, Ordering::Relaxed);
            // Emit the click event directly from the WndProc. We can't
            // rely on CEF's FocusHandler::on_set_focus to emit, because
            // CEF only fires that callback when Windows-level focus
            // *changes* — clicks inside a pane that already has keyboard
            // focus (the user clicked another DOM pane, then clicked
            // back into this pane content) produce WM_LBUTTONDOWN but
            // no CEF focus callback, leaving a flag armed forever.
            if let Some(ctx) = find_context(hwnd) {
                if let Some(state) = ctx.state.upgrade() {
                    tracing::info!(
                        block_id = %ctx.block_id,
                        "[pane-wndproc] WM_LBUTTONDOWN — emitting browser-pane-clicked",
                    );
                    crate::events::emit_event_from_state(
                        &state,
                        "browser-pane-clicked",
                        &serde_json::json!({ "block_id": ctx.block_id }),
                    );
                } else {
                    tracing::warn!("[pane-wndproc] WM_LBUTTONDOWN — state dropped, skipping emit");
                }
            } else {
                tracing::warn!("[pane-wndproc] WM_LBUTTONDOWN — no pane context for hwnd {:p}", hwnd);
            }
        }

        if msg == WM_SETFOCUS {
            // Intentional focus from the frontend's giveFocus() IPC: honor it
            // once, then revert to redirect-mode for subsequent events.
            if ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed) {
                tracing::info!("[pane-wndproc] WM_SETFOCUS allowed (intentional)");
                // Fall through to the original WndProc.
            } else {
                // Programmatic focus (page load, JS window.focus()): redirect
                // to the TOP-LEVEL ancestor, not the immediate parent.
                // `GetParent` on a descendant HWND (Chrome_WidgetWin_1,
                // Chrome_RenderWidgetHostHWND, …) returns the pane's outer
                // HWND, which is still inside the pane tree — redirecting
                // there leaves focus stuck in the pane. `GetAncestor(GA_ROOT)`
                // walks all the way up to the top-level window that hosts
                // both main and pane, which is the correct place to land.
                //
                // Guard added 2026-05-02: refuse the redirect when another
                // top-level HWND owns foreground or when this root has been
                // redirected within the last 100 ms. See
                // `should_redirect_pane_focus_to_root` for rationale.
                let root = GetAncestor(hwnd, GA_ROOT);
                if !root.is_null()
                    && root != hwnd
                    && should_redirect_pane_focus_to_root(root)
                {
                    SetFocus(root);
                }
                return 0;
            }
        }

        let original = PANE_WNDPROCS
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        if original != 0 {
            let proc_fn: unsafe extern "system" fn(
                *mut std::ffi::c_void, u32, usize, isize,
            ) -> isize = std::mem::transmute(original);
            CallWindowProcW(Some(proc_fn), hwnd, msg, wparam, lparam)
        } else {
            0
        }
    }

    // Subclass the outer HWND — but only once. Re-calling SetWindowLongPtrW
    // would replace our hook with itself and poison PANE_WNDPROCS.
    let already_hooked = PANE_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if !already_hooked {
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if original != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(hwnd as usize, original);
            }
            tracing::info!("[pane-subclass] installed focus-redirect WndProc on pane HWND {:p}", hwnd);
        }
    }

    // Chromium creates inner HWNDs (widget + render) below the outer HWND.
    // Mouse input reaches the deepest descendant, so we must walk the whole
    // tree and subclass every one.
    unsafe extern "system" fn enum_children(
        child: *mut std::ffi::c_void,
        _lparam: isize,
    ) -> i32 {
        let already = PANE_WNDPROCS
            .lock()
            .ok()
            .map(|m| m.contains_key(&(child as usize)))
            .unwrap_or(false);
        if already {
            return 1;
        }
        let orig = SetWindowLongPtrW(child, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if orig != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(child as usize, orig);
            }
            let mut class_buf = [0u16; 64];
            let n = windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                child, class_buf.as_mut_ptr(), class_buf.len() as i32,
            );
            let class_name = String::from_utf16_lossy(&class_buf[..n as usize]);
            tracing::info!("[pane-subclass] subclassed child HWND {:p} class={}", child, class_name);
        }
        1 // continue
    }
    windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        hwnd, Some(enum_children), 0,
    );
}

// ── Tests ───────────────────────────────────────────────────────────────
//
// The Win32 calls themselves can't be unit-tested without a real HWND and
// window message loop. What we can test here is the focus-bypass flag's
// behavior as a simple AtomicBool — it's the only testable invariant the
// `wndproc_hook` relies on.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn allow_pane_focus_once_starts_false() {
        // Note: this static is global to the process, so other tests can
        // have modified it. Read-only assertion before mutation.
        let _ = ALLOW_PANE_FOCUS_ONCE.load(Ordering::Relaxed);
    }

    #[test]
    fn allow_pane_focus_once_swap_returns_prev_and_clears() {
        ALLOW_PANE_FOCUS_ONCE.store(true, Ordering::Relaxed);
        let prev = ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed);
        assert!(prev, "swap should return the prior true value");
        assert!(!ALLOW_PANE_FOCUS_ONCE.load(Ordering::Relaxed),
            "after swap(false), flag must be cleared");
    }

    #[test]
    fn allow_pane_focus_once_swap_when_false_returns_false() {
        ALLOW_PANE_FOCUS_ONCE.store(false, Ordering::Relaxed);
        let prev = ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed);
        assert!(!prev, "swap on cleared flag should return false");
    }
}
