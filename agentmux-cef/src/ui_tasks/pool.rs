// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Pool-window promote tasks (drag-to-tear-off / new-window / pane promote) and
// the mother-window resize tasks after a pane tear-off. Split out of
// `ui_tasks.rs` unchanged.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;
#[cfg(not(target_os = "windows"))]
use super::get_window_on_ui;

/// Windows-only: drive the CEF Views `Window` set_bounds() + show() for a
/// promoted pool window — the same path the macOS/Linux promote uses
/// (`PromotePoolWindowTask`). The Windows promote positions the raw HWND via
/// Win32 and never touched the Views `Window`, so the browser's view-hierarchy /
/// compositor visibility never flipped from hidden -> the promoted window painted
/// BLANK despite a valid DOM. This is the macOS-vs-Windows asymmetry. Bounds are
/// DIP (CEF Views space); the Win32 caller converts physical -> DIP via
/// `app::dpi_scale_at`. Must run on the UI thread.
/// See docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md.
// Windows-only: run the macOS-parity CEF Views show() on the UI thread. The
// Windows promote runs on the IPC thread, but CEF Views calls are UI-thread-only,
// so the set_bounds()+show() must be posted here (mirroring the macOS
// PromotePoolWindowTask). The Window was cached at on_window_created because
// browser_view.window() returns None for pool windows post-load on Windows.
#[cfg(target_os = "windows")]
wrap_task! {
    pub struct PromotePoolWindowViewsShowTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            use cef::ImplWindow;
            match crate::commands::window_pool::take_pool_window_view(&self.label) {
                Some(window) => {
                    // set_bounds is DIP (the Win32 promote already positioned the
                    // HWND in physical px; this syncs the Views Window so show()
                    // doesn't jump and performs the real hidden->visible transition).
                    window.set_bounds(Some(&cef::Rect {
                        x: self.x,
                        y: self.y,
                        width: self.width,
                        height: self.height,
                    }));
                    window.show();
                    // Belt-and-suspenders compositor nudge. NOTE: CefBrowserHost
                    // ::WasResized is only load-bearing in windowless/OSR mode; in
                    // CEF windowed mode (our case) it is effectively a no-op. The
                    // ACTUAL fix is the CEF Views window.show() above (the genuine
                    // hidden->visible transition). Kept as a cheap hint in case the
                    // host ever runs OSR; do not rely on it. (plan doc §6.)
                    if let Some(host) =
                        self.state.get_browser(&self.label).and_then(|b| b.host())
                    {
                        host.was_resized();
                    }
                    tracing::info!(
                        target: "pool:new-window",
                        label = %self.label,
                        x = self.x, y = self.y, width = self.width, height = self.height,
                        "[pool] CEF Views set_bounds + show on cached Window (macOS-parity, UI thread)"
                    );
                }
                None => {
                    tracing::warn!(
                        target: "pool:new-window",
                        label = %self.label,
                        "[pool] no cached CEF Views Window at promote show task — fix not applied"
                    );
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn post_promote_pool_window_views_show(
    state: &Arc<AppState>,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePoolWindowViewsShowTask::new(
        state.clone(),
        label.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Pool window promote (macOS / Linux) — Phase 7 ─────────────────────────
//
// Moves a pre-warmed pool window from its off-screen holding position
// (-32000, -32000) to the tear-off destination and emits `pool:promote` so
// the renderer attaches the new workspace. Windows uses its own promote path
// (promote_pool_window cfg(windows)) with Win32 HWND + SetWindowPos + taskbar
// show. Non-Windows uses CEF Views Window::set_bounds() which is the
// cross-platform equivalent and runs correctly on the UI thread on macOS and
// Linux.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePoolWindowTask {
        state: Arc<AppState>,
        label: String,
        workspace_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "dnd:tearoff:pool",
                    label = %self.label,
                    "[pool:promote] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            // Pool windows were kept hidden (on_load_end skips show() for
            // window-pool-* labels to avoid focus steal on macOS/Linux). Set
            // the target bounds first so the window appears at the correct
            // position, then show(). The user just performed a drag-to-tear-off
            // so activation is expected and desired here.
            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pool:promote] window repositioned + shown via set_bounds + show"
            );

            // Signal the renderer to attach the workspace. The frontend's
            // awaitPoolPromote() listener was installed at pool-spawn time;
            // mark_pool_window_renderer_ready gates queue insertion on it, so
            // the listener is guaranteed to be ready before this event fires.
            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:promote",
                &serde_json::json!({ "workspaceId": self.workspace_id }),
            );

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                workspace_id = %self.workspace_id,
                "[pool:promote] pool:promote event emitted — renderer will attach workspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pool_window(
    state: &Arc<AppState>,
    label: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePoolWindowTask::new(
        state.clone(),
        label.to_string(),
        workspace_id.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Promote pool window for new-window (Cmd+N / File → New Window) ────────
//
// Identical mechanical flow to PromotePoolWindowTask (set_bounds + show) but
// emits `pool:new-window` instead of `pool:promote`, carrying no workspaceId.
// The frontend's awaitPoolPromote handles both events; on `pool:new-window` it
// omits workspaceId from the URL so initHostNewWindow creates a fresh workspace
// rather than reattaching an existing one.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePoolWindowForNewWindowTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        initial_view: Option<String>,
        initial_meta: Option<String>,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "pool:new-window",
                    label = %self.label,
                    "[pool:new-window] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "pool:new-window",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pool:new-window] window repositioned + shown via set_bounds + show"
            );

            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:new-window",
                &serde_json::json!({
                    "initialView": self.initial_view,
                    "initialMeta": self.initial_meta,
                }),
            );

            tracing::info!(
                target: "pool:new-window",
                label = %self.label,
                "[pool:new-window] pool:new-window emitted — renderer will create fresh workspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pool_window_for_new_window(
    state: &Arc<AppState>,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    initial_view: Option<String>,
    initial_meta: Option<String>,
) {
    let mut task = PromotePoolWindowForNewWindowTask::new(
        state.clone(),
        label.to_string(),
        x,
        y,
        width,
        height,
        initial_view,
        initial_meta,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Promote pane pool window (macOS / Linux) ──────────────────────────────
//
// Repositions a floating-pool-{uuid} frameless window from its off-screen
// holding position to the drop-target bounds and emits pool:pane-promote so
// the renderer mounts FloatingPaneWorkspace with the given paneId+workspaceId.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePanePoolWindowTask {
        state: Arc<AppState>,
        label: String,
        pane_id: String,
        workspace_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "pool:pane",
                    label = %self.label,
                    "[pane-pool] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "pool:pane",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pane-pool] window repositioned + shown"
            );

            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:pane-promote",
                &serde_json::json!({
                    "paneId": self.pane_id,
                    "workspaceId": self.workspace_id,
                }),
            );

            tracing::info!(
                target: "pool:pane",
                label = %self.label,
                pane_id = %self.pane_id,
                workspace_id = %self.workspace_id,
                "[pane-pool] pool:pane-promote emitted — renderer will mount FloatingPaneWorkspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pane_pool_window(
    state: &Arc<AppState>,
    label: &str,
    pane_id: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePanePoolWindowTask::new(
        state.clone(),
        label.to_string(),
        pane_id.to_string(),
        workspace_id.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Mother-window resize after pane tear-off ──────────────────────────────
//
// When a full-height pane is torn off (top-to-bottom column), the mother
// window shrinks by the pane's column width so remaining panes keep their
// absolute pixel sizes.
//
// Spec: docs/specs/SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md

/// Resize the mother window to `new_w_dip` on macOS/Linux via CEF Views
/// `set_bounds`. Width is in CSS/DIP pixels (same coordinate space as the
/// floater args); height is read from the current bounds and preserved.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct ResizeMotherWindowTask {
        state: Arc<AppState>,
        label: String,
        new_w_dip: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    label = %self.label,
                    "[tear-off] ResizeMotherWindowTask: source window not found (already closed?)"
                );
                return;
            };
            let old = window.bounds();
            window.set_bounds(Some(&cef::Rect {
                x: old.x,
                y: old.y,
                width: self.new_w_dip,
                height: old.height,
            }));
            tracing::info!(
                label = %self.label,
                old_w = old.width,
                new_w = self.new_w_dip,
                "[tear-off] mother window resized after pane tear-off"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_resize_mother_window(state: &Arc<AppState>, label: &str, new_w_dip: i32) {
    let mut task = ResizeMotherWindowTask::new(state.clone(), label.to_string(), new_w_dip);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Resize the mother window to `new_w_dip` on Windows via Win32 `SetWindowPos`.
/// `new_w_dip` is in CSS/DIP pixels; this function converts to physical pixels
/// using the source window's monitor DPI before calling `SetWindowPos`.
/// `hwnd` is resolved directly from `source_window_label` in
/// `open_floating_pane_window` (not the cascade-hook fallback `parent_main_hwnd`)
/// so the resize always targets the actual source window.
#[cfg(target_os = "windows")]
wrap_task! {
    pub struct ResizeMotherWindowWin32Task {
        state: Arc<AppState>,
        hwnd: isize,
        new_w_dip: i32,
    }

    impl Task {
        fn execute(&self) {
            use windows_sys::Win32::Foundation::POINT;
            use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
            use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindowRect, SetWindowPos, SWP_NOMOVE, SWP_NOACTIVATE, SWP_NOZORDER,
            };

            unsafe {
                let hwnd = self.hwnd as windows_sys::Win32::Foundation::HWND;
                let mut wr = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                GetWindowRect(hwnd, &mut wr);
                let current_h = wr.bottom - wr.top;
                let pt = POINT { x: wr.left, y: wr.top };
                let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
                let mut dpi_x: u32 = 0;
                let mut dpi_y: u32 = 0;
                let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                let dpi_scale = if hr != 0 || dpi_x == 0 { 1.0f32 } else { dpi_x as f32 / 96.0 };
                let new_w_px = (self.new_w_dip as f32 * dpi_scale).round() as i32;
                let ok = SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0, 0,
                    new_w_px,
                    current_h,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                tracing::info!(
                    hwnd = self.hwnd,
                    new_w_dip = self.new_w_dip,
                    new_w_px,
                    dpi_scale,
                    ok = (ok != 0),
                    "[tear-off] mother window resized after pane tear-off (Win32)"
                );
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub fn post_resize_mother_window_win32(state: &Arc<AppState>, hwnd: isize, new_w_dip: i32) {
    let mut task = ResizeMotherWindowWin32Task::new(state.clone(), hwnd, new_w_dip);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Issue #2977 WS3 — mark a window always-on-top.
///
/// The tray panel is specified as a "small, separate, **always-on-top** CEF
/// window" (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §2). Without
/// it the panel drops behind whatever the user was working in the moment they
/// click back into it, which defeats a companion surface (Codex P2 on PR
/// #3002).
///
/// Uses the CEF Views `Window::set_always_on_top` rather than a Win32
/// `HWND_TOPMOST` call so it works identically on every platform and needs no
/// HWND resolution — and so it applies equally to a pool-promoted window
/// (HWND already exists) and a cold-path one (HWND does not exist until CEF
/// creates it). Posted, because Views calls are UI-thread-only.
pub fn post_set_always_on_top(state: &Arc<AppState>, label: &str) {
    let mut task = SetAlwaysOnTopTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    pub struct SetAlwaysOnTopTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Resolve on the UI thread: for the cold path the window may not
            // have existed when this task was posted. A miss is logged, not
            // fatal — a panel that is merely not-topmost is still usable,
            // whereas panicking here would take down the UI thread.
            match super::get_window_on_ui(&self.state, &self.label) {
                Some(window) => {
                    window.set_always_on_top(1);
                    tracing::info!(
                        target: "tray:panel",
                        label = %self.label,
                        "[panel] set always-on-top"
                    );
                }
                None => tracing::warn!(
                    target: "tray:panel",
                    label = %self.label,
                    "[panel] window not resolvable yet — not set always-on-top"
                ),
            }
        }
    }
}
