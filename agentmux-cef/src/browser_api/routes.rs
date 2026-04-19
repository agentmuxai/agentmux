// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Axum route handlers for `/agentmux/browser/*`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::json;

use super::cdp::CdpSession;
use super::types::{ApiResponse, Element, QueryData, QueryReq};
use crate::state::AppState;

/// `POST /agentmux/browser/query` — find DOM elements matching a CSS
/// selector in the specified pane. See `SPEC_BROWSER_DOM_API.md`
/// §5.2 for response shape.
pub async fn query(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<QueryReq>,
) -> (StatusCode, Json<ApiResponse<QueryData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    // Resolve block_id → CDP target id.
    let target = match state
        .browser_api
        .target_cache
        .resolve(&state, &req.block_id)
        .await
    {
        Ok(t) => t,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    // Open the CDP WebSocket for that target.
    let debug_port = *state.debug_port.lock();
    let ws_url = format!("ws://127.0.0.1:{debug_port}/devtools/page/{target}");
    let mut cdp = match CdpSession::connect(&ws_url).await {
        Ok(s) => s,
        Err(e) => {
            // Target might be stale (pane closed underneath us).
            state.browser_api.target_cache.forget(&req.block_id);
            return ok_body(ApiResponse::err(format!("CDP connect: {e}")));
        }
    };

    // Inject the helper (idempotent — it guards on `window.__amq_query`).
    let helper = include_str!("scripts/query.js");
    if let Err(e) = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": helper,
                "returnByValue": false,
            }),
        )
        .await
    {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(format!("CDP inject helper: {e}")));
    }

    // Call the helper. Serializing the selector through serde_json
    // handles quote-escaping safely.
    let selector_js = serde_json::to_string(&req.selector)
        .unwrap_or_else(|_| "\"\"".to_string());
    let call_expr = format!(
        "__amq_query({sel}, {lim})",
        sel = selector_js,
        lim = req.limit.unwrap_or(0),
    );
    let eval_result = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": call_expr,
                "returnByValue": true,
                "awaitPromise": false,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP call __amq_query: {e}")));
        }
    };

    let _ = cdp.close().await;

    // CDP Runtime.evaluate reply shape (with returnByValue):
    //   { result: { type, value } } or { result: { type, value: { error } } }
    let value = eval_result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Check for script-level error surfaced by the helper.
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return ok_body(ApiResponse::err(format!("DOM query error: {err}")));
    }

    // Normal case: { matches: [...] }
    let matches: Vec<Element> = value
        .get("matches")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();

    ok_body(ApiResponse::ok(QueryData { matches }))
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| token == expected)
        .unwrap_or(false)
}

fn ok_body<T>(body: ApiResponse<T>) -> (StatusCode, Json<ApiResponse<T>>) {
    (StatusCode::OK, Json(body))
}
