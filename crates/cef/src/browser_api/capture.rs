// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a secret a page shows once, for the Vault
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! Two routes, for srv only:
//! - `/agentmux/browser/capture_value` reads the element a reference names (an
//!   input's value, otherwise its text's one secret) and answers with the value and
//!   the origin of the frame it is in, taken from the host's own frame map, never
//!   from the page. srv stores it; the agent never sees it.
//! - `/agentmux/browser/capture_mark`, once srv has stored it, marks the element as
//!   holding a Vault value (`data-agentmux-vault`, on the element or a text node's
//!   parent). A capture srv refused leaves no mark.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{call_on, fail, isolated_context, resolve, unauthorized};
use super::types::ApiResponse;
use super::vault_fill::{locate, TAINT_ATTR};
use crate::state::AppState;

/// The longest value a capture takes: a key or a token, not a page.
pub const MAX_VALUE: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
pub struct CaptureReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
}

#[derive(Debug, Serialize)]
pub struct Captured {
    pub value: String,
    /// The origin of the frame the element is in.
    pub origin: String,
    /// The page's URL.
    pub page: String,
}

/// The element's value or text, trimmed, and whether it is a field (whose whole
/// value is the secret). A text node is read whole.
const READ: &str = r#"function () {
  const el = this.nodeType === 1 ? this : this.parentElement;
  const field = !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el === this;
  const v = field ? String(this.value == null ? "" : this.value) : String(this.textContent || "");
  return { value: v.trim(), field };
}"#;

/// Marks the element (a text node's parent) as holding a Vault value: only once
/// the value is known to be saved, so a refused capture leaves no mark.
const MARK: &str = r#"function (attr) {
  const el = this.nodeType === 1 ? this : this.parentElement;
  if (el) el.setAttribute(attr, "1");
  return true;
}"#;

/// `POST /agentmux/browser/capture_value`.
pub async fn capture_value_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CaptureReq>,
) -> (StatusCode, Json<ApiResponse<Captured>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-capture").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = capture(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

async fn capture(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &CaptureReq) -> Result<Captured, String> {
    let (located, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let cdp = &mut in_frame;
    let ctx = isolated_context(cdp, frame.as_ref().map(|f| f.frame_id.as_str())).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    let read = call_on(cdp, &obj, READ, vec![]).await?;
    let Some(text) = read.get("value").and_then(Value::as_str) else {
        return Err(format!("{} has no text to save", req.ref_));
    };
    let field = read.get("field").and_then(Value::as_bool).unwrap_or(false);
    let value = pick(text, field, &req.ref_)?;
    Ok(Captured { value, origin: located.origin, page: located.page })
}

/// `POST /agentmux/browser/capture_mark`: the element's value is in the Vault now.
pub async fn capture_mark_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CaptureReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-capture-mark").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = mark(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(()) => (StatusCode::OK, Json(ApiResponse::ok(json!({ "marked": true })))),
        Err(e) => fail(e),
    }
}

async fn mark(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &CaptureReq) -> Result<(), String> {
    let (_, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let cdp = &mut in_frame;
    let ctx = isolated_context(cdp, frame.as_ref().map(|f| f.frame_id.as_str())).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    call_on(cdp, &obj, MARK, vec![json!(TAINT_ATTR)]).await?;
    Ok(())
}

/// The secret in what was read: a field's whole value; in text, its one token with
/// a one-time secret's shape (text like "Value: <secret>" is more than the secret).
fn pick(text: &str, field: bool, ref_: &str) -> Result<String, String> {
    check(text, ref_)?;
    if field || !text.chars().any(char::is_whitespace) {
        return Ok(text.to_string());
    }
    let tokens: Vec<&str> = text.split_whitespace().filter(|t| agentmux_common::shown_once::looks_shown_once(t)).collect();
    match tokens.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("{ref_} holds text, not one secret: capture the element that holds only the value (its own line in a scoped snapshot)")),
        _ => Err(format!("{ref_} holds {} secrets: capture the element that holds only the one you want", tokens.len())),
    }
}

/// A value worth saving: not empty, and not a page's worth of text.
fn check(value: &str, ref_: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{ref_} is empty: there is nothing to save"));
    }
    if value.len() > MAX_VALUE {
        return Err(format!("{ref_} holds more than {} KB, more than a key or token: name the element that holds only the value", MAX_VALUE / 1024));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_around_a_secret_is_left_out_and_a_fields_value_is_kept_whole() {
        let secret = "Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123";
        assert_eq!(pick(&format!("Value {secret}"), false, "e5").unwrap(), secret);
        assert_eq!(pick(secret, false, "e5").unwrap(), secret);
        let codes = "recovery codes:\nabcd-1234 efgh-5678\nijkl-9012";
        assert_eq!(pick(codes, true, "e8").unwrap(), codes, "a field's value is the secret, spaces, lines and all");
        let none = pick("Copy this value now please", false, "e5").unwrap_err();
        assert!(none.contains("not one secret"), "{none}");
        let two = pick(&format!("{secret} and {secret}"), false, "e5").unwrap_err();
        assert!(two.contains("holds 2 secrets") && !two.contains(secret), "{two}");
    }

    #[test]
    fn an_empty_or_page_sized_element_is_refused_without_quoting_it() {
        assert!(check("Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123", "e3").is_ok());
        assert!(check("", "e3").unwrap_err().contains("nothing to save"));
        let page = "word ".repeat(5000);
        let err = check(&page, "e3").unwrap_err();
        assert!(err.contains("more than 16 KB") && !err.contains("word"), "{err}");
    }
}
