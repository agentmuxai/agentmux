// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filling a strong password into a sign-up form
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §6.2).
//!
//! `/agentmux/browser/signup_fill`, for srv only: srv sends the password it
//! made and already keeps in the Vault, once the person chose "Use a strong
//! password" from the pane's operator-only menu, and the frame origin it is
//! for. The host finds the field for a new password in a frame of exactly that
//! origin, by the rules `signin_notice.js` offered it by, and its confirm field
//! if the form has one, and sets the password into both with the sign-in
//! fill's checks ([`super::signin_fill::type_into`]): still a visible password
//! field of that origin, on top, marked as holding a Vault value. The answer
//! never holds the value.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::act::{fail, unauthorized};
use super::cdp::CdpSession;
use super::signin_fill::{call_on_object, frames_for, object, type_into, VISIBLE};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SignupFillReq {
    pub block_id: String,
    /// The exact origin the password was made for: only a frame of this origin is filled.
    pub origin: String,
    /// Held only for the length of this request.
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SignupFillData {
    /// How many fields took it: the new password, and its confirm field.
    pub fields: u8,
}

/// Which password fields count when telling a sign-up form's shape: those the
/// page renders, as `signin_notice.js` counts them (`shown`), so the field it
/// offered is the field found here. The one filled must also be [`VISIBLE`].
const SHOWN: &str = r#"(el) => el.isConnected && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden""#;

/// The first visible field for a new password: marked `new-password`, the
/// first of two password fields whose second says confirm, or a form's one
/// password field with a button like "Create account"; never in a form with a
/// `current-password` field (a password change, §7.3).
fn find_new_password() -> String {
    format!(
        r#"(() => {{
  const visible = {VISIBLE};
  const shown = {SHOWN};
  const ac = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();
  const confirmWords = /confirm|repeat|retype|again|verify/i;
  const signupWords = /create (an |your )?account|sign ?up|register|join/i;
  const described = (el) => [...(el.labels || [])].map((l) => l.textContent).join(" ") + ` ${{el.name}} ${{el.id}} ${{ac(el)}} ${{el.placeholder || ""}}`;
  for (const p of [...document.querySelectorAll("input[type=password]")].filter(visible)) {{
    const group = p.form || p.closest("form");
    const passwords = [...(group || document).querySelectorAll("input[type=password]")].filter(shown);
    if (passwords.some((i) => ac(i).includes("current-password"))) continue;
    if (ac(p).includes("new-password")) return p;
    if (passwords.length === 2 && p === passwords[0] && confirmWords.test(described(passwords[1]))) return p;
    if (passwords.length === 1 && group && [...group.querySelectorAll("button, input[type=submit]")].some((b) => signupWords.test(b.textContent || b.value || ""))) return p;
  }}
  return null;
}})()"#
    )
}

/// The field that confirms `this`: the second of exactly two password fields
/// its form (or the page) renders, when `this` is the first. It is filled only
/// if it is [`VISIBLE`] too (`type_into` checks).
fn confirm_for() -> String {
    format!(
        r#"function () {{
  const shown = {SHOWN};
  const passwords = [...((this.form || this.closest("form")) || document).querySelectorAll("input[type=password]")].filter(shown);
  return passwords.length === 2 && passwords[0] === this ? passwords[1] : null;
}}"#
    )
}

/// `POST /agentmux/browser/signup_fill`.
pub async fn signup_fill_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<SignupFillReq>) -> (StatusCode, Json<ApiResponse<SignupFillData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "signup-fill").await {
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

async fn fill(root: &mut CdpSession, req: &SignupFillReq) -> Result<SignupFillData, String> {
    for (session, frames) in frames_for(root, &req.origin).await? {
        let mut cdp = root.in_session(session.as_deref());
        for frame in frames {
            let ctx = super::act::isolated_context(&mut cdp, Some(&frame)).await?;
            let Some(field) = object(&mut cdp, ctx, &find_new_password()).await? else { continue };
            // Found before the first value goes in: setting it runs the page's handlers.
            let confirm = call_on_object(&mut cdp, &field, &confirm_for()).await?;
            type_into(&mut cdp, &field, &req.password, &req.origin, "password").await?;
            let Some(confirm) = confirm else { return Ok(SignupFillData { fields: 1 }) };
            type_into(&mut cdp, &confirm, &req.password, &req.origin, "password").await?;
            return Ok(SignupFillData { fields: 2 });
        }
    }
    Err(format!("no field for a new password on {} is on the page now, so nothing was filled", req.origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_field_for_a_new_password_is_filled_never_a_password_change() {
        let find = find_new_password();
        for needle in ["input[type=password]", "visible", "current-password", "continue", "new-password", "passwords.length === 2", "confirmWords", "signupWords"] {
            assert!(find.contains(needle), "{needle}");
        }
        let at = |needle: &str| find.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at(r#"ac(i).includes("current-password")"#) < at(r#"ac(p).includes("new-password")"#), "a password change is skipped first");
        let confirm = confirm_for();
        assert!(confirm.contains("passwords.length === 2 && passwords[0] === this"));
    }

    #[test]
    fn the_rules_match_the_listener_that_offered_the_field() {
        let script = crate::browser_pane::signin_notice::SCRIPT;
        // The same fields counted: the listener's `shown`, word for word.
        let body = SHOWN.trim_start_matches("(el) => ");
        assert!(script.contains(&format!("const shown = (el) => {body};")), "signin_notice.js counts fields with: {body}");
        assert!(find_new_password().contains(".filter(shown)") && confirm_for().contains(".filter(shown)"));
        for words in ["/confirm|repeat|retype|again|verify/i", "/create (an |your )?account|sign ?up|register|join/i"] {
            assert!(script.contains(words), "signin_notice.js: {words}");
            assert!(find_new_password().contains(words), "signup_fill.rs: {words}");
        }
    }
}
