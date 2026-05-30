// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! UI-thread task that actually creates a CEF browser pane.
//!
//! Moved out of `browser_panes.rs` during Phase 3 of the pane modularization
//! split (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). The task
//! structure here is a straight lift — same pre-flight checks, same
//! `browser_host_create_browser` call. `BrowserPaneManager::create` still
//! calls `post_task(ThreadId::UI, ..)` with an instance of this task; the
//! only change is the import path.
//!
//! Dependencies (one-way, no cycle):
//!   - `cef::*` for CEF types and the `wrap_task!` macro.
//!   - `crate::state::AppState` for the label queue and the Arc passed to
//!     the pane's handler.
//!   - `crate::client::{AgentMuxHandler, AgentMuxClient}` for the pane's
//!     CEF client. Phase 4 will flip this direction by moving the pane
//!     callbacks into `pane/callbacks.rs`; until then, `client` is fine as
//!     a one-way dependency.
//!   - `crate::commands::window::resolve_window_hwnd` to resolve the parent
//!     HWND from the pane's target `window_label` (CEF Views returns null on
//!     Alloy, so the Windows native-child path needs the HWND explicitly).

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

wrap_task! {
    pub struct CreateBrowserPaneTask {
        state: Arc<AppState>,
        block_id: String,
        label: String,
        url: String,
        rect: Rect,
        // The pane's TARGET top-level window. Both platforms read it:
        // Linux/macOS looks up the CefWindow to attach the overlay to (in
        // `state.windows`); Windows resolves it to the parent HWND via
        // `resolve_window_hwnd`, so a redocked pane is parented to its target
        // window rather than the process's first-visible top-level (the floater).
        window_label: String,
    }

    impl Task {
        fn execute(&self) {
            // Running on the CEF UI thread.
            //
            // Two completely separate paths by platform:
            //   - Windows: native child window via WindowInfo::set_as_child +
            //     browser_host_create_browser. Requires the parent HWND,
            //     resolved from `window_label` via `resolve_window_hwnd`.
            //   - Linux / macOS: CEF Views via browser_view_create +
            //     Window::add_overlay_view. The Windows native-child path does
            //     not work on Wayland (cef#2804) and is officially unsupported
            //     on macOS. We use AddOverlayView (not AddChildView) so the
            //     pane cohabits cleanly with the host UI's full-window
            //     BrowserView. See
            //     `docs/specs/embedded-browser-panes-linux-macos-2026-05-03.md`.

            #[cfg(not(target_os = "windows"))]
            {
                crate::browser_pane::creation_views::create_browser_pane_view(
                    self.state.clone(),
                    self.block_id.clone(),
                    self.label.clone(),
                    self.url.clone(),
                    self.rect.clone(),
                    self.window_label.clone(),
                );
                return;
            }

            #[cfg(target_os = "windows")]
            {
                // Resolve the parent HWND from the pane's TARGET `window_label`,
                // NOT `find_own_top_level_window` (which returns the process's
                // FIRST visible top-level). Under redock churn the still-alive
                // floater is that first window, so the redocked pane was parented
                // to the dying floater and cascade-destroyed when its HWND died
                // → black render. Proven via the `create-parent` pane-trace:
                // requested_window=main but parent_hwnd == the floater's HWND.
                // `resolve_window_hwnd` is cache-first + IsWindow-guarded and
                // handles `main` and `floating-*` identically.
                let parent_hwnd_raw = unsafe {
                    crate::commands::window::resolve_window_hwnd(&self.state, &self.window_label)
                };
                if parent_hwnd_raw.is_null() {
                    tracing::error!(block_id = %self.block_id, window_label = %self.window_label, "cannot resolve target window HWND — aborting browser pane creation");
                    return;
                }

                // [pane-trace] CONFIRM the parent window. `find_own_top_level_window`
                // returns the process's FIRST visible top-level, NOT necessarily
                // `window_label`'s window. Under redock churn the dying floater can
                // be that first window, so the freshly-redocked pane gets parented
                // to it and is cascade-destroyed when the floater's HWND dies
                // (child-before-parent) → black render. If `actual_parent_label`
                // here is a `floating-*` while `requested_window` is `main`, that's
                // the bug. DO NOT REMOVE (see browser_pane::trace).
                {
                    let actual_parent_label = self
                        .state
                        .window_hwnds
                        .lock()
                        .iter()
                        .find(|(_, h)| **h == parent_hwnd_raw as isize)
                        .map(|(l, _)| l.clone());
                    crate::browser_pane::trace::pane_trace(
                        &self.block_id,
                        "create-parent",
                        &format!(
                            "requested_window={} parent_hwnd={:?} actual_parent_label={:?}",
                            self.window_label, parent_hwnd_raw, actual_parent_label
                        ),
                    );
                }

                // Phase B.5 (window_meta step d) — pre-create handoff.
                // Browser panes are not top-level windows; the kind value here
                // is irrelevant (on_after_created skips the taskbar/report-open
                // logic for browser-pane-* labels).
                // Phase F.1 — routed through the host reducer.
                self.state.host_dispatch(
                    crate::reducer::HostCommand::EnqueuePendingWindowCreation {
                        entry: crate::state::PendingWindowCreation {
                            label: self.label.clone(),
                            kind: crate::state::WindowKind::FullInstance,
                            parent_instance_id: None,
                        },
                    },
                );

                let handler = crate::client::AgentMuxHandler::new_with_browser_pane(self.state.clone(), 0, true);
                let mut client = Some(crate::client::AgentMuxClient::new(handler, true));

                let url_cef = CefString::from(self.url.as_str());
                let settings = BrowserSettings::default();

                let parent_hwnd = sys::HWND(parent_hwnd_raw as *mut _);
                // Use the clean set_as_child helper — it fills style/parent/bounds
                // correctly and leaves other fields zeroed (in particular `window`
                // which is an OUTPUT field filled by CEF).
                let mut window_info = WindowInfo::default().set_as_child(parent_hwnd, &self.rect);
                // Match the main process runtime style (ALLOY throughout the app).
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
                    tracing::error!(block_id = %self.block_id, "browser_host_create_browser returned 0");
                    return;
                }

                tracing::info!(
                    block_id = %self.block_id,
                    label = %self.label,
                    url = %self.url,
                    x = self.rect.x, y = self.rect.y,
                    w = self.rect.width, h = self.rect.height,
                    "browser pane created on UI thread"
                );
            }
        }
    }
}
