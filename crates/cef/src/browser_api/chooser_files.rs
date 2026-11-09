// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `set_files` with `chooser: true`: the files go into the file chooser the page opened and the host
//! kept for the agent (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §6,
//! `client/chooser_hold.rs`). Whether the agent may at all, and which files, is srv's decision (the
//! grant's uploads power and the workspace rules); this route re-checks that the page is still on the
//! granted sites, and that the chooser is still waiting and still the page's own.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::act::{fail, page_url, unauthorized, UploadFile};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ChooserFilesReq {
    pub block_id: String,
    pub files: Vec<UploadFile>,
    /// The sites the person's grant is bound to; the page must still be on one.
    pub allowed_origins: Vec<String>,
}

/// `POST /agentmux/browser/chooser_files` — answers `{ after: [{ name, size }] }`.
pub async fn chooser_files_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChooserFilesReq>,
) -> (axum::http::StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "chooser_files").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = page_url(&mut cdp).await;
    let _ = cdp.close().await;
    if !agentmux_common::allowed_origins::allows(&req.allowed_origins, &now) {
        return fail("no file chooser was filled: the page is off the sites the person granted");
    }
    match crate::client::chooser_hold::fill(&req.block_id, &now, &req.files) {
        Ok(after) => (axum::http::StatusCode::OK, Json(ApiResponse::ok(json!({ "after": after })))),
        Err(e) => fail(e),
    }
}
