// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A browser pane's own page closing itself (`window.close()`, e.g. a
//! sign-in's "close this window").
//!
//! AgentMux never closes a pane through CEF: `BrowserPaneManager::close`
//! marks it Closing first, so a pane still Live in `do_close` was closed by
//! its page. Letting CEF run its default close there treats the pane and main
//! as one close unit (browser_panes/close.rs) and takes the whole window down,
//! which quit AgentMux
//! (docs/retro/RETRO_BROWSER_PANE_WINDOW_CLOSE_QUITS_APP_2026_10_16.md).
//! Instead the close is held back and the pane's own window closes the pane,
//! as its × does (`browser-model.ts`), which tears it down the safe way.

use cef::Browser;

use crate::state::AppState;

/// For `do_close` on a browser pane's own browser (not one of its popups):
/// true when its page closed it and the close is held back (the caller
/// returns true to CEF); false to let the close go on as before.
#[cfg(target_os = "windows")]
pub(crate) fn held_back(state: &AppState, browser: &mut Browser) -> bool {
    let Some(block_id) = state.block_id_for_browser(browser) else {
        return false;
    };
    // Routed to the pane's own window (a torn-off pane has its own JS
    // context), like `browser-pane-clicked`.
    match state.browser_pane_window_label(&block_id) {
        Some(window_label) => {
            tracing::info!(block_id = %block_id, window = %window_label, "browser pane's page closed itself; asking its window to close the pane");
            crate::events::emit_event_to_window(
                state,
                &window_label,
                "browser-pane-close-requested",
                &serde_json::json!({ "block_id": block_id }),
            );
        }
        None => {
            tracing::warn!(block_id = %block_id, "browser pane's page closed itself, but the pane has no window to close it")
        }
    }
    true
}

/// macOS and Linux panes are CEF Views, whose default close differs and is
/// untested here: unchanged.
#[cfg(not(target_os = "windows"))]
pub(crate) fn held_back(_state: &AppState, _browser: &mut Browser) -> bool {
    false
}
