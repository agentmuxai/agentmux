// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ctrl+Wheel recovery for floating panes (Windows).
//!
//! # Why this exists
//!
//! A floating pane hosts its browser as a **child HWND**
//! (`floating_pane.rs`, `WindowInfo::set_as_child`), unlike the main window,
//! which is CEF **Views**-hosted (`app/mod.rs`). Child-HWND CEF browsers do not
//! dispatch Ctrl+Wheel to the renderer at all — CEF consumes it for its own
//! native, `HostZoomMap`-shared page zoom before the DOM ever sees it.
//!
//! This was measured rather than assumed
//! (`docs/analysis/ANALYSIS_FLOATER_CTRL_SCROLL_ZOOM_2026_08_31.md` §7), with a
//! `wheel` recorder installed in each window and driven by a real mouse:
//!
//! | window                  | input        | events reaching the DOM |
//! |-------------------------|--------------|-------------------------|
//! | main, docked terminal   | Ctrl+Scroll  | 22 / 22, `ctrl: true`   |
//! | floater, same terminal  | Ctrl+Scroll  | **0**                   |
//! | floater, same terminal  | plain scroll | 49, `ctrl: false`       |
//!
//! So the floater receives wheel input normally and **only** Ctrl+Wheel is
//! suppressed. Every in-page mechanism was proven working end to end (§6): the
//! per-view handlers, the `term:zoom` RPC, the read-back, and the font
//! application. The single missing link is the DOM event.
//!
//! # What it does
//!
//! Rather than reimplement zoom per view type — `term`, `armory`, `editor`,
//! `swarm` and `warden` each own a capture-phase Ctrl+Wheel handler — this hook
//! **restores the event Chromium swallowed** and lets those existing handlers
//! run unchanged. `MK_CONTROL` in `wParam` is set by the system from global key
//! state and is delivered to the WndProc regardless of what CEF later does with
//! the message, so it is a reliable trigger.
//!
//! The precedent is `browser_pane/hwnd.rs:403`, which needed the same
//! interception for the same reason: browser panes are *also* child-HWND
//! browsers. They got a workaround; floaters never did.
//!
//! # KNOWN LIMITATION — Windows only
//!
//! This whole file is `cfg(windows)`. The browser-pane equivalent carries the
//! same gap (`browser_pane/hwnd.rs:395`), and no macOS/Linux counterpart exists
//! for either. On those platforms Ctrl+Wheel over a floater still falls through
//! to CEF's native shared zoom. Stated rather than hidden.

#![cfg(target_os = "windows")]

use std::sync::{Arc, Weak};

/// Map of subclassed HWND -> its original WndProc, so the hook can delegate
/// everything it does not handle. Covers the browser's outer child HWND and
/// every Chromium descendant, because mouse input is delivered to the deepest
/// descendant under the cursor, not to the ancestor.
static FLOATER_WHEEL_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Per-floater context, keyed by the HWND the hook was installed on. Only the
/// outermost installed HWND is keyed; descendants walk up via `GetParent` to
/// find it, mirroring `browser_pane::hwnd`'s `find_context`.
#[derive(Clone)]
struct FloaterWheelContext {
    state: Weak<crate::state::AppState>,
    /// The floater's window label (`floating-<uuid>`), used to route the event
    /// to *that* window's renderer. `emit_event_from_state`'s "main"/
    /// first-available fallback would deliver it to the wrong window — the same
    /// defect reagentx caught for `browser-pane-clicked` on PR #2597.
    label: String,
}

static FLOATER_WHEEL_CONTEXT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, FloaterWheelContext>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const WM_MOUSEWHEEL: u32 = 0x020A;
/// `MK_CONTROL`, low word of `wParam`.
const MK_CONTROL: usize = 0x0008;

