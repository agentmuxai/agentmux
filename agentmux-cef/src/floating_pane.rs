// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32-native owned-window creator for floating panes. Phase 1 of
//! issue #810 (floating-pane tear-off).
//!
//! This module is intentionally Windows-only and intentionally
//! separate from `crate::ui_tasks` (which posts the standard CEF Views
//! top-level windows). A floating pane is a *raw* `WS_POPUP` HWND with
//! `WS_EX_TOOLWINDOW`, owner = source main window. CEF Views does not
//! expose tool-window / owner semantics in a way that lets us achieve
//! the no-taskbar + minimize-cascade behavior the spec requires, so we
//! drop down to `CreateWindowExW` and then embed a CEF browser inside
//! the resulting HWND via `WindowInfo::set_as_child` — the same
//! mechanism the browser-pane creation path uses
//! (`browser_pane/creation.rs`).
//!
//! See issue #810 / `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`
//! for the full design and phase plan.
//!
//! ## Scope of Phase 1
//!
//! - Register the IPC command (`open_floating_pane_window`).
//! - Allocate a stable window label.
//! - Create the owned `WS_POPUP | WS_EX_TOOLWINDOW` HWND.
//! - Embed a CEF browser inside it via `WindowInfo::set_as_child`.
//! - Browser loads `<frontend>?floatingPaneId=<id>&windowLabel=<lbl>`.
//!
//! ## Out of scope for Phase 1 (per spec §9)
//!
//! - Drag-to-tear-off wiring (Phase 3).
//! - Floating-pane frontend shell that renders the full `<Block>`
//!   (Phase 2). Phase 1's stub shell renders only a placeholder so the
//!   primitive can be validated end-to-end.
//! - Re-dock (Phase 4).
//! - Persistence (Phase 5).
//! - macOS / Linux ports (deferred).

#![cfg(target_os = "windows")]

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

wrap_task! {
    pub struct CreateFloatingWindowTask {
        state: Arc<AppState>,
        pane_id: String,
        window_label: String,
        url: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            // Runs on the CEF UI thread.
            //
            //   1. Find the source main window's HWND (we own this).
            //   2. CreateWindowExW with WS_EX_TOOLWINDOW + WS_POPUP +
            //      owner HWND. That's what gives us no taskbar / no
            //      Alt-Tab and the minimize / restore / destroy cascade.
            //   3. Embed a CEF browser inside via `set_as_child` —
            //      same pattern as `browser_pane/creation.rs:109`.

            // TODO(phase-6, codex P2 on #811): With multiple main
            // windows in the same process (future tab-tear-off-to-same-
            // process scenarios, sub-windows, etc.), `find_own_top_level_window`
            // returns the FIRST visible window of this process, which
            // may not be the source of the tear-off. The right fix is
            // an API change to thread the source window's label/HWND
            // through `OpenFloatingPaneArgs`. Today (Phase 1) there's
            // exactly one main window per process, so this is harmless.
            // Every early-return from execute() AFTER the host's
            // `post_create_floating_window` enqueued a
            // `PendingWindowCreation` must dispatch
            // `DequeuePendingWindowCreation` — `on_after_created`
            // only fires on success. The `floating-` exclusion in
            // `orphan_reconcile.rs` is belt-and-suspenders; this is
            // the actual cleanup. Codex/reagent P1 round 2 on #811.
            let dequeue = || {
                self.state.host_dispatch(
                    crate::reducer::HostCommand::DequeuePendingWindowCreation,
                );
            };

            let owner_hwnd_raw = unsafe { crate::commands::window::find_own_top_level_window() };
            if owner_hwnd_raw.is_null() {
                tracing::error!(
                    pane_id = %self.pane_id,
                    label = %self.window_label,
                    "[floating-pane] cannot find source main HWND — aborting",
                );
                dequeue();
                return;
            }

            let outer_hwnd = match create_owned_popup(
                owner_hwnd_raw,
                &self.window_label,
                self.x,
                self.y,
                self.width,
                self.height,
            ) {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!(
                        pane_id = %self.pane_id,
                        label = %self.window_label,
                        error = %e,
                        "[floating-pane] CreateWindowExW failed",
                    );
                    dequeue();
                    return;
                }
            };

            tracing::info!(
                pane_id = %self.pane_id,
                label = %self.window_label,
                hwnd = ?outer_hwnd,
                "[floating-pane] outer HWND created",
            );

            // CEF embed — the browser is a WS_CHILD of the outer HWND.
            let rect = Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            };

            let handler = crate::client::AgentMuxHandler::new_with_browser_pane(
                self.state.clone(),
                0,
                true,
            );
            let mut client = Some(crate::client::AgentMuxClient::new(handler, true));

            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            let parent_hwnd = sys::HWND(outer_hwnd as *mut _);
            let mut window_info = WindowInfo::default().set_as_child(parent_hwnd, &rect);
            window_info.runtime_style = RuntimeStyle::ALLOY;

            let result = browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None, // extra_info
                None, // request_context
            );

            if result == 0 {
                tracing::error!(
                    pane_id = %self.pane_id,
                    label = %self.window_label,
                    "[floating-pane] browser_host_create_browser returned 0",
                );
                // Cleanup-on-failure (codex P1 on #811). The outer
                // HWND was already created + shown via
                // `SW_SHOWNOACTIVATE` inside `create_owned_popup`; if
                // we return here without `DestroyWindow` it sits on
                // screen as a phantom empty tool window. Also dequeue
                // the pending-creation entry that
                // `post_create_floating_window` enqueued — without
                // this, `on_after_created` (which fires only on
                // success) never dequeues, and the leaked entry
                // permanently blocks orphan reconciliation despite
                // the `floating-` exclusion in orphan_reconcile.rs
                // (belt-and-suspenders).
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
                    DestroyWindow(outer_hwnd as *mut std::ffi::c_void);
                }
                dequeue();
                return;
            }

            tracing::info!(
                pane_id = %self.pane_id,
                label = %self.window_label,
                "[floating-pane] CEF browser embedded in floating HWND",
            );
        }
    }
}

