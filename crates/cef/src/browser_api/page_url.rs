// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The address a browser pane shows, for srv's own checks (a bypass grant, a
//! standing grant, the origin a committing click happens on). Not `eval`:
//! `eval` is the agent's tool and scans the whole page for Vault values and
//! one-time secrets first, which on a large page (a cloud console with dozens
//! of frames) takes longer than the person's button waits, and it refuses
//! outright on a page that shows a secret. It also runs on the page's main
//! thread, behind whatever the page is doing. The navigation history is the
//! browser's own record of where the pane is, answered without running page
//! script.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::act::{fail, unauthorized};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PageUrlReq {
    pub block_id: String,
}

/// `POST /agentmux/browser/page_url` — the address the pane's page is on, as
/// `{ url }`.
pub async fn page_url_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PageUrlReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "page_url").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let history = cdp.call("Page.getNavigationHistory", json!({})).await;
    let _ = cdp.close().await;
    match history.map_err(|e| e.to_string()).and_then(|h| current_url(&h).ok_or_else(|| "couldn't read the page's address".to_string())) {
        Ok(url) => (StatusCode::OK, Json(ApiResponse::ok(json!({ "url": url })))),
        Err(e) => fail(e),
    }
}

/// The address of the history entry the page is on.
fn current_url(history: &Value) -> Option<String> {
    let at = history.get("currentIndex")?.as_u64()? as usize;
    history.get("entries")?.as_array()?.get(at)?.get("url")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_the_entry_the_page_is_on_not_the_last_one() {
        let h = json!({
            "currentIndex": 1,
            "entries": [{ "url": "https://a.test/" }, { "url": "https://b.test/x" }, { "url": "https://c.test/" }],
        });
        assert_eq!(current_url(&h).as_deref(), Some("https://b.test/x"));
    }

    #[test]
    fn an_answer_without_an_entry_has_no_address() {
        assert_eq!(current_url(&json!({ "currentIndex": 0, "entries": [] })), None);
        assert_eq!(current_url(&json!({ "currentIndex": 3, "entries": [{ "url": "https://a.test/" }] })), None);
        assert_eq!(current_url(&json!({})), None);
    }
}
