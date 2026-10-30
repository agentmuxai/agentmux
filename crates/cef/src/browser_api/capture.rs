// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a secret a page shows once, for the Vault
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! `/agentmux/browser/capture_value`, for srv only: reads the element a reference
//! names (an input's value, otherwise its text, trimmed), marks it as holding a
//! Vault value (`data-agentmux-vault`, on the element or a text node's parent), and
//! answers with the value and the origin of the frame it is in, taken from the
//! host's own frame map, never from the page. srv stores it; the agent never sees
//! it.

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

/// The element's value or text, trimmed, after marking it as a Vault value.
/// A text node is read whole and its parent element marked.
const READ_AND_MARK: &str = r#"function (attr) {
  const el = this.nodeType === 1 ? this : this.parentElement;
  const isField = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el === this;
  const v = isField ? String(this.value == null ? "" : this.value) : String(this.textContent || "");
  if (el) el.setAttribute(attr, "1");
  return v.trim();
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
    let read = call_on(cdp, &obj, READ_AND_MARK, vec![json!(TAINT_ATTR)]).await?;
    let value = match read {
        Value::String(s) => s,
        _ => return Err(format!("{} has no text to save", req.ref_)),
    };
    check(&value, &req.ref_)?;
    Ok(Captured { value, origin: located.origin, page: located.page })
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
    fn an_empty_or_page_sized_element_is_refused_without_quoting_it() {
        assert!(check("Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123", "e3").is_ok());
        assert!(check("", "e3").unwrap_err().contains("nothing to save"));
        let page = "word ".repeat(5000);
        let err = check(&page, "e3").unwrap_err();
        assert!(err.contains("more than 16 KB") && !err.contains("word"), "{err}");
    }
}
