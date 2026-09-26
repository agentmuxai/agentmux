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
    FocusElementReq, FocusInfoData, FocusInfoReq, HistoryReq, NavigateReq, QueryData, QueryReq,
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

    // Resolve block_id → the browser whose page holds it.
    let resolved = match state
        .browser_api
        .target_cache
        .resolve(&state, &req.block_id)
        .await
    {
        Ok(t) => t,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    // In-process CDP session on that browser (see `super::cdp`).
    let mut cdp = CdpSession::attach(&state, &resolved.label);

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
    // handles quote-escaping safely. `scope_to_block` selects whether the
    // query is scoped to this block's own [data-blockid] subtree (a
    // shared window page) or unscoped (a dedicated browser pane, which IS
    // already this block's own isolated page).
    let selector_js = serde_json::to_string(&req.selector)
        .unwrap_or_else(|_| "\"\"".to_string());
    let block_id_js = if resolved.scope_to_block {
        serde_json::to_string(&req.block_id).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };
    let call_expr = format!(
        "__amq_query({sel}, {lim}, {bid})",
        sel = selector_js,
        lim = req.limit.unwrap_or(0),
        bid = block_id_js,
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
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

    // Scoped to this block's own [data-blockid] subtree on a shared page —
    // see resolver::ResolvedTarget::scope_to_block — same as query/click.
    // NOT scoped for a dedicated browser-pane target, which already IS
    // this block's own isolated page. (reagent P0, PR #3445: this used to
    // report document.activeElement for the WHOLE resolved page
    // unconditionally, reasoned as safe only because focus_info wasn't yet
    // reachable by any agent — this PR's own BrowserFocusInfo tool is what
    // makes that reasoning stale.)
    let block_id_js = if scope_to_block {
        serde_json::to_string(&req.block_id).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };
    let eval_result = match cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": format!("__amq_focus_info({block_id_js})"),
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };
    if let Err(e) = reject_if_shared_target(scope_to_block, "eval", &req.block_id) {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(e));
    }

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

/// `POST /agentmux/browser/screenshot` — capture the pane's rendered
/// viewport (PNG by default, JPEG on request). Uses CDP
/// `Page.captureScreenshot`.
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    let format = match req.format.as_deref() {
        Some("jpeg") => "jpeg",
        _ => "png",
    };
    let mut params = json!({
        "format": format,
        "fromSurface": true,
    });
    if format == "jpeg" {
        params["quality"] = json!(req.quality.unwrap_or(80).min(100));
    }

    // Shared-page targets (main/pool/floating windows) hold other panes'
    // DOM too — clip the capture to this block's own [data-blockid] rect
    // so a screenshot can't read another pane's on-screen content. A
    // dedicated browser-pane target already IS this block's own page, so
    // no clip is needed (and __amq_rect_of would find nothing to clip to —
    // there's no [data-blockid] concept inside third-party page content).
    if scope_to_block {
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
        let block_id_js = serde_json::to_string(&req.block_id).unwrap_or_else(|_| "\"\"".to_string());
        let rect_expr = format!("__amq_rect_of({block_id_js})");
        let rect_reply = cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": rect_expr, "returnByValue": true }),
            )
            .await;
        let rect = rect_reply
            .ok()
            .and_then(|v| v.get("result").and_then(|r| r.get("value")).cloned())
            .filter(|v| !v.is_null());
        match rect {
            Some(r) => {
                let x = r.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = r.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let width = r.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let height = r.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
                params["clip"] = json!({ "x": x, "y": y, "width": width, "height": height, "scale": 1.0 });
            }
            None => {
                let _ = cdp.close().await;
                return ok_body(ApiResponse::err(format!(
                    "pane not found: no [data-blockid=\"{}\"] element on this page",
                    req.block_id
                )));
            }
        }
    }

    let cap = match cdp
        .call("Page.captureScreenshot", params)
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
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

    // Ask the helper for the element's centroid in viewport coords,
    // scoped to this block's own [data-blockid] subtree on shared pages
    // (see resolver::ResolvedTarget::scope_to_block) — an agent can only
    // ever click inside its own pane this way.
    let selector_js = serde_json::to_string(&req.selector).unwrap_or_else(|_| "\"\"".into());
    let block_id_js = if scope_to_block {
        serde_json::to_string(&req.block_id).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };
    let centroid_expr = format!("__amq_centroid_of({selector_js}, {block_id_js})");
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

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

    let selector_js = serde_json::to_string(&req.selector).unwrap_or_else(|_| "\"\"".into());
    let block_id_js = if scope_to_block {
        serde_json::to_string(&req.block_id).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };
    let script = format!("__amq_focus({selector_js}, {block_id_js})");

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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };

    // Helper is needed on every path below (selector-focus AND the
    // no-selector ownership check), so inject it unconditionally
    // up front rather than duplicating the injection in both branches.
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
    let block_id_js = if scope_to_block {
        serde_json::to_string(&req.block_id).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };

    if let Some(sel) = &req.selector {
        // Focus a specific, ownership-checked selector first.
        let sel_js = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
        let script = format!("__amq_focus({sel_js}, {block_id_js})");
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
    } else if scope_to_block {
        // No selector: keys/text land on whatever currently has focus on
        // the resolved page. For a dedicated browser pane that page
        // already IS this block's own content, so that's fine as-is. But
        // for a SHARED page (this branch), "whatever has focus" could be
        // a different pane entirely — verify ownership before dispatching
        // blind. (reagent P0, PR #3445: this branch didn't exist before;
        // an omitted selector against a shared target dispatched
        // unconditionally to the page's current focus, letting an agent
        // inject keystrokes into another pane's focused element.)
        let owned_expr = format!("__amq_focus_owned_by({block_id_js})");
        let reply = cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": owned_expr, "returnByValue": true }),
            )
            .await;
        let owned = reply
            .as_ref()
            .ok()
            .and_then(|v| v.get("result"))
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !owned {
            let _ = cdp.close().await;
            return ok_body(ApiResponse::err(
                "dispatch_key: no `selector` was given, and the element currently \
                 focused on this shared page isn't in your own pane (or nothing is \
                 focused there) — pass `selector` to focus an element in your own \
                 pane first."
                    .to_string(),
            ));
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

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };
    if let Err(e) = reject_if_shared_target(scope_to_block, "navigate", &req.block_id) {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(e));
    }

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

