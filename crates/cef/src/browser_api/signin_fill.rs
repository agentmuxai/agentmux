// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filling a saved sign-in the person chose
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3).
//!
//! `/agentmux/browser/signin_fill`, for srv only: srv sends the user name and
//! password of the `login` entry the person picked from the pane's "Sign in as"
//! menu (operator-only, so no agent's `UI` click reaches it), and the frame
//! origin it is saved for. The host finds a sign-in
//! form in a frame of exactly that origin (one visible `type=password` field in
//! its form or field group, and its user name field), types the user name, finds
//! the password field again (a page can swap it meanwhile), types the password,
//! and marks both fields (`data-agentmux-vault`) so every snapshot, read-back
//! and screenshot treats them as secret. The answer never holds a value.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{call_on, fail, unauthorized, PREPARE_FILL, SET_VALUE};
use super::cdp::CdpSession;
use super::types::ApiResponse;
use super::vault_fill::TAINT_ATTR;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SigninFillReq {
    pub block_id: String,
    /// The exact origin the sign-in is saved for: only a frame of this origin is filled.
    pub origin: String,
    pub username: String,
    /// Held only for the length of this request.
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SigninFillData {
    pub filled: bool,
    /// The form had a user name field and it was filled.
    pub username_filled: bool,
}

/// A field a person can see and type into: shown, not hidden by style,
/// `aria-hidden` or a clip, not transparent, not disabled or read-only. A
/// sign-in is never typed into a field the person can't see (§10.2); that it
/// is on screen and on top is checked once it is scrolled to ([`ON_TOP`]).
const VISIBLE: &str = r#"(el) => {
  if (!el || !el.isConnected || el.disabled || el.readOnly) return false;
  const r = el.getBoundingClientRect();
  if (r.width < 2 || r.height < 2) return false;
  for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
    const s = getComputedStyle(n);
    if (s.visibility === "hidden" || s.display === "none" || Number(s.opacity) === 0) return false;
    if (n.getAttribute("aria-hidden") === "true") return false;
    if (s.clip && s.clip !== "auto") return false;
  }
  const own = getComputedStyle(el).clipPath;
  return !own || own === "none";
}"#;

/// Whether `this`, scrolled to, is on screen and is what a click at its centre
/// would reach: nothing of the page's is drawn over it.
const ON_TOP: &str = r#"function () {
  const r = this.getBoundingClientRect();
  const x = r.left + r.width / 2, y = r.top + r.height / 2;
  const w = this.ownerDocument.defaultView;
  if (!w || x < 0 || y < 0 || x > w.innerWidth || y > w.innerHeight) return false;
  const root = this.getRootNode();
  const at = (root && typeof root.elementFromPoint === "function" ? root : this.ownerDocument).elementFromPoint(x, y);
  return at === this;
}"#;

/// The sign-in form's password field: one visible `type=password` field in its
/// form or, outside a form, in the page, and not one for a new password.
fn find_password() -> String {
    format!(
        r#"(() => {{
  const visible = {VISIBLE};
  const shown = (root) => [...root.querySelectorAll("input[type=password]")].filter(visible);
  for (const p of shown(document)) {{
    if ((p.getAttribute("autocomplete") || "").toLowerCase().includes("new-password")) continue;
    if (shown(p.form || document).length === 1) return p;
  }}
  return null;
}})()"#
    )
}

/// The user name field that goes with this password field: the one marked as
/// such, else the last visible text or email field before it.
fn user_for() -> String {
    format!(
        r#"function () {{
  const visible = {VISIBLE};
  const inputs = [...(this.form || document).querySelectorAll("input")].filter(visible);
  const typed = (i) => ["text", "email", "tel", ""].includes(i.type);
  const marked = inputs.find((i) => i !== this && /\b(username|email)\b/.test((i.getAttribute("autocomplete") || "").toLowerCase()));
  if (marked) return marked;
  return inputs.slice(0, inputs.indexOf(this)).reverse().find(typed) || null;
}}"#
    )
}