/// Walk up the parent chain looking for a registered floater context. The
/// bound of 8 matches `browser_pane::hwnd::find_context`; Chromium's hierarchy
/// under the browser is only 2-3 deep.
unsafe fn find_context(mut hwnd: *mut std::ffi::c_void) -> Option<FloaterWheelContext> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
    for _ in 0..8 {
        if let Ok(map) = FLOATER_WHEEL_CONTEXT.lock() {
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
    use windows_sys::Win32::UI::WindowsAndMessaging::CallWindowProcW;

    if msg == WM_MOUSEWHEEL && (wparam & MK_CONTROL) != 0 {
        // High word of wParam, signed: positive = wheel forward/away from the
        // user (zoom in), negative = toward the user (zoom out). Passed through
        // as a DOM-style `deltaY`, whose sign convention is the OPPOSITE
        // (positive deltaY = scroll down = zoom out), so it is negated here.
        // The per-view handlers all branch on `ev.deltaY > 0`.
        let raw_delta = (wparam >> 16) as u16 as i16;
        let delta_y = -(raw_delta as f64);

        if let Some(ctx) = find_context(hwnd) {
            if let Some(state) = ctx.state.upgrade() {
                crate::events::emit_event_to_window(
                    &state,
                    &ctx.label,
                    "floater:ctrl-wheel",
                    &serde_json::json!({ "deltaY": delta_y }),
                );
                // Consume: do NOT call the original WndProc. Letting it run is
                // what produces CEF's native shared page zoom.
                return 0;
            }
        }

        // No context, or state dropped. Deliberately fall through to the
        // original WndProc rather than returning 0 here.
        //
        // `browser_pane/hwnd.rs:403` does the opposite — its `return 0` sits
        // outside the context lookup, so an unresolvable HWND silently eats the
        // input and does nothing at all. That is a latent bug there, and
        // repeating it would mean a floater whose context went missing would
        // lose Ctrl+Wheel entirely with no fallback. Falling through at least
        // leaves CEF's native zoom, which is degraded but not dead.
        tracing::warn!(
            "[floater-wheel] ctrl+wheel with no floater context for hwnd {:p} — falling through",
            hwnd
        );
    }

    let original = FLOATER_WHEEL_WNDPROCS
        .lock()
        .ok()
        .and_then(|m| m.get(&(hwnd as usize)).copied())
        .unwrap_or(0);
    if original != 0 {
        return CallWindowProcW(Some(std::mem::transmute(original)), hwnd, msg, wparam, lparam);
    }
    windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Subclass `hwnd` and every Chromium descendant so Ctrl+Wheel can be recovered
/// and forwarded to `label`'s renderer.
///
/// **Must be re-invoked after every navigation.** Chromium recreates
/// `Chrome_RenderWidgetHostHWND` on each page load, so a subclass installed once
/// ends up stranded on a destroyed HWND — the same reason
/// `install_browser_pane_focus_redirect` is wired from both
/// `on_after_created_browser_pane` and `on_load_end_browser_pane`. Re-invoking
/// is cheap and idempotent: already-subclassed HWNDs are skipped.
pub unsafe fn install_floater_ctrl_wheel_hook(
    state: &Arc<crate::state::AppState>,
    hwnd: *mut std::ffi::c_void,
    label: &str,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, SetWindowLongPtrW, GWLP_WNDPROC,
    };

    if hwnd.is_null() {
        return;
    }

    if let Ok(mut map) = FLOATER_WHEEL_CONTEXT.lock() {
        map.insert(
            hwnd as usize,
            FloaterWheelContext {
                state: Arc::downgrade(state),
                label: label.to_string(),
            },
        );
    }

    // Subclass the outer HWND once. Re-calling SetWindowLongPtrW would replace
    // our hook with itself and poison the original-WndProc map.
    let already = FLOATER_WHEEL_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if !already {
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if original != 0 {
            if let Ok(mut map) = FLOATER_WHEEL_WNDPROCS.lock() {
                map.insert(hwnd as usize, original);
            }
            tracing::info!(
                "[floater-wheel] installed ctrl+wheel hook on {:p} label={}",
                hwnd,
                label
            );
        }
    }

    unsafe extern "system" fn enum_children(child: *mut std::ffi::c_void, _lp: isize) -> i32 {
        let already = FLOATER_WHEEL_WNDPROCS
            .lock()
            .ok()
            .map(|m| m.contains_key(&(child as usize)))
            .unwrap_or(false);
        if already {
            return 1;
        }
        let orig = SetWindowLongPtrW(child, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if orig != 0 {
            if let Ok(mut map) = FLOATER_WHEEL_WNDPROCS.lock() {
                map.insert(child as usize, orig);
            }
        }
        1 // continue enumeration
    }
    EnumChildWindows(hwnd, Some(enum_children), 0);
}

/// Restore original WndProcs and drop bookkeeping for a closing floater.
///
/// Called from the floater's close path. Skips `SetWindowLongPtrW` on HWNDs the
/// OS has already destroyed (guarded by `IsWindow`) but still clears the map
/// entries, so a later HWND value reused by Windows can't inherit a stale hook.
pub unsafe fn remove_floater_ctrl_wheel_hook(label: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetWindowLongPtrW, GWLP_WNDPROC,
    };

    let roots: Vec<usize> = match FLOATER_WHEEL_CONTEXT.lock() {
        Ok(map) => map
            .iter()
            .filter(|(_, ctx)| ctx.label == label)
            .map(|(h, _)| *h)
            .collect(),
        Err(_) => return,
    };
    if roots.is_empty() {
        return;
    }

    // Un-subclass the root and everything beneath it. Children are collected
    // first (EnumChildWindows can't run while we mutate), then restored.
    for root in &roots {
        let root_ptr = *root as *mut std::ffi::c_void;
        let mut targets: Vec<usize> = vec![*root];
        if IsWindow(root_ptr) != 0 {
            unsafe extern "system" fn collect(child: *mut std::ffi::c_void, lp: isize) -> i32 {
                let out = &mut *(lp as *mut Vec<usize>);
                out.push(child as usize);
                1
            }
            windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
                root_ptr,
                Some(collect),
                &mut targets as *mut Vec<usize> as isize,
            );
        }
        if let Ok(mut map) = FLOATER_WHEEL_WNDPROCS.lock() {
            for h in targets {
                if let Some(orig) = map.remove(&h) {
                    let hp = h as *mut std::ffi::c_void;
                    if IsWindow(hp) != 0 {
                        SetWindowLongPtrW(hp, GWLP_WNDPROC, orig);
                    }
                }
            }
        }
    }

    if let Ok(mut map) = FLOATER_WHEEL_CONTEXT.lock() {
        map.retain(|_, ctx| ctx.label != label);
    }
    tracing::info!("[floater-wheel] removed ctrl+wheel hook for label={}", label);
}

