// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/agentmux/browser/capture_copy`, for srv only: the value a page's own Copy button copies goes into the
//! Vault without the system clipboard (#47629).
//!
//! The way around was to press the button and read the clipboard, which passes the value through the
//! system clipboard (and any clipboard history or sync the person has on) and needs a host terminal. Here
//! the host puts `scripts/copy_intercept.js` over the page's three ways of copying, clicks the button the
//! reference names (a real click, checked like `act`'s: one that commits something answers `needs_approval`
//! first, as `capture_download` does), takes the text the page tried to copy, and puts the page's own
//! functions back. The value goes to srv; the clipboard is never written.
//!
//! What it cannot take: a page that kept its own reference to the browser's clipboard function before the
//! intercept ran, a button inside a frame, or a button that copies nothing. Then nothing was taken, and the
//! answer says so and that the clipboard may have been written, so the agent can tell the person.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{check_click, click_element, fail, isolated_context, resolve, unauthorized, ClickCheck};
use super::cdp::CdpSession;
use super::types::ApiResponse;
use super::vault_fill::locate;
use crate::state::AppState;

const INTERCEPT: &str = include_str!("scripts/copy_intercept.js");
/// The longest value a copy gives: a key or a token, as for any capture.
const MAX_VALUE: usize = super::capture::MAX_VALUE;
/// How long it waits for the page to copy after the click, unless told otherwise, and the longest.
const DEFAULT_WAIT: Duration = Duration::from_secs(5);
const MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct CopyReq {
    pub block_id: String,
    /// The page's copy button.
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// The `needs_approval` the person approved, sent back with the approved click.
    #[serde(default)]
    pub approved_as: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Copied {
    pub value: String,
    /// The origin of the page the button is on.
    pub origin: String,
    pub page: String,
}

/// `POST /agentmux/browser/capture_copy`.
pub async fn capture_copy_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<CopyReq>) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-capture-copy").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = copy(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

/// The script as an expression that runs `mode` on the page's own `window`.
fn run(nonce: &str, mode: &str) -> String {
    format!("({INTERCEPT})(window, {}, {})", json!(nonce), json!(mode))
}

async fn eval(root: &mut CdpSession, expression: String) -> Result<Value, String> {
    let v = root.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true })).await?;
    if v.get("exceptionDetails").is_some() {
        return Err("the page would not let AgentMux watch its copy".into());
    }
    Ok(v.pointer("/result/value").cloned().unwrap_or(Value::Null))
}

/// `{"needs_approval": ...}` when the click waits for the person, else [`Copied`].
async fn copy(state: &Arc<AppState>, root: &mut CdpSession, req: &CopyReq) -> Result<Value, String> {
    let (located, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    if frame.is_some() {
        return Err(format!("{} is inside a frame, where AgentMux can't watch a copy: capture the text itself (by its ref, or by near, the label beside it)", req.ref_));
    }
    let mut on_page = root.in_session(None);
    let ctx = isolated_context(&mut on_page, None).await?;
    let obj = resolve(&mut on_page, backend, ctx, &req.ref_).await?;
    let approved = req.approved_as.is_some();
    if let ClickCheck::Ask(ask) = check_click(&mut on_page, &obj, &located.facts, "", &located.page, req.approved_as.as_ref(), approved).await? {
        return Ok(json!({ "needs_approval": ask }));
    }
    let nonce = format!("__am{}", uuid::Uuid::new_v4().simple());
    eval(root, run(&nonce, "install")).await?;
    let taken = click_and_wait(root, &mut on_page, &obj, &nonce, req).await;
    // The page's own functions go back whatever happened.
    let _ = eval(root, run(&nonce, "restore")).await;
    let value = taken?;
    serde_json::to_value(Copied { value, origin: located.origin, page: located.page }).map_err(|e| e.to_string())
}

async fn click_and_wait(root: &mut CdpSession, on_page: &mut CdpSession, obj: &str, nonce: &str, req: &CopyReq) -> Result<String, String> {
    click_element(on_page, root, None, obj).await?;
    let wait = req.timeout_ms.map(Duration::from_millis).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let got = eval(root, run(nonce, "read")).await?;
        if let Some(text) = got.get("value").and_then(Value::as_str) {
            return take(text, &req.ref_);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{} copied nothing AgentMux could take within {} seconds. The page may copy another way (it may have kept its own reference to the clipboard), in which case the system clipboard may hold the value: ask the person to clear it. Capture the text itself by its ref, or by near",
                req.ref_,
                wait.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The copied text, if it is something worth saving. The refusals never quote it.
fn take(text: &str, ref_: &str) -> Result<String, String> {
    let value = text.trim();
    if value.is_empty() {
        return Err(format!("{ref_} copied nothing: there is nothing to save"));
    }
    if value.len() > MAX_VALUE {
        return Err(format!("{ref_} copied more than {} KB, more than a key or token", MAX_VALUE / 1024));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_runs_as_one_expression_with_its_mode_and_nonce_quoted() {
        let e = run("__am1", "install");
        assert!(e.starts_with("(function copyIntercept") || e.contains("copyIntercept"), "{e}");
        assert!(e.ends_with("(window, \"__am1\", \"install\")"), "{e}");
    }

    #[test]
    fn a_copied_value_is_trimmed_and_an_empty_or_page_sized_one_is_refused_unquoted() {
        assert_eq!(take("  sk_live_abc  \n", "e3").unwrap(), "sk_live_abc");
        assert!(take("   ", "e3").unwrap_err().contains("nothing to save"));
        let big = "word ".repeat(5000);
        let err = take(&big, "e3").unwrap_err();
        assert!(err.contains("16 KB") && !err.contains("word"), "{err}");
    }
}