/// `POST /agentmux/browser/back` — walk the pane's history one step back.
/// Routed through CDP (`Page.goBack`) rather than
/// `BrowserPaneManager::go_back` to keep the resolver cache honest: the
/// target URL changes after the hop, so we invalidate the cache entry.
///
/// Returns ack-ok even when there's no prior history — CDP is a no-op
/// in that case, and agents should query `browser-pane-nav-state` events
/// (or `eval("location.href")`) to confirm what happened.
pub async fn back(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<HistoryReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };
    if let Err(e) = reject_if_shared_target(scope_to_block, "back", &req.block_id) {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(e));
    }

    // `Page.navigateToHistoryEntry` needs an entryId; simpler to call
    // Page.goBack which CDP exposes directly.
    let reply = cdp.call("Page.goBack", json!({})).await;
    let _ = cdp.close().await;

    state.browser_api.target_cache.forget(&req.block_id);

    match reply {
        Ok(_) => ok_body(ApiResponse::ok(AckData::new())),
        Err(e) => ok_body(ApiResponse::err(format!("CDP Page.goBack: {e}"))),
    }
}

/// `POST /agentmux/browser/forward` — walk the pane's history one step forward.
/// See `back` for the target-cache rationale.
pub async fn forward(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<HistoryReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };
    if let Err(e) = reject_if_shared_target(scope_to_block, "forward", &req.block_id) {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(e));
    }

    let reply = cdp.call("Page.goForward", json!({})).await;
    let _ = cdp.close().await;

    state.browser_api.target_cache.forget(&req.block_id);

    match reply {
        Ok(_) => ok_body(ApiResponse::ok(AckData::new())),
        Err(e) => ok_body(ApiResponse::err(format!("CDP Page.goForward: {e}"))),
    }
}