#[cfg(test)]
mod tests {
    //! The Win32 calls need a real HWND and message pump, so what is testable
    //! here is the wheel-delta sign conversion — the one piece of pure logic,
    //! and the piece most likely to be silently inverted.

    /// Mirrors the conversion in `wndproc_hook`.
    fn delta_y_from_wparam(wparam: usize) -> f64 {
        let raw = (wparam >> 16) as u16 as i16;
        -(raw as f64)
    }

    const WHEEL_DELTA: usize = 120;

    #[test]
    fn wheel_forward_becomes_negative_delta_y_zoom_in() {
        // Wheel away from the user: positive WM_MOUSEWHEEL delta. The per-view
        // handlers zoom IN when `ev.deltaY <= 0`, so this must be negative.
        let w = (WHEEL_DELTA << 16) | super::MK_CONTROL;
        assert_eq!(delta_y_from_wparam(w), -120.0);
    }

    #[test]
    fn wheel_toward_user_becomes_positive_delta_y_zoom_out() {
        // Negative i16 in the high word, i.e. -120.
        let raw: i16 = -120;
        let w = ((raw as u16 as usize) << 16) | super::MK_CONTROL;
        assert_eq!(delta_y_from_wparam(w), 120.0);
    }

    #[test]
    fn mk_control_bit_is_read_from_the_low_word() {
        // A wheel delta large enough to occupy the high word must not be
        // mistaken for the modifier bits.
        let with_ctrl = (WHEEL_DELTA << 16) | super::MK_CONTROL;
        let without = WHEEL_DELTA << 16;
        assert_ne!(with_ctrl & super::MK_CONTROL, 0);
        assert_eq!(without & super::MK_CONTROL, 0);
    }
}