/// Posts the create-floating-window task to the CEF UI thread. Returns
/// immediately. Mirrors the shape of `ui_tasks::post_create_window` but
/// goes through this module so the path is grep-able.
pub fn post_create_floating_window(
    state: &Arc<AppState>,
    args: &crate::commands::floating_pane::OpenFloatingPaneArgs,
    window_label: &str,
) {
    // Compose the URL the floating window's CEF browser will load. The
    // frontend's cef-init detects `floatingPaneId` and routes to the
    // floating-pane shell instead of the main workspace.
    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let base_url = crate::commands::window::resolve_frontend_base_url(ipc_port);
    let separator = if base_url.contains('?') { "&" } else { "?" };
    // pane_id is a UUID-ish identifier in current callers — no
    // percent-encoding needed today. Use a minimal escape that handles
    // a few special chars in case future callers pass arbitrary names.
    let url = format!(
        "{}{}ipc_port={}&ipc_token={}&windowLabel={}&floatingPaneId={}",
        base_url,
        separator,
        ipc_port,
        ipc_token,
        window_label,
        escape_query_value(&args.pane_id),
    );

    // Phase B.5 pre-create handoff — same shape as the main
    // open-window path so the existing window_meta plumbing (label →
    // kind → parent) sees the floater as a recognized creation.
    // Phase 6 will introduce a dedicated `WindowKind::FloatingPane`
    // to skip the taskbar / report-open logic in `on_after_created`;
    // Phase 1 reuses `Subwindow` (also hidden from taskbar today) so
    // the existing handler path holds.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: window_label.to_string(),
                kind: crate::state::WindowKind::Subwindow,
                parent_instance_id: None,
            },
        },
    );

    let mut task = CreateFloatingWindowTask::new(
        state.clone(),
        args.pane_id.clone(),
        window_label.to_string(),
        url,
        args.x,
        args.y,
        args.width,
        args.height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

/// Minimal query-string escaping for the pane id. Encodes the small
/// set of characters that would break query-string parsing. Avoids
/// pulling in a `url`/`urlencoding` dependency for a single-call site.
fn escape_query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                // Encode as %XX for each UTF-8 byte.
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// CreateWindowExW wrapper that produces the owned `WS_POPUP +
/// WS_EX_TOOLWINDOW` HWND used as the floating-pane outer shell.
///
/// The class is registered once per process; subsequent calls reuse
/// the registered atom.
fn create_owned_popup(
    owner_hwnd_raw: *mut std::ffi::c_void,
    window_label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<*mut std::ffi::c_void, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, RegisterClassExW, ShowWindow, CS_HREDRAW, CS_VREDRAW,
        SW_SHOWNOACTIVATE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW, WS_POPUP,
    };

    // ---- Register the class once per process ----
    static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();
    static CLASS_NAME: &str = "AgentMuxFloatingPane";

    let mut class_name_utf16: Vec<u16> = OsStr::new(CLASS_NAME).encode_wide().collect();
    class_name_utf16.push(0);

    // TODO(phase-6, codex P1 on #811 — explicitly deferred): The
    // documented CEF embedding pattern is for the host's wndproc to
    // intercept WM_CLOSE and route through `CloseBrowser(false)` so
    // DoClose fires before destroy. Phase 1 uses DefWindowProcW —
    // the OS X-button cascade still works end-to-end:
    //
    //   1. User clicks X → DefWindowProcW(WM_CLOSE) → DestroyWindow.
    //   2. Outer HWND's WM_DESTROY cascades into the CEF child HWND
    //      (WS_CHILD of outer).
    //   3. CEF's wndproc on the child runs its destroy handler →
    //      OnBeforeClose fires on AgentMuxHandler → reducer
    //      UnregisterBrowser cleans `state.browsers` + `window_meta`.
    //
    // What's *skipped*: the DoClose hook's chance to cancel close
    // (e.g. for a "Are you sure?" prompt). Floating panes have no
    // such prompt, so this is harmless for Phase 1. The full custom
    // wndproc is Phase 6 polish per spec §9. Files this needs to
    // touch: replace `lpfnWndProc: Some(DefWindowProcW)` with a
    // custom proc; add a `OnceLock<Arc<AppState>>` accessor (mirror
    // `wrr::win_event::app_state`); in WM_CLOSE iterate
    // `state.list_browsers()` and call `host.close_browser(false)`
    // on any whose `window_handle()`'s GA_ROOT ancestor matches our
    // outer HWND.
    CLASS_REGISTERED.call_once(|| unsafe {
        let h_instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name_utf16.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        let atom = RegisterClassExW(&wnd_class);
        if atom == 0 {
            tracing::error!(
                "[floating-pane] RegisterClassExW failed for {CLASS_NAME}; CreateWindowExW will fail",
            );
        }
    });

    let mut title_utf16: Vec<u16> = OsStr::new(&format!("AgentMux — {window_label}"))
        .encode_wide()
        .collect();
    title_utf16.push(0);

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name_utf16.as_ptr(),
            title_utf16.as_ptr(),
            // WS_POPUP for free positioning (NOT WS_CHILD — children
            // are clipped to parent's client area). WS_OVERLAPPEDWINDOW
            // for the resizable border + sysmenu. Phase 6 will
            // customize the title bar via WM_NCHITTEST; Phase 1 ships
            // with the default chrome so drag works out of the box.
            WS_POPUP | WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            owner_hwnd_raw as HWND,
            std::ptr::null_mut(),
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        return Err(format!("CreateWindowExW returned NULL (GetLastError={err})"));
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    Ok(hwnd as *mut std::ffi::c_void)
}

#[cfg(test)]
mod tests {
    use super::escape_query_value;

    #[test]
    fn escape_passes_through_safe_chars() {
        assert_eq!(escape_query_value("abc-XYZ_123.~"), "abc-XYZ_123.~");
    }

    #[test]
    fn escape_encodes_special_chars() {
        assert_eq!(escape_query_value("a b"), "a%20b");
        assert_eq!(escape_query_value("a&b=c"), "a%26b%3Dc");
        assert_eq!(escape_query_value("a/b"), "a%2Fb");
    }

    #[test]
    fn escape_encodes_multibyte_utf8() {
        // U+00E9 'é' is 0xC3 0xA9 in UTF-8.
        assert_eq!(escape_query_value("é"), "%C3%A9");
    }
}
