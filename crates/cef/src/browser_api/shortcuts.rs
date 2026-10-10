// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shortcut table's App API routes: `list_shortcuts`, `run_command` and
//! `press_keys` (docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md
//! §4). srv sends the caller's own, verified block; these resolve the window
//! whose page holds it and call the frontend's `window.__agentmux_shortcuts`
//! there (frontend/app/keybindings/app-api.ts). No script from the request
//! runs: the only request values that reach the page are serialized as JSON
//! string arguments.
//!
//! `press_keys` sends real key events through the DevTools protocol, so they
//! reach the page as the user's keys would, after the OS and (on macOS) the
//! menu bar: the plan's layer L2. The page decides what may be pressed and
//! with which modifiers; this module only sends what it planned.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::cdp::CdpSession;
use super::routes::authorized_pub as authorized;
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListReq {
    pub block_id: String,
}

#[derive(Deserialize)]
pub struct RunReq {
    pub block_id: String,
    pub command: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Deserialize)]
pub struct PressReq {
    pub block_id: String,
    pub keys: String,
    #[serde(default)]
    pub target: Option<String>,
}

type Reply = (StatusCode, Json<ApiResponse<Value>>);

fn unauthorized() -> Reply {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse::err("unauthorized: missing or invalid bearer token")),
    )
}

fn reply(r: Result<Value, String>) -> Reply {
    (
        StatusCode::OK,
        Json(match r {
            Ok(v) => ApiResponse::ok(v),
            Err(e) => ApiResponse::err(e),
        }),
    )
}

/// A CDP session on the AgentMux window that holds `block_id`. A dedicated
/// browser pane is a page of its own, with no shortcut table in it.
async fn open_app_window(state: &Arc<AppState>, block_id: &str) -> Result<CdpSession, String> {
    let resolved = state.browser_api.target_cache.resolve(state, block_id).await?;
    if !resolved.scope_to_block {
        return Err(format!(
            "pane {block_id:?} is a browser pane, which has no AgentMux shortcuts: \
             call from your own agent pane"
        ));
    }
    Ok(CdpSession::attach(state, &resolved.label))
}

/// Evaluates `expr` in the page and returns its value, or the exception as
/// an error.
async fn eval(cdp: &mut CdpSession, expr: &str) -> Result<Value, String> {
    let v = cdp
        .call("Runtime.evaluate", json!({ "expression": expr, "returnByValue": true }))
        .await
        .map_err(|e| format!("CDP eval: {e}"))?;
    if let Some(exc) = v.get("exceptionDetails") {
        let msg = exc
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(|d| d.as_str())
            .or_else(|| exc.get("text").and_then(|t| t.as_str()))
            .unwrap_or("unknown exception");
        return Err(format!("the page threw: {msg}"));
    }
    Ok(v.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(Value::Null))
}

/// `window.__agentmux_shortcuts.<call>`, or a clear error on a frontend
/// that predates it.
fn api_call(call: &str) -> String {
    format!(
        "(() => {{ const s = window.__agentmux_shortcuts; \
         if (!s) throw new Error('this window has no shortcut API (a build from before it)'); \
         return s.{call}; }})()"
    )
}

/// A JSON string literal for `s`, safe to splice into an expression.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn js_opt(s: Option<&str>) -> String {
    s.map(js_str).unwrap_or_else(|| "undefined".into())
}

async fn with_window<F, Fut>(state: &Arc<AppState>, block_id: &str, f: F) -> Result<Value, String>
where
    F: FnOnce(CdpSession) -> Fut,
    Fut: std::future::Future<Output = (CdpSession, Result<Value, String>)>,
{
    let cdp = open_app_window(state, block_id).await?;
    let (cdp, result) = f(cdp).await;
    let _ = cdp.close().await;
    result
}

/// `POST /agentmux/browser/list_shortcuts`
pub async fn list_shortcuts(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<ListReq>) -> Reply {
    if !authorized(&headers, &state.ipc_token) {
        return unauthorized();
    }
    reply(
        with_window(&state, &req.block_id, |mut cdp| async move {
            let r = eval(&mut cdp, &api_call("list()")).await;
            (cdp, r.map(|list| json!({ "shortcuts": list })))
        })
        .await,
    )
}

