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

            // Register the outer HWND in `state.window_hwnds` under the
            // floater's label. Without this, `resolve_window_hwnd(label)`
            // falls through to the reducer-registry path which goes
            // host.window_handle() (returns the CEF inner WS_CHILD) →
            // GetAncestor(GA_ROOT), and in our setup GA_ROOT lands on
            // MAIN's HWND (not the outer floater), so any
            // close_window_by_label("floating-…") would actually post
            // WM_CLOSE to MAIN and cascade-destroy every owned floater.
            //
            // Capturing the known-good outer HWND here ensures the
            // cache lookup (added to resolve_window_hwnd) returns the
            // right window for floater-targeted IPCs.
            self.state
                .window_hwnds
                .lock()
                .insert(self.window_label.clone(), outer_hwnd as isize);

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

            // Edge-resize: subclass the freshly-created CEF child so its
            // WM_NCHITTEST forwards the floater's resize border (HTTRANSPARENT)
            // to the outer wndproc → native WS_THICKFRAME resize. Done HERE,
            // synchronously after create — the floater's GW_CHILD exists now;
            // installing later (on_after_created) misses it. SPEC_FLOATING_PANE_EDGE_RESIZE.
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, GW_CHILD};
                let child = GetWindow(outer_hwnd as *mut std::ffi::c_void, GW_CHILD);
                if !child.is_null() {
                    install_child_nchittest_forwarder(child);
                } else {
                    tracing::warn!(
                        label = %self.window_label,
                        "[floating-pane] no CEF child after create — edge-resize disabled",
                    );
                }
            }
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
    // pane_id is a UUID-ish identifier in current callers — no
    // percent-encoding needed today. Use a minimal escape that handles
    // a few special chars in case future callers pass arbitrary names.
    // workspaceId threads through to the floater's `initApp` →
    // `initHostNewWindow` path which already understands a `?workspaceId=`
    // URL param (the existing tab tear-off uses the same handoff —
    // see frontend/app-init.ts:236). Phase 1 floaters didn't pass it
    // and rendered the placeholder shell; Phase 2 callers (#1077) pass
    // the newly-created workspace id so the floater renders the actual
    // `<Block>` via the standard new-window init.
    let url = match crate::commands::window::resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let mut u = format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}&floatingPaneId={}",
                base_url,
                separator,
                ipc_port,
                ipc_token,
                window_label,
                escape_query_value(&args.pane_id),
            );
            if let Some(ws_id) = args.workspace_id.as_deref().filter(|s| !s.is_empty()) {
                u.push_str("&workspaceId=");
                u.push_str(&escape_query_value(ws_id));
            }
            u
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                window_label = %window_label,
                "[floating-pane] frontend assets unavailable — opening static error page",
            );
            crate::commands::window::assets_missing_data_url(&e)
        }
    };

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

/// WndProc for the floating-pane class. Removes the system non-client
/// area (no native title bar / borders drawn) and maps the 6 CSS-px
/// edge bands to `HT{LEFT,RIGHT,...}` so the window remains resizable.
///
/// Window-drag is intentionally NOT handled here. We tried HTCAPTION
/// over the pane header but two problems made it unworkable:
///
///   1. The pane header sits below tile-layout padding (some y-offset
///      from the window top), so any hard-coded Y range is fragile.
///   2. Mouse events in an HTCAPTION zone never reach the CEF child
///      HWND — buttons inside the header (close / magnify / mic /
///      view-specific endIconButtons) stop responding.
///
/// Window drag is instead JS-driven in
/// `frontend/app/workspace/floating-pane-workspace.tsx`: a targeted
/// document-level mousedown listener scoped to `[data-role="block-header"]`,
/// `preventDefault`-ing to suppress the HTML5 dragstart that
/// pragmatic-dnd would have used (which otherwise tore the pane off
/// into a second floating window — the "double tear-off" bug). Same
/// pattern as the main window's `useWindowDrag` hook.
///
/// See `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`.
// ── Edge-resize hit-test forwarder ───────────────────────────────────────
// SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md (PR A).
//
// The floater's parent wndproc maps a resize border in WM_NCHITTEST, but the
// embedded CEF child windows fill the client area (incl. the border) and
// receive the edge mouse events first — so the parent's HT{LEFT,…} zones are
// never reached and native resize never starts. We subclass the floater's
// descendant CEF windows so that, over the border, they return HTTRANSPARENT
// (falling through to the floater's own WM_NCHITTEST → native resize). All
// other messages — and WM_NCHITTEST off the border — delegate unchanged.

