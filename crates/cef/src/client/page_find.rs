// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The answer to a browser pane's find in page: CEF reports each count update, and the host tells the
//! window that owns the pane, whose find bar shows "3 of 12".

use std::sync::Arc;

use cef::*;

/// The `browser-pane-find-result` event's payload, with CEF's numbers as they come: an update that changes only
/// the count carries `active` -1 (the current match is unchanged), and one that changes only the current match
/// carries `count` -1. The window keeps what it has for a -1; `count` grows over several updates until `final`.
pub(crate) fn find_result_payload(block_id: &str, count: i32, active: i32, final_update: bool) -> serde_json::Value {
    serde_json::json!({ "block_id": block_id, "count": count, "active": active, "final": final_update })
}

wrap_find_handler! {
    pub(crate) struct AgentMuxFindHandler {
        state: Arc<crate::state::AppState>,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            browser: Option<&mut Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            final_update: ::std::os::raw::c_int,
        ) {
            let Some(browser) = browser else { return };
            let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, browser) else { return };
            let Some(label) = self.state.browser_pane_window_label(&block_id) else { return };
            let payload = find_result_payload(&block_id, count, active_match_ordinal, final_update != 0);
            crate::events::emit_event_to_window(&self.state, &label, "browser-pane-find-result", &payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::find_result_payload;

    #[test]
    fn a_result_names_the_pane_and_passes_cefs_unchanged_markers_through() {
        // CEF reports -1 for "this number did not change": the window must be told, not given a 0.
        let p = find_result_payload("b1", 12, 3, true);
        assert_eq!(p, serde_json::json!({ "block_id": "b1", "count": 12, "active": 3, "final": true }));
        let p = find_result_payload("b1", -1, -1, false);
        assert_eq!(p, serde_json::json!({ "block_id": "b1", "count": -1, "active": -1, "final": false }));
    }
}
