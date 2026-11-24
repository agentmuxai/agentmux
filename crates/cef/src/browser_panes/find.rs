// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Find in page for a browser pane (Ctrl/Cmd+F): the page's own text search, which CEF runs and
//! highlights. The answer (how many matches, which one is current) comes back through the pane's find
//! handler (`client/page_find.rs`) as the `browser-pane-find-result` event.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::BrowserPaneManager;

/// Longest text the host searches for: a search box holds a word or a phrase, never a page.
const MAX_FIND_CHARS: usize = 500;

impl BrowserPaneManager {
    /// The `browser_pane_find` and `browser_pane_stop_find` IPC commands (`ipc.rs`).
    pub fn run_find_command(&self, cmd: &str, args: &serde_json::Value, state: &Arc<AppState>) {
        let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
        if cmd == "browser_pane_stop_find" {
            let clear = args.get("clear_selection").and_then(|v| v.as_bool()).unwrap_or(true);
            return self.stop_find(block_id, clear, state);
        }
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let flag = |key: &str, default: bool| args.get(key).and_then(|v| v.as_bool()).unwrap_or(default);
        self.find(block_id, text, flag("forward", true), flag("match_case", false), flag("find_next", false), state);
    }
    /// Searches the pane's page for `text` (`find_next`: step to the next or previous match of the same
    /// text; otherwise a new search from the top of the page). An empty text ends the search instead.
    pub fn find(&self, block_id: &str, text: &str, forward: bool, match_case: bool, find_next: bool, state: &Arc<AppState>) {
        if text.is_empty() {
            self.stop_find(block_id, true, state);
            return;
        }
        let text: String = text.chars().take(MAX_FIND_CHARS).collect();
        if let Some(host) = self.live_browser(state, block_id).and_then(|b| b.host()) {
            host.find(Some(&CefString::from(text.as_str())), forward as i32, match_case as i32, find_next as i32);
        }
    }

    /// Ends the search in the pane's page. `clear_selection`: the page's own selection goes too (closing
    /// the find bar); otherwise the current match stays selected, which is what Enter in the bar leaves.
    pub fn stop_find(&self, block_id: &str, clear_selection: bool, state: &Arc<AppState>) {
        if let Some(host) = self.live_browser(state, block_id).and_then(|b| b.host()) {
            host.stop_finding(clear_selection as i32);
        }
    }
}
