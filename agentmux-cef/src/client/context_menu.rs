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
    /// Runs on CEF's `run_context_menu`. Suppresses CEF's native menu for a
    /// browser pane — returns 1 (handled) and calls `callback.cancel()` —
    /// ONLY when `browser-pane-context-menu` was actually routed somewhere a
    /// listener can hear it. When `block_id`/`params` can't be resolved, OR
    /// the pane's owning window can't be determined, falls through to CEF's
    /// own native menu (return 0) instead: suppressing unconditionally in
    /// that case would show NO menu at all (reagentx P1 on PR #2599) — worse
    /// than Chromium's native one.
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
        frame: Option<&mut Frame>,
        params: Option<&mut ContextMenuParams>,
        callback: Option<&mut RunContextMenuCallback>,
    ) -> ::std::os::raw::c_int {
        let Some(cb) = callback else { return 0 };
        let Some(b) = browser.as_deref() else { return 0 };
        let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b) else {
            return 0;
        };
        let Some(p) = params else { return 0 };

        // Route to the pane's ACTUAL owning window, not just "main" — a pane
        // torn off into its own floating window has its own JS context, and
        // `blockframe.tsx`'s listener there is what needs this event. Same
        // fix already applied to `browser-pane-shortcut` (codex P2 on
        // #2548) — see that call site for the identical pattern.
        let Some(window_label) = self.state.browser_pane_window_label(&block_id) else {
            tracing::warn!(
                "[browser-pane-context-menu] no owning window label for block_id={}, falling back to CEF's native menu",
                block_id
            );
            return 0;
        };

        // Stash the ACTUAL frame that was right-clicked (not necessarily
        // `browser.main_frame()` — a page with sub-frames/iframes, e.g. ads
        // or embeds, can have the selection/focus live in one of those
        // instead) so the Copy/Cut/Paste menu items act on the right frame
        // when the user clicks one, rather than always the top-level frame
        // regardless of where the right-click actually landed (reagentx P1
        // on PR #2599). Print/View Source/Inspect/Reload/Back/Forward stay
        // top-level-only — CEF's own equivalents for those are Browser-level
        // (or, for View Source, an accepted simplification — see
        // `browser_panes::navigation::view_source`'s doc comment).
        if let Some(f) = frame.as_deref() {
            self.state
                .browser_pane_context_menu_frame
                .lock()
                .insert(block_id.clone(), f.clone());
        }

        // Coordinates are relative to the PANE's own render view origin, not
        // the main window — the frontend translates them using the pane's
        // own `.browser-placeholder` rect (the inner content element the
        // native overlay is positioned to exactly match — NOT the outer
        // `.block-<block_id>` wrapper, which also contains the header/title
        // bar/nav bar above it; using that offset the menu downward by the
        // header chrome's height, reagentx P1 on an earlier PR #2599 pass).
        // See the `browser-pane-context-menu` listener in `blockframe.tsx`.
        let x = p.xcoord();
        let y = p.ycoord();
        let link_url = CefString::from(&p.link_url()).to_string();
        let page_url = CefString::from(&p.page_url()).to_string();
        let selection_text = CefString::from(&p.selection_text()).to_string();
        let is_editable = p.is_editable() != 0;

        let mut b_owned = b.clone();
        let can_go_back = b_owned.can_go_back() != 0;
        let can_go_forward = b_owned.can_go_forward() != 0;

        let delivered = crate::events::emit_event_to_window(
            &self.state,
            &window_label,
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
        if !delivered {
            return 0;
        }

        cb.cancel();
        1
    }
}
