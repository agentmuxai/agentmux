// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/find` — backs Browser's `find`: the lines of the
//! page's accessibility tree whose element has a given role and name, each
//! with a reference that `click`, `fill`, `select` and `check` take, so an
//! agent needn't read a whole big page to act on one element
//! (docs/specs/PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §7). The host reads
//! the tree and keeps the references, as for a snapshot.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use agentmux_common::api_types::{UiBrowserElementMatch, UiBrowserFindRequest};

use crate::server::ui_handlers::{err_response, proxy_data, target_block_id};
use crate::server::AppState;

/// The host's request for `query` on `block_id`, or why there is none: a
/// find with neither a name nor a role would list the whole page.
fn host_body(block_id: &str, query: &UiBrowserElementMatch) -> Result<serde_json::Value, String> {
    let role = query.role.as_deref().map(str::trim).filter(|r| !r.is_empty());
    if query.name.trim().is_empty() && role.is_none() {
        return Err("find needs a name (part of the element's accessible name), a role, or both".to_string());
    }
    Ok(json!({ "block_id": block_id, "role": role, "name": query.name, "exact": query.exact }))
}

pub(crate) async fn handle_ui_browser_find(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserFindRequest>,
) -> impl IntoResponse {
    // The same checks as a snapshot: the caller's identity, that the pane is
    // theirs, and that it isn't waiting for the user.
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let body = match host_body(&block_id, &req.query) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser find");
    proxy_data(&state, "find", body, std::time::Duration::from_secs(30)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(role: Option<&str>, name: &str, exact: bool) -> UiBrowserElementMatch {
        UiBrowserElementMatch { role: role.map(str::to_string), name: name.to_string(), exact }
    }

    #[test]
    fn the_host_gets_the_pane_and_the_query() {
        let body = host_body("b1", &q(Some(" button "), "Save", true)).unwrap();
        assert_eq!(body, json!({ "block_id": "b1", "role": "button", "name": "Save", "exact": true }));
        let body = host_body("b1", &q(None, "Save", false)).unwrap();
        assert_eq!(body["role"], serde_json::Value::Null);
        assert!(host_body("b1", &q(Some("dialog"), "", false)).is_ok(), "a role alone is enough");
    }

    #[test]
    fn a_find_for_nothing_in_particular_is_refused() {
        let e = host_body("b1", &q(Some("  "), " ", false)).unwrap_err();
        assert!(e.starts_with("find needs a name"), "{e}");
    }

    #[test]
    fn the_request_reads_its_query_beside_the_signature() {
        let req: UiBrowserFindRequest = serde_json::from_value(json!({
            "agent_id": "a", "ts_secs": 1, "sig": "s", "pane": "p", "role": "link", "name": "Next"
        }))
        .unwrap();
        assert_eq!(req.query, q(Some("link"), "Next", false));
        assert_eq!(req.pane.as_deref(), Some("p"));
    }
}