/// Whether `this` is still a visible password field of a sign-in form.
fn still_password() -> String {
    format!(r#"function () {{ return this.type === "password" && ({VISIBLE})(this); }}"#)
}

/// Checks `this` is still in a document of the origin and on top, marks it as
/// holding a Vault value, and sets its value, in one synchronous call on the
/// field itself: the page gets no turn between the checks and the value, and
/// nothing is typed wherever the focus happens to be (a page could move it, even
/// into another site's frame, between a focus check and `Input.insertText`).
fn check_and_set() -> String {
    format!(
        r#"function (origin, v, attr) {{
  if (!({IN_ORIGIN}).call(this, origin)) return "origin";
  if (!({ON_TOP}).call(this)) return "covered";
  this.setAttribute(attr, "1");
  ({SET_VALUE}).call(this, v);
  return this.value === v ? "ok" : "refused";
}}"#
    )
}

/// Whether `this` is still in a document of `origin`. The frame tree was read
/// before the awaited calls: a frame that navigated since holds another site's
/// document, and the old one's fields are gone from it.
const IN_ORIGIN: &str = r#"function (origin) {
  const w = this.ownerDocument && this.ownerDocument.defaultView;
  return this.isConnected && !!w && w.location.origin === origin;
}"#;

/// `POST /agentmux/browser/signin_fill`.
pub async fn signin_fill_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<SigninFillReq>) -> (StatusCode, Json<ApiResponse<SigninFillData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "signin-fill").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = fill(&mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

/// The frames of `tree` whose origin is exactly `origin`.
fn frames_of_origin(tree: &Value, origin: &str) -> Vec<String> {
    fn walk(node: &Value, origin: &str, out: &mut Vec<String>) {
        let frame = node.get("frame");
        let text = |k: &str| frame.and_then(|f| f.get(k)).and_then(Value::as_str);
        if let (Some(id), Some(o)) = (text("id"), text("securityOrigin")) {
            if o == origin {
                out.push(id.to_string());
            }
        }
        for child in node.get("childFrames").and_then(Value::as_array).into_iter().flatten() {
            walk(child, origin, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree.get("frameTree") {
        walk(root, origin, &mut out);
    }
    out
}

async fn fill(root: &mut CdpSession, req: &SigninFillReq) -> Result<SigninFillData, String> {
    // The page's own frames first, then each cross-site iframe through its session.
    let tree = root.call("Page.getFrameTree", json!({})).await?;
    let mut sessions: Vec<(Option<String>, Vec<String>)> = vec![(None, frames_of_origin(&tree, &req.origin))];
    let targets = root.attach_frames().await;
    let seen: HashSet<String> = sessions[0].1.iter().cloned().collect();
    for t in &targets {
        let mut on = root.in_session(Some(&t.session_id));
        let Ok(tree) = on.call("Page.getFrameTree", json!({})).await else { continue };
        let frames: Vec<String> = frames_of_origin(&tree, &req.origin).into_iter().filter(|f| !seen.contains(f)).collect();
        sessions.push((Some(t.session_id.clone()), frames));
    }
    for (session, frames) in sessions {
        let mut cdp = root.in_session(session.as_deref());
        for frame in frames {
            if let Some(done) = fill_frame(&mut cdp, &frame, req).await? {
                return Ok(done);
            }
        }
    }
    Err(format!("no sign-in form for {} is on the page now, so nothing was typed", req.origin))
}

/// Fill the sign-in form in `frame`, if it has one: `None` when it has none.
async fn fill_frame(cdp: &mut CdpSession, frame: &str, req: &SigninFillReq) -> Result<Option<SigninFillData>, String> {
    let ctx = super::act::isolated_context(cdp, Some(frame)).await?;
    let Some(password) = object(cdp, ctx, &find_password()).await? else { return Ok(None) };
    let user = call_on_object(cdp, &password, &user_for()).await?;
    let username_filled = match &user {
        Some(user) if !req.username.is_empty() => {
            type_into(cdp, user, &req.username, &req.origin).await?;
            true
        }
        _ => false,
    };
    // Typing the user name ran the page's handlers: the password field must
    // still be there and still a password field, or it is found again.
    let password = if call_on(cdp, &password, &still_password(), vec![]).await?.as_bool() == Some(true) {
        password
    } else {
        object(cdp, ctx, &find_password()).await?.ok_or("the password field left the page after the user name was typed, so the password wasn't")?
    };
    type_into(cdp, &password, &req.password, &req.origin).await?;
    Ok(Some(SigninFillData { filled: true, username_filled }))
}

/// Evaluate `expression` in context `ctx` and return the element it gives, if any.
async fn object(cdp: &mut CdpSession, ctx: i64, expression: &str) -> Result<Option<String>, String> {
    let v = cdp.call("Runtime.evaluate", json!({ "expression": expression, "contextId": ctx, "returnByValue": false })).await?;
    Ok(v.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string))
}

/// Call `func` on element `obj` and return the element it gives, if any.
async fn call_on_object(cdp: &mut CdpSession, obj: &str, func: &str) -> Result<Option<String>, String> {
    let v = cdp.call("Runtime.callFunctionOn", json!({ "objectId": obj, "functionDeclaration": func, "returnByValue": false })).await?;
    Ok(v.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string))
}

/// Focus `obj` and scroll it into view (as the person would), then check it and
/// set `text` as its value in one call ([`check_and_set`]). Checked to be in a
/// document of `origin` before focusing too.
async fn type_into(cdp: &mut CdpSession, obj: &str, text: &str, origin: &str) -> Result<(), String> {
    let left = || format!("the page left {origin}, so nothing more was typed");
    if call_on(cdp, obj, IN_ORIGIN, vec![json!(origin)]).await?.as_bool() != Some(true) {
        return Err(left());
    }
    let prepared = call_on(cdp, obj, PREPARE_FILL, vec![]).await?;
    if let Some(e) = prepared.get("error").and_then(Value::as_str) {
        return Err(e.to_string());
    }
    match call_on(cdp, obj, &check_and_set(), vec![json!(origin), json!(text), json!(TAINT_ATTR)]).await?.as_str() {
        Some("ok") => Ok(()),
        Some("origin") => Err(left()),
        Some("covered") => Err("the field is off screen or something is drawn over it, so nothing more was typed".to_string()),
        _ => Err("the field didn't take the value, so nothing more was typed".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_frames_of_the_exact_origin_are_filled() {
        let tree = json!({ "frameTree": {
            "frame": { "id": "main", "securityOrigin": "https://accounts.example.com" },
            "childFrames": [
                { "frame": { "id": "sub", "securityOrigin": "https://login.accounts.example.com" } },
                { "frame": { "id": "plain", "securityOrigin": "http://accounts.example.com" } },
                { "frame": { "id": "again", "securityOrigin": "https://accounts.example.com" } }
            ]
        }});
        assert_eq!(frames_of_origin(&tree, "https://accounts.example.com"), ["main", "again"]);
        assert!(frames_of_origin(&tree, "https://other.example.com").is_empty());
    }

    #[test]
    fn the_scripts_type_only_into_visible_sign_in_fields() {
        let find = find_password();
        for needle in ["input[type=password]", "new-password", "=== 1", "visible"] {
            assert!(find.contains(needle), "{needle}");
        }
        for needle in ["opacity", "visibility", "readOnly", "disabled", "getBoundingClientRect", "aria-hidden", "s.clip", "clipPath"] {
            assert!(VISIBLE.contains(needle), "{needle}");
        }
        for needle in ["innerWidth", "innerHeight", "elementFromPoint(x, y)", "at === this"] {
            assert!(ON_TOP.contains(needle), "{needle}");
        }
        assert!(user_for().contains("username|email"));
        assert!(still_password().contains(r#"this.type === "password""#));
        for needle in ["isConnected", "defaultView", "location.origin === origin"] {
            assert!(IN_ORIGIN.contains(needle), "{needle}");
        }
    }

    #[test]
    fn the_value_is_set_on_the_checked_field_not_typed_where_the_focus_is() {
        let set = check_and_set();
        // In the order: origin, on top, marked, then the value, all on `this`.
        let at = |needle: &str| set.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at("location.origin === origin") < at("elementFromPoint(x, y)"));
        assert!(at("elementFromPoint(x, y)") < at("this.setAttribute(attr"));
        assert!(at("this.setAttribute(attr") < at("d.set.call(this, v)"));
        assert!(!set.contains("activeElement"), "where the focus is doesn't decide where the value goes");
    }
}
