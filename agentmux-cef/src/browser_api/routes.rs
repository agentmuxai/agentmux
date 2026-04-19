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
    AckData, ApiResponse, ClickElementReq, DispatchKeyReq, Element, EvalData, EvalReq,
    FocusElementReq, FocusInfoData, FocusInfoReq, NavigateReq, QueryData, QueryReq,
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

/// `POST /agentmux/browser/click_element` — synthesize a real mouse
/// click on the first element matching `selector`. Dispatches
/// `Input.dispatchMouseEvent` (mousePressed + mouseReleased) at the
/// element's centroid.
///
/// Note: this is a "real" mouse event, NOT a DOM `.click()` — so
/// `:focus-visible`, pointer-related listeners, and the pane's
/// Win32 focus-routing behave identically to a human click. That's
/// why the stress test uses this rather than eval'ing `.click()`.
pub async fn click_element(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ClickElementReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
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

    // Inject helpers so __amq_centroid_of is available.
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

    // Ask the helper for the element's centroid in viewport coords.
    let selector_js = serde_json::to_string(&req.selector).unwrap_or_else(|_| "\"\"".into());
    let centroid_expr = format!("__amq_centroid_of({selector_js})");
    let cent_reply = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": centroid_expr,
                "returnByValue": true,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP centroid query: {e}")));
        }
    };

    let cent = cent_reply
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if cent.is_null() {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(format!(
            "selector {:?} matched no element",
            req.selector
        )));
    }

    let x = cent.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = cent.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // Dispatch mousePressed + mouseReleased. `buttons: 1` = left
    // primary; `button: "left"` is the CDP enum.
    for event_type in ["mousePressed", "mouseReleased"] {
        if let Err(e) = cdp
            .call(
                "Input.dispatchMouseEvent",
                json!({
                    "type": event_type,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "buttons": 1,
                    "clickCount": 1,
                }),
            )
            .await
        {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP dispatchMouseEvent {event_type}: {e}")));
        }
    }

    let _ = cdp.close().await;
    ok_body(ApiResponse::ok(AckData::new()))
}

/// `POST /agentmux/browser/focus_element` — call `.focus()` on the
/// first matching element. Does not synthesize a mouse event; use
/// `click_element` when you want the full mouse-gesture semantics.
pub async fn focus_element(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FocusElementReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
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

    let selector_js = serde_json::to_string(&req.selector).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        "(() => {{ const e = document.querySelector({sel}); \
            if (!e) return false; e.focus(); return true; }})()",
        sel = selector_js,
    );

    let reply = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": script,
                "returnByValue": true,
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP focus_element: {e}")));
        }
    };
    let _ = cdp.close().await;

    let got = reply
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !got {
        return ok_body(ApiResponse::err(format!(
            "selector {:?} matched no element",
            req.selector
        )));
    }
    ok_body(ApiResponse::ok(AckData::new()))
}

/// `POST /agentmux/browser/dispatch_key` — send text or a named key
/// to whatever has focus in the pane. Optionally focuses a selector
/// first.
pub async fn dispatch_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<DispatchKeyReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    // Exactly one of text / key must be set.
    if req.text.is_some() == req.key.is_some() {
        return ok_body(ApiResponse::err(
            "dispatch_key requires exactly one of `text` or `key`".to_string(),
        ));
    }

    let mut cdp = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    // Optionally focus a selector first.
    if let Some(sel) = &req.selector {
        let sel_js = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
        let script = format!(
            "(() => {{ const e = document.querySelector({sel}); \
                if (!e) return false; e.focus(); return true; }})()"
        );
        let reply = cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true }),
            )
            .await;
        let ok_focus = reply
            .as_ref()
            .ok()
            .and_then(|v| v.get("result"))
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !ok_focus {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!(
                "dispatch_key: selector {sel:?} matched no element"
            )));
        }
    }

    if let Some(text) = &req.text {
        // Input.insertText is atomic and handles IME / composition
        // correctly. Preferred over key-by-key dispatch for strings.
        if let Err(e) = cdp
            .call("Input.insertText", json!({ "text": text }))
            .await
        {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(format!("CDP Input.insertText: {e}")));
        }
    } else if let Some(key) = &req.key {
        let (key_name, code, windows_virtual_key_code) = match key.as_str() {
            "Enter" => ("Enter", "Enter", 13),
            "Tab" => ("Tab", "Tab", 9),
            "Escape" => ("Escape", "Escape", 27),
            "Backspace" => ("Backspace", "Backspace", 8),
            "ArrowUp" => ("ArrowUp", "ArrowUp", 38),
            "ArrowDown" => ("ArrowDown", "ArrowDown", 40),
            "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37),
            "ArrowRight" => ("ArrowRight", "ArrowRight", 39),
            "Space" => (" ", "Space", 32),
            other => {
                let _ = cdp.close().await;
                return ok_body(ApiResponse::err(format!(
                    "dispatch_key: unsupported key name {other:?} — supported: \
                     Enter, Tab, Escape, Backspace, ArrowUp/Down/Left/Right, Space"
                )));
            }
        };

        for event_type in ["keyDown", "keyUp"] {
            if let Err(e) = cdp
                .call(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": event_type,
                        "key": key_name,
                        "code": code,
                        "windowsVirtualKeyCode": windows_virtual_key_code,
                        "nativeVirtualKeyCode": windows_virtual_key_code,
                    }),
                )
                .await
            {
                let _ = cdp.close().await;
                return ok_body(ApiResponse::err(format!(
                    "CDP Input.dispatchKeyEvent {event_type}: {e}"
                )));
            }
        }
    }

    let _ = cdp.close().await;
    ok_body(ApiResponse::ok(AckData::new()))
}

/// `POST /agentmux/browser/navigate` — navigate the pane to a new
/// URL via CDP `Page.navigate`. (We could call
/// `BrowserPaneManager::navigate` directly, but routing through CDP
/// keeps the resolver's URL cache consistent — a subsequent request
/// will re-probe `/json` if the target id changes.)
pub async fn navigate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<NavigateReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
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

    let reply = cdp.call("Page.navigate", json!({ "url": req.url })).await;
    let _ = cdp.close().await;

    // After navigation the target's URL changes — forget the cache
    // entry so the next resolver probe re-matches against the new URL.
    state.browser_api.target_cache.forget(&req.block_id);

    match reply {
        Ok(_) => ok_body(ApiResponse::ok(AckData::new())),
        Err(e) => ok_body(ApiResponse::err(format!("CDP Page.navigate: {e}"))),
    }
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
