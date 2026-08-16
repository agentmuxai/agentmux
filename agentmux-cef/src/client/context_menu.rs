// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `ContextMenuHandler` body for browser panes — suppresses CEF's native
//! Chrome-style right-click menu (Back/Forward/Reload/Print/View Page
//! Source/Inspect) and hands control to the frontend's own unified pane
//! context menu instead. See
//! docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md.
//!
//! Main-app windows never install this handler at all (see
//! `client::handlers::AgentMuxClient::context_menu_handler`, gated on
//! `is_browser_pane`) — the main app's own right-click menu is handled
//! entirely in DOM via `app.tsx`'s `onContextMenu={showTextInputContextMenu}`,
//! which already works because the main app IS its own DOM content, unlike a
//! browser pane's native overlay.

use cef::*;

use super::AgentMuxHandler;

impl AgentMuxHandler {
    /// Runs on CEF's `run_context_menu`. ALWAYS suppresses CEF's native menu
    /// for a browser pane — returns 1 (handled) and calls `callback.cancel()`
    /// unconditionally, including when `block_id`/`params` can't be resolved,
    /// since falling through to Chromium's own Back/Forward/Print/View
    /// Source/Inspect menu in that edge case would be a worse and more
    /// confusing experience than the pane simply not showing a context menu
    /// that one time (same posture as other block_id-resolution failures
    /// elsewhere in this file, e.g. `on_loading_state_change_browser_pane`).
    ///
    /// `cancel()`, never `cont()`: `cont()` tells CEF a specific native menu
    /// command was chosen (by id) and to execute it — we're not running any
    /// CEF-native command through this path at all. The frontend's menu
    /// items (Back/Forward/Reload/Print/View Source/Inspect/split/replace/
    /// color/close) each invoke their own existing IPC command directly
    /// (`browser_pane_go_back`, `browser_pane_print`, etc.) once the user
    /// clicks one — entirely independent of this callback.
    pub(crate) fn run_context_menu(
        &mut self,
        browser: Option<&mut Browser>,
        params: Option<&mut ContextMenuParams>,
        callback: Option<&mut RunContextMenuCallback>,
    ) -> ::std::os::raw::c_int {
        let Some(cb) = callback else { return 0 };
        let suppress = |cb: &mut RunContextMenuCallback| {
            cb.cancel();
            1
        };
        let Some(b) = browser.as_deref() else {
            return suppress(cb);
        };
        let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b) else {
            return suppress(cb);
        };
        let Some(p) = params else {
            return suppress(cb);
        };

        // Coordinates are relative to the PANE's own render view origin, not
        // the main window — the frontend translates them using the pane's
        // own DOM wrapper rect (`.block-<block_id>`), which the native
        // overlay is always positioned to exactly match. See
        // `browser-pane-context-menu-bridge.ts`.
        let x = p.xcoord();
        let y = p.ycoord();
        let link_url = CefString::from(&p.link_url()).to_string();
        let page_url = CefString::from(&p.page_url()).to_string();
        let selection_text = CefString::from(&p.selection_text()).to_string();
        let is_editable = p.is_editable() != 0;

        let mut b_owned = b.clone();
        let can_go_back = b_owned.can_go_back() != 0;
        let can_go_forward = b_owned.can_go_forward() != 0;

        crate::events::emit_event_from_state(
            &self.state,
            "browser-pane-context-menu",
            &serde_json::json!({
                "block_id": block_id,
                "x": x,
                "y": y,
                "link_url": link_url,
                "page_url": page_url,
                "selection_text": selection_text,
                "is_editable": is_editable,
                "can_go_back": can_go_back,
                "can_go_forward": can_go_forward,
            }),
        );

        suppress(cb)
    }
}