/// Resize-border width in CSS px; DPI-scaled per call. Shared by the floater
/// wndproc's WM_NCHITTEST and the child forwarder so the strips line up. A
/// hard-coded physical-px constant would shrink the zone on HiDPI (6 physical
/// px = 3 CSS px at 200% → unreachable).
const RESIZE_BORDER_CSS: i32 = 6;

/// If `(screen_x, screen_y)` lies in `floater`'s resize border, return the
/// matching `HT*` edge/corner code; else `None`. Screen coords (the
/// `WM_NCHITTEST` lparam space).
unsafe fn floater_resize_hittest(
    floater: *mut std::ffi::c_void,
    screen_x: i32,
    screen_y: i32,
) -> Option<isize> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let mut rect: RECT = std::mem::zeroed();
    if GetWindowRect(floater, &mut rect) == 0 {
        return None;
    }
    let inside = screen_x >= rect.left
        && screen_x < rect.right
        && screen_y >= rect.top
        && screen_y < rect.bottom;
    if !inside {
        return None;
    }
    let dpi = GetDpiForWindow(floater) as i32;
    let dpi = if dpi > 0 { dpi } else { 96 };
    let b = (RESIZE_BORDER_CSS * dpi / 96).max(1);
    let left = screen_x - rect.left < b;
    let right = rect.right - screen_x < b;
    let top = screen_y - rect.top < b;
    let bottom = rect.bottom - screen_y < b;
    let ht = match (top, bottom, left, right) {
        (true, _, true, _) => HTTOPLEFT,
        (true, _, _, true) => HTTOPRIGHT,
        (_, true, true, _) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (_, _, true, _) => HTLEFT,
        (_, _, _, true) => HTRIGHT,
        (true, _, _, _) => HTTOP,
        (_, true, _, _) => HTBOTTOM,
        _ => return None,
    };
    Some(ht as isize)
}

