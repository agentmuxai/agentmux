// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Axum route handlers for `/agentmux/browser/*`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::json;

use super::cdp::CdpSession;
use super::types::{
    ApiResponse, Element, EvalData, EvalReq, FocusInfoData, FocusInfoReq, QueryData, QueryReq,
    ScreenshotData, ScreenshotReq,
};
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

/// `POST /agentmux/browser/focus_info` — report the current
/// `document.activeElement` of the pane as an `Element`. See §5.2.
pub async fn focus_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FocusInfoReq>,
) -> (StatusCode, Json<ApiResponse<FocusInfoData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let mut cdp = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    // Inject helpers (idempotent).
    let helper = include_str!("scripts/query.js");
    if let Err(e) = cdp
        .call(
            "Runtime.evaluate",
            json!({ "expression": helper, "returnByValue": false }),
        )
        .await
    {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(format!("CDP inject helper: {e}")));
    }

    let eval_result = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": "__amq_focus_info()",
                "returnByValue": true,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP call __amq_focus_info: {e}")));
        }
    };

    let _ = cdp.close().await;

    let value = eval_result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // value is either null or an Element object.
    let focused: Option<Element> = if value.is_null() {
        None
    } else {
        match serde_json::from_value(value) {
            Ok(e) => Some(e),
            Err(e) => return ok_body(ApiResponse::err(format!("parse element: {e}"))),
        }
    };
    ok_body(ApiResponse::ok(FocusInfoData { focused }))
}

/// `POST /agentmux/browser/eval` — run arbitrary JS in the pane's
/// renderer, return the serialized value.
///
/// Thin wrapper over CDP `Runtime.evaluate`. The script runs in the
/// pane's JS world (not an isolated context); treat it as arbitrary
/// code execution in whatever origin the pane currently loads.
pub async fn eval(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<EvalReq>,
) -> (StatusCode, Json<ApiResponse<EvalData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let mut cdp = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    let eval_result = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": req.script,
                "returnByValue": true,
                "awaitPromise": req.await_promise,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP eval: {e}")));
        }
    };

    let _ = cdp.close().await;

    // CDP shape: { result: { type, value } } on success,
    //            { exceptionDetails: { ... } } on throw.
    if let Some(exc) = eval_result.get("exceptionDetails") {
        let msg = exc
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(|d| d.as_str())
            .or_else(|| exc.get("text").and_then(|t| t.as_str()))
            .unwrap_or("unknown exception")
            .to_string();
        return ok_body(ApiResponse::ok(EvalData {
            result: serde_json::Value::Null,
            type_: "undefined".to_string(),
            exception: Some(msg),
        }));
    }

    let result = eval_result.get("result").cloned().unwrap_or_default();
    let type_ = result
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("undefined")
        .to_string();
    let value = result.get("value").cloned().unwrap_or(serde_json::Value::Null);

    ok_body(ApiResponse::ok(EvalData {
        result: value,
        type_,
        exception: None,
    }))
}

/// `POST /agentmux/browser/screenshot` — capture a PNG of the pane's
/// rendered viewport. Uses CDP `Page.captureScreenshot`.
pub async fn screenshot(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ScreenshotReq>,
) -> (StatusCode, Json<ApiResponse<ScreenshotData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let mut cdp = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    let cap = match cdp
        .call(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP Page.captureScreenshot: {e}")));
        }
    };

    let _ = cdp.close().await;

    let data = match cap.get("data").and_then(|d| d.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return ok_body(ApiResponse::err(
                "Page.captureScreenshot returned no `data` field".to_string(),
            ))
        }
    };

    ok_body(ApiResponse::ok(ScreenshotData { png_base64: data }))
}

// ── shared helpers ──────────────────────────────────────────────────────

async fn open_cdp_for_block(
    state: &Arc<AppState>,
    block_id: &str,
) -> Result<CdpSession, String> {
    let target = state
        .browser_api
        .target_cache
        .resolve(state, block_id)
        .await?;
    let debug_port = *state.debug_port.lock();
    let ws_url = format!("ws://127.0.0.1:{debug_port}/devtools/page/{target}");
    CdpSession::connect(&ws_url).await.map_err(|e| {
        // On connect failure the cached target may be stale; drop it
        // so the next call re-probes.
        state.browser_api.target_cache.forget(block_id);
        format!("CDP connect: {e}")
    })
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