/// `POST /agentmux/browser/run_command`
pub async fn run_command(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<RunReq>) -> Reply {
    if !authorized(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let call = format!("run({}, {})", js_str(&req.command), js_opt(req.target.as_deref()));
    reply(
        with_window(&state, &req.block_id, |mut cdp| async move {
            let r = eval(&mut cdp, &api_call(&call)).await;
            (cdp, r)
        })
        .await,
    )
}

/// `POST /agentmux/browser/press_keys`
pub async fn press_keys(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<PressReq>) -> Reply {
    if !authorized(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let plan_call = format!("plan({}, {})", js_str(&req.keys), js_opt(req.target.as_deref()));
    reply(
        with_window(&state, &req.block_id, |mut cdp| async move {
            let r = press(&mut cdp, &plan_call).await;
            (cdp, r)
        })
        .await,
    )
}

async fn press(cdp: &mut CdpSession, plan_call: &str) -> Result<Value, String> {
    let plan = eval(cdp, &api_call(plan_call)).await?;
    if let Some(reason) = plan.get("reason").and_then(|r| r.as_str()) {
        return Ok(json!({ "sent": false, "reason": reason }));
    }
    let events = plan.get("events").and_then(|e| e.as_array()).cloned().unwrap_or_default();
    if events.is_empty() {
        return Err("the page planned no key events".into());
    }
    // The page's clock, so `last().at` compares with it.
    let before = eval(cdp, "Date.now()").await?.as_f64().unwrap_or(0.0);
    for ev in &events {
        for params in key_events(ev) {
            cdp.call("Input.dispatchKeyEvent", params)
                .await
                .map_err(|e| format!("CDP Input.dispatchKeyEvent: {e}"))?;
        }
    }
    // A handler that runs on a later task (a chord's timer, a pane's effect)
    // gets a moment to note what it resolved.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let last = eval(cdp, &api_call("last()")).await?;
    let resolved = match last.get("at").and_then(|a| a.as_f64()) {
        Some(at) if at >= before => json!({ "command": last.get("command"), "by": last.get("by") }),
        _ => Value::Null,
    };
    Ok(json!({
        "sent": true,
        "modifiers": plan.get("modifiers"),
        "candidates": plan.get("commands"),
        "resolved": resolved,
    }))
}

/// The CDP calls for one planned key: down then up, with its modifiers.
/// `rawKeyDown` (no `text`), so a shortcut never types its letter into a
/// focused text box.
fn key_events(ev: &Value) -> Vec<Value> {
    let key = ev.get("key").cloned().unwrap_or(Value::Null);
    let code = ev.get("code").cloned().unwrap_or(Value::Null);
    let vk = ev.get("keyCode").and_then(|k| k.as_i64()).unwrap_or(0);
    let modifiers = ev.get("modifiers").and_then(|m| m.as_i64()).unwrap_or(0);
    ["rawKeyDown", "keyUp"]
        .iter()
        .map(|t| {
            json!({
                "type": t,
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": vk,
                "modifiers": modifiers,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_values_reach_the_page_only_as_string_literals() {
        let call = format!("run({}, {})", js_str("x'); alert(1); ('"), js_opt(None));
        assert_eq!(call, r#"run("x'); alert(1); ('", undefined)"#);
        assert_eq!(js_str("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(js_opt(Some("blk")), r#""blk""#);
    }

    #[test]
    fn a_key_is_sent_down_then_up_with_its_modifiers_and_no_text() {
        let evs = key_events(&json!({ "key": "d", "code": "KeyD", "keyCode": 68, "modifiers": 4 }));
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0]["type"], "rawKeyDown");
        assert_eq!(evs[1]["type"], "keyUp");
        for e in &evs {
            assert_eq!(e["modifiers"], 4);
            assert_eq!(e["windowsVirtualKeyCode"], 68);
            assert!(e.get("text").is_none());
        }
    }
}