/// Subclass a CEF child window's `WndProc` so its `WM_NCHITTEST` returns
/// `HTTRANSPARENT` for cursor positions within the floater's resize border —
/// the OS then dispatches the hit-test to the parent (floater) wndproc, whose
/// `WM_NCHITTEST` (`HT{LEFT,…}`) runs the native `WS_THICKFRAME` resize.
/// Non-edge hits delegate to the original wndproc (`HTCLIENT`).
///
/// Why: a CEF browser embedded as `WS_CHILD` covers the whole client area, and
/// the OS sends `WM_NCHITTEST` to the **topmost window under the cursor = the
/// child**, which returns `HTCLIENT` — so without this the parent's
/// `WM_NCHITTEST` never fires at the edge and resize is a no-op. Install on the
/// floater's `GW_CHILD` **synchronously at create time** (the child exists
/// right after `browser_host_create_browser`); installing later (e.g. in
/// `on_after_created`) misses it. (Proven approach from the parked #1132 work;
/// the earlier inset variant left a thick white DWM border that couldn't be
/// repainted, so it was abandoned for this forwarder.)
///
/// Idempotent (keyed by child HWND). Prunes its map entry on `WM_NCDESTROY` so
/// a recycled HWND value can't make a later install short-circuit. Win32-only.
pub(crate) unsafe fn install_child_nchittest_forwarder(child_hwnd: *mut std::ffi::c_void) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWLP_WNDPROC,
    };

    type Raw = unsafe extern "system" fn(*mut std::ffi::c_void, u32, usize, isize) -> isize;
    static ORIGINAL_WNDPROCS: Mutex<Option<HashMap<usize, isize>>> = Mutex::new(None);

    unsafe extern "system" fn forwarder(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CallWindowProcW, DefWindowProcW, GetParent, HTTRANSPARENT, WM_NCDESTROY, WM_NCHITTEST,
        };
        let original = || -> isize {
            ORIGINAL_WNDPROCS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&(hwnd as usize)).copied())
                .unwrap_or(0)
        };
        if msg == WM_NCDESTROY {
            // Child torn down — forward to the original so CEF finishes
            // teardown, THEN prune the map entry (before the HWND value can be
            // recycled). Without this, a recycled HWND makes the idempotent
            // install short-circuit and resize silently breaks. ReAgent P2 (#1132).
            let orig = original();
            let result = if orig != 0 {
                CallWindowProcW(Some(std::mem::transmute::<isize, Raw>(orig)), hwnd, msg, wparam, lparam)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            };
            if let Some(m) = ORIGINAL_WNDPROCS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                m.remove(&(hwnd as usize));
            }
            return result;
        }
        if msg == WM_NCHITTEST {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            // Measure the edge band against the PARENT (the floater) rect; the
            // child fills it, so the shared helper gives the same band as the
            // parent's own WM_NCHITTEST.
            let parent = GetParent(hwnd);
            let measure = if parent.is_null() { hwnd } else { parent };
            if floater_resize_hittest(measure, x, y).is_some() {
                return HTTRANSPARENT as isize;
            }
        }
        let orig = original();
        if orig != 0 {
            CallWindowProcW(Some(std::mem::transmute::<isize, Raw>(orig)), hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    let mut guard = ORIGINAL_WNDPROCS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.contains_key(&(child_hwnd as usize)) {
        return; // already subclassed — idempotent
    }
    let original = GetWindowLongPtrW(child_hwnd, GWLP_WNDPROC);
    map.insert(child_hwnd as usize, original);
    SetWindowLongPtrW(child_hwnd, GWLP_WNDPROC, forwarder as *const () as isize);
    tracing::info!(target: "edge-resize", child = ?child_hwnd, "installed WM_NCHITTEST forwarder on CEF child");
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn floating_pane_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    match msg {
        // Claim the entire window rect as client area — no system title
        // bar / borders drawn. WS_THICKFRAME (via WS_OVERLAPPEDWINDOW)
        // still gives us the resize border for `WM_NCHITTEST` to map.
        WM_NCCALCSIZE if wparam == 1 => return 0,
        // Suppress the DWM activation border repaint.
        WM_NCACTIVATE => return 1,
        WM_NCHITTEST => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            // Shared border math (same helper the child forwarder uses, so the
            // child's HTTRANSPARENT strip and this strip are pixel-identical).
            if let Some(ht) = floater_resize_hittest(hwnd, x, y) {
                return ht;
            }

            // Pane-header drag is JS-driven (see floating-pane-workspace.tsx);
            // we don't map any zone to HTCAPTION here. Fall through to
            // HTCLIENT so clicks reach CEF and the JS handler.
        }
        // Resize the floater's FRONTEND browser (header + layout) to fill the
        // client area on every outer-window resize (maximize / restore, and
        // future edge-resize). CEF `set_as_child` browsers don't self-resize,
        // and our custom wndproc replaced CEF's default proc, so without this
        // the browser stays at its creation size while the outer grows.
        //
        // CRITICAL: resize the BOTTOM-most direct child, NOT `GW_CHILD` (the
        // topmost). The frontend browser is embedded at floater creation, so
        // it's the oldest child and sits at the BOTTOM of the Z-order. A
        // BROWSER pane adds a SECOND direct child later — the native
        // web-content window from `browser_pane_create` — which lands ABOVE
        // the frontend. Resizing the topmost child therefore stretched the
        // web-content window over the whole floater, hiding the header and
        // swallowing all input (the "maximized browser pane is fullscreen,
        // can't click anything" trap). We only size the frontend browser; once
        // it has the new viewport it reflows and repositions its own
        // web-content child below the header via `browser_pane_resize`. CEF
        // cascades the resize to the frontend's inner render-widget children.
        // (Terminal/agent floaters have a single child, so bottom-most == that
        // child — unchanged.) Falls through to DefWindowProcW afterwards.
        WM_SIZE => {
            // Walk the sibling Z-order from topmost (GW_CHILD) to the
            // bottom-most direct child = the frontend browser.
            let mut frontend = GetWindow(hwnd, GW_CHILD);
            let mut next = frontend;
            while !next.is_null() {
                frontend = next;
                next = GetWindow(next, GW_HWNDNEXT);
            }
            if !frontend.is_null() {
                let mut rc = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                if GetClientRect(hwnd, &mut rc) != 0 {
                    SetWindowPos(
                        frontend,
                        std::ptr::null_mut(),
                        0,
                        0,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
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
        CreateWindowExW, RegisterClassExW, ShowWindow, CS_HREDRAW, CS_VREDRAW, SW_SHOWNOACTIVATE,
        WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP, WS_THICKFRAME,
    };

    // ---- Register the class once per process ----
    static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();
    static CLASS_NAME: &str = "AgentMuxFloatingPane";

    let mut class_name_utf16: Vec<u16> = OsStr::new(CLASS_NAME).encode_wide().collect();
    class_name_utf16.push(0);

    // TODO(phase-6, codex P1 on #811 — explicitly deferred): The
    // documented CEF embedding pattern is for the host's wndproc to
    // intercept WM_CLOSE and route through `CloseBrowser(false)` so
    // DoClose fires before destroy. Today the OS X-button cascade
    // still works end-to-end via `floating_pane_wndproc`'s fallthrough
    // to `DefWindowProcW(WM_CLOSE)`:
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
    // such prompt, so this is harmless. The full WM_CLOSE → CloseBrowser
    // routing is still future work.
    CLASS_REGISTERED.call_once(|| unsafe {
        let h_instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(floating_pane_wndproc),
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
            // are clipped to parent's client area). WS_THICKFRAME for
            // the resize border. NO `WS_CAPTION` — Win32 still reserves
            // title-bar space for WS_CAPTION windows even when
            // WM_NCCALCSIZE returns 0, which leaves a system title bar
            // drawn on top of the client area AND truncates the
            // effective client size (CEF embedded at (0,0,W,H) overruns
            // the visible client → content cut off bottom+right). The
            // frontend's `BlockFrame_Header` is the only chrome.
            WS_POPUP | WS_THICKFRAME,
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

    // Tell DWM to extend the entire frame into the client area. Without
    // this, DWM keeps drawing the standard Win32 title bar (and the
    // minimize/maximize/close caption buttons that come with it) on top
    // of our client area, even though our `WM_NCCALCSIZE → 0` says the
    // client area fills the window. Mirrors the main-window setup in
    // `client/wndproc.rs::setup_native_frameless` — combined with our
    // WndProc's `WM_NCCALCSIZE`/`WM_NCACTIVATE`/`WM_NCHITTEST`, this
    // gives a truly chrome-free outer HWND. The docked-pane's standard
    // `BlockFrame_Header` (33 CSS px, `--header-height` in theme.scss:97)
    // is the sole chrome — drag is JS-driven from
    // `frontend/app/workspace/floating-pane-workspace.tsx`.
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
        use windows_sys::Win32::UI::Controls::MARGINS;
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let hr = DwmExtendFrameIntoClientArea(hwnd, &margins);
        if hr != 0 {
            tracing::warn!(
                "[floating-pane] DwmExtendFrameIntoClientArea failed hr=0x{hr:08x} — system title bar may still be drawn",
            );
        }
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