/// `POST /agentmux/browser/reload` — reload the current page. `ignore_cache`
/// (default false) maps to the CDP flag — true is the equivalent of Ctrl+F5
/// (bypass the http cache). The pane's current URL is preserved, so no
/// target-cache invalidation is needed.
pub async fn reload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<HistoryReq>,
) -> (StatusCode, Json<ApiResponse<AckData>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
        );
    }

    let (mut cdp, scope_to_block) = match open_cdp_for_block(&state, &req.block_id).await {
        Ok(c) => c,
        Err(e) => return ok_body(ApiResponse::err(e)),
    };
    if let Err(e) = reject_if_shared_target(scope_to_block, "reload", &req.block_id) {
        let _ = cdp.close().await;
        return ok_body(ApiResponse::err(e));
    }

    let reply = cdp
        .call("Page.reload", json!({ "ignoreCache": req.ignore_cache }))
        .await;
    let _ = cdp.close().await;

    match reply {
        Ok(_) => ok_body(ApiResponse::ok(AckData::new())),
        Err(e) => ok_body(ApiResponse::err(format!("CDP Page.reload: {e}"))),
    }
}

// ── shared helpers ──────────────────────────────────────────────────────

/// Returns the open CDP session plus whether the resolved target is a
/// SHARED page (true — caller must scope DOM lookups to this block's own
/// `[data-blockid]` subtree) or a dedicated browser-pane page (false —
/// already isolated, scoping would break third-party content that has no
/// `data-blockid` concept). See `resolver::ResolvedTarget`.
async fn open_cdp_for_block(
    state: &Arc<AppState>,
    block_id: &str,
) -> Result<(CdpSession, bool), String> {
    // No connect step, so nothing can go stale between resolve and use: the
    // session addresses the browser by its host-state label, and a
    // navigation-driven renderer swap keeps the same `Browser`. (The old
    // socket client re-resolved on connect failure because `/json` target
    // ids rotated on such swaps.)
    let resolved = state.browser_api.target_cache.resolve(state, block_id).await?;
    Ok((CdpSession::attach(state, &resolved.label), resolved.scope_to_block))
}

/// Rejects `eval`/`navigate`/`back`/`forward`/`reload` against a Path-2
/// (shared-window) target. Those CDP calls (`Runtime.evaluate`,
/// `Page.navigate`/`goBack`/`goForward`/`reload`) act on the resolved
/// page AS A WHOLE — there is no way to scope `Page.navigate` to "just
/// this block's subtree" the way `query`/`click_element` scope a DOM
/// lookup. For a Path 1 target (a dedicated `view: "browser"` pane) that
/// whole page already IS this block's own isolated content, so acting on
/// it is safe. For a Path 2 target (any other pane — agent/terminal/
/// editor/etc., a DOM node inside a page SHARED with the main/pool/
/// floating window's own chrome and every other pane in it) it is not:
/// `eval` would run arbitrary JS with access to the entire shared page,
/// and `navigate`/`back`/`forward`/`reload` would navigate or rewrite
/// history for that whole shared window out from under every other pane
/// in it, for a `block_id` that isn't even a browser pane. See
/// `resolver::ResolvedTarget::scope_to_block` and
/// `docs/specs/SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20.md` §2-3.1.
///
/// Deliberately sync/pure (no `CdpSession` param) so it's unit-testable
/// without a live CDP connection — callers close their own session on the
/// `Err` path, same as every other error branch in this file.
fn reject_if_shared_target(scope_to_block: bool, op: &str, block_id: &str) -> Result<(), String> {
    if scope_to_block {
        return Err(format!(
            "{op}: block {block_id:?} is not a dedicated browser pane. {op} acts on the \
             whole resolved page, which for this block is a page SHARED with other panes \
             and the app's own chrome, not an isolated page — only a `view: \"browser\"` \
             pane block is a safe target for this operation."
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::reject_if_shared_target;

    #[test]
    fn allows_a_dedicated_browser_pane_target() {
        assert!(reject_if_shared_target(false, "eval", "block-1").is_ok());
    }

    #[test]
    fn rejects_a_shared_window_target_with_a_clear_message() {
        let err = reject_if_shared_target(true, "navigate", "block-1")
            .expect_err("a shared-window target must be rejected");
        assert!(err.contains("navigate"), "message should name the rejected operation: {err}");
        assert!(err.contains("block-1"), "message should name the offending block: {err}");
        assert!(
            err.contains("not a dedicated browser pane"),
            "message should explain why: {err}"
        );
    }

    #[test]
    fn every_op_name_this_module_calls_it_with_appears_in_the_message() {
        for op in ["eval", "navigate", "back", "forward", "reload"] {
            let err = reject_if_shared_target(true, op, "b").unwrap_err();
            assert!(err.starts_with(op), "expected message to start with {op:?}: {err}");
        }
    }
}
