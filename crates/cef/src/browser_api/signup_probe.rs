// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telling a sign-up form's new-password field, for an agent that names it by
//! reference (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §7.1).
//!
//! `/agentmux/browser/signup_probe`, for srv only. The person's own offer comes
//! from the page's `focusin` (`signin_notice.js`); an agent has no focus to
//! report, so it names the field by a reference from its snapshot. The host
//! finds that element, answers with the origin of the frame it is in (from the
//! host's own frame map, never the page), checks it is a visible password
//! field the page's rules take for a new password, and marks it with a fresh
//! token (`data-agentmux-signup`) that `signup_fill` fills by. A form that also
//! holds a `current-password` field is a password change (§7.3): the answer
//! says so and no token is made. The answer never holds a value.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::act::{call_on, fail, isolated_context, resolve, unauthorized};
use super::signin_fill::VISIBLE;
use super::signup_fill::SHOWN;
use super::types::ApiResponse;
use super::vault_fill::locate;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SignupProbeReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
}

#[derive(Debug, Serialize, Default)]
pub struct SignupProbeData {
    /// The origin of the frame the field is in.
    pub origin: String,
    /// The token `signup_fill` finds the field by; empty for a password change.
    pub field: String,
    /// The form also asks for the current password: a password change.
    pub change: bool,
    pub passwordrules: Option<String>,
    pub minlength: Option<usize>,
    pub maxlength: Option<usize>,
}

/// On the element: whether it is a visible field for a new password, by the
/// rules `signin_notice.js` offers a field by (the same words and the same
/// count of the fields the page shows), and if so a fresh token on it. Gives
/// `{ error }`, `{ change: true }`, or the token and the field's own limits.
fn probe_js() -> String {
    format!(
        r#"function () {{
  const visible = {VISIBLE};
  const shown = {SHOWN};
  const ac = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();
  const confirmWords = /confirm|repeat|retype|again|verify/i;
  const signupWords = /create (an |your )?account|sign ?up|register|join/i;
  const described = (el) => [...(el.labels || [])].map((l) => l.textContent).join(" ") + ` ${{el.name}} ${{el.id}} ${{ac(el)}} ${{el.placeholder || ""}}`;
  if (!(this instanceof HTMLInputElement) || this.type !== "password") return {{ error: "that element isn't a password field" }};
  if (!visible(this)) return {{ error: "that password field isn't visible, enabled and on top, so no password was made" }};
  const group = this.form || this.closest("form");
  const passwords = [...(group || document).querySelectorAll("input[type=password]")].filter(shown);
  if (passwords.some((i) => ac(i).includes("current-password"))) return {{ change: true }};
  const fresh =
    ac(this).includes("new-password") ||
    (passwords.length === 2 && this === passwords[0] && confirmWords.test(described(passwords[1]))) ||
    (passwords.length === 1 && !!group && [...group.querySelectorAll("button, input[type=submit]")].some((b) => signupWords.test(b.textContent || b.value || "")));
  if (!fresh) return {{ error: "that isn't a field for a new password: name the first password field of a sign-up form (one marked new-password, one whose next field is a confirmation, or the only one in a form with a Create account button)" }};
  const token = typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${{Date.now().toString(36)}}-${{Math.random().toString(36).slice(2)}}`;
  for (const other of document.querySelectorAll("[data-agentmux-signup]")) other.removeAttribute("data-agentmux-signup");
  this.setAttribute("data-agentmux-signup", token);
  const limit = (n) => (n > 0 ? n : undefined);
  return {{ field: token, passwordrules: this.getAttribute("passwordrules") || undefined, minlength: limit(this.minLength), maxlength: limit(this.maxLength) }};
}}"#
    )
}

/// `POST /agentmux/browser/signup_probe`.
pub async fn signup_probe_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<SignupProbeReq>) -> (StatusCode, Json<ApiResponse<SignupProbeData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "signup-probe").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = probe(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

async fn probe(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &SignupProbeReq) -> Result<SignupProbeData, String> {
    let (located, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let cdp = &mut in_frame;
    let ctx = isolated_context(cdp, frame.as_ref().map(|f| f.frame_id.as_str())).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    let found = call_on(cdp, &obj, &probe_js(), vec![]).await?;
    if let Some(e) = found.get("error").and_then(serde_json::Value::as_str) {
        return Err(format!("{}: {e}", req.ref_));
    }
    if located.origin.is_empty() {
        return Err("the field's site couldn't be told, so no password was made".to_string());
    }
    if found.get("change").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(SignupProbeData { origin: located.origin, change: true, ..Default::default() });
    }
    let text = |k: &str| found.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
    let number = |k: &str| found.get(k).and_then(serde_json::Value::as_u64).map(|n| n as usize);
    Ok(SignupProbeData {
        origin: located.origin,
        field: text("field").unwrap_or_default(),
        change: false,
        passwordrules: text("passwordrules"),
        minlength: number("minlength"),
        maxlength: number("maxlength"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_change_is_told_before_any_token_is_made() {
        let js = probe_js();
        let at = |needle: &str| js.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at(r#"ac(i).includes("current-password")"#) < at("randomUUID"), "a change form gets no token");
        assert!(at("visible(this)") < at("randomUUID"), "a hidden field gets no token");
        assert!(at("const fresh") < at("setAttribute"), "only a field for a new password is marked");
    }

    #[test]
    fn the_rules_are_the_listeners_and_the_fillers() {
        let js = probe_js();
        let script = crate::browser_pane::signin_notice::SCRIPT;
        for words in ["/confirm|repeat|retype|again|verify/i", "/create (an |your )?account|sign ?up|register|join/i"] {
            assert!(script.contains(words) && js.contains(words), "{words}");
        }
        assert!(js.contains(".filter(shown)") && js.contains(r#"setAttribute("data-agentmux-signup", token)"#));
        assert!(script.contains(r#"const SIGNUP_ATTR = "data-agentmux-signup";"#));
    }
}
