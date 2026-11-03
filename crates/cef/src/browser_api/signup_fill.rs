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
    /// The token the listener put on the field it offered (`data-agentmux-signup`):
    /// only that field is filled.
    pub field: String,
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

/// The field the listener offered, by its token, if it is still a visible
/// field for a new password: marked `new-password`, the first of two password
/// fields whose second says confirm, or a form's one password field with a
/// button like "Create account"; never in a form with a `current-password`
/// field (a password change, §7.3).
fn find_new_password(token: &str) -> String {
    let token = serde_json::to_string(token).unwrap_or_default();
    format!(
        r#"(() => {{
  const token = {token};
  const visible = {VISIBLE};
  const shown = {SHOWN};
  const ac = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();
  const confirmWords = /confirm|repeat|retype|again|verify/i;
  const signupWords = /create (an |your )?account|sign ?up|register|join/i;
  const described = (el) => [...(el.labels || [])].map((l) => l.textContent).join(" ") + ` ${{el.name}} ${{el.id}} ${{ac(el)}} ${{el.placeholder || ""}}`;
  for (const p of [...document.querySelectorAll("input[type=password]")].filter(visible)) {{
    if (p.getAttribute("data-agentmux-signup") !== token) continue;
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

/// The other field of the pair `this` belongs to: the other of exactly two
/// password fields its form (or the page) renders, whichever of the two was
/// offered (both can be marked `new-password`), so the new password and its
/// confirmation always get the same value. It is filled only if it is
/// [`VISIBLE`] too (`type_into` checks).
fn confirm_for() -> String {
    format!(
        r#"function () {{
  const shown = {SHOWN};
  const passwords = [...((this.form || this.closest("form")) || document).querySelectorAll("input[type=password]")].filter(shown);
  return passwords.length === 2 && passwords.includes(this) ? passwords.find((p) => p !== this) : null;
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
    if req.field.is_empty() || req.field.len() > 64 || !req.field.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("no field was offered for this password, so nothing was filled".to_string());
    }
    for (session, frames) in frames_for(root, &req.origin).await? {
        let mut cdp = root.in_session(session.as_deref());
        for frame in frames {
            let ctx = super::act::isolated_context(&mut cdp, Some(&frame)).await?;
            let Some(field) = object(&mut cdp, ctx, &find_new_password(&req.field)).await? else { continue };
            // Found before the first value goes in: setting it runs the page's handlers.
            let confirm = call_on_object(&mut cdp, &field, &confirm_for()).await?;
            type_into(&mut cdp, &field, &req.password, &req.origin, "password").await?;
            let Some(confirm) = confirm else { return Ok(SignupFillData { fields: 1 }) };
            type_into(&mut cdp, &confirm, &req.password, &req.origin, "password").await?;
            return Ok(SignupFillData { fields: 2 });
        }
    }
    Err(format!("the field for a new password the offer was for isn't on {} now, so nothing was filled", req.origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_field_for_a_new_password_is_filled_never_a_password_change() {
        let find = find_new_password("t-1");
        for needle in ["input[type=password]", "visible", "current-password", "continue", "new-password", "passwords.length === 2", "confirmWords", "signupWords"] {
            assert!(find.contains(needle), "{needle}");
        }
        let at = |needle: &str| find.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at(r#"ac(i).includes("current-password")"#) < at(r#"ac(p).includes("new-password")"#), "a password change is skipped first");
        let confirm = confirm_for();
        assert!(confirm.contains("passwords.length === 2 && passwords.includes(this) ? passwords.find((p) => p !== this)"), "either field of the pair fills the other");
    }

    #[test]
    fn only_the_field_the_offer_was_for_is_filled() {
        let find = find_new_password("t-1");
        assert!(find.contains(r#"const token = "t-1";"#));
        let at = |needle: &str| find.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at(r#"p.getAttribute("data-agentmux-signup") !== token"#) < at(r#"ac(p).includes("new-password")"#), "the token is checked before anything else");
        assert!(crate::browser_pane::signin_notice::SCRIPT.contains(r#"const SIGNUP_ATTR = "data-agentmux-signup";"#));
        // A token is written into the script as a JSON string: never code.
        assert!(find_new_password(r#"x"; alert(1); ""#).contains(r#"const token = "x\"; alert(1); \"";"#));
    }

    #[test]
    fn the_rules_match_the_listener_that_offered_the_field() {
        let script = crate::browser_pane::signin_notice::SCRIPT;
        // The same fields counted: the listener's `shown`, word for word.
        let body = SHOWN.trim_start_matches("(el) => ");
        assert!(script.contains(&format!("const shown = (el) => {body};")), "signin_notice.js counts fields with: {body}");
        assert!(find_new_password("t-1").contains(".filter(shown)") && confirm_for().contains(".filter(shown)"));
        for words in ["/confirm|repeat|retype|again|verify/i", "/create (an |your )?account|sign ?up|register|join/i"] {
            assert!(script.contains(words), "signin_notice.js: {words}");
            assert!(find_new_password("t-1").contains(words), "signup_fill.rs: {words}");
        }
    }
}
