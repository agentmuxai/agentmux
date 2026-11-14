// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `generate_password`: an agent makes an account with a password the
//! Vault makes (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §7).
//!
//! The agent names the new-password field of a sign-up form by a reference from
//! its snapshot. The host finds it and says which site's frame it is in
//! (`signup_probe`: from its own frame map, never the page). The person is
//! asked in the Vault's own prompt, a window modal outside every pane, which no
//! `UI` click reaches: "Allow once" or "Deny". Then the pane shows "<agent> is
//! creating an account on <site> [Stop]" for 10 seconds, and only then is the
//! password made, kept in the Vault as an unconfirmed `login` of the tab's
//! profile, and typed into the field and its confirm field, as for the person's
//! own "Use a strong password" (browser_signup.rs). The agent is told the entry
//! and the site, never the value, and then fills the other fields itself and
//! clicks the form's button, which asks as any committing click does.
//!
//! A form that also asks for the current password is a password change (§7.3):
//! it is refused here, since a changed password locks the person out of their
//! own account. In an Incognito tab, which keeps no sign-ins, the password is
//! kept in Personal, tagged (§9.2).

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::vault::signups;
use crate::backend::vault::signins::PERSONAL;
use crate::backend::vault::uses::{Ended, UseRequest};
use agentmux_common::api_types::UiAutomationAuth;

use super::browser_actions::{note, How, Kind};
use super::browser_signup::Field;
use super::ui_handlers::{err_response, target_block_id};
use super::AppState;

/// How long the person has to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);
/// How long the notice shows before the password is made and typed.
#[cfg(not(test))]
const NOTICE: Duration = Duration::from_secs(10);
#[cfg(test)]
const NOTICE: Duration = Duration::from_millis(150);
const HOST_WAIT: Duration = Duration::from_secs(20);
/// What the Activity line is called.
const MADE: &str = "made a password on sign-up";
/// What the agent is told when another notice shows on the pane.
const BUSY: &str = "another notice is showing on this pane: no password was made; try again when it is done";
/// The longest user name the agent may give.
const MAX_USERNAME: usize = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct AccountRequest {
    auth: UiAutomationAuth,
    #[serde(default)]
    pane: Option<String>,
    /// The new-password field, by its reference from the latest snapshot or find.
    #[serde(default, rename = "ref")]
    ref_: Option<String>,
    /// The user name the agent signs up with, if it knows it yet; the one the
    /// form is sent with is the one the entry keeps.
    #[serde(default)]
    username: Option<String>,
    /// Why the agent makes the account: one line of 8 to 300 characters, required.
    #[serde(default)]
    reason: Option<String>,
}

/// What the host told of the field.
struct Probed {
    field: Field,
    change: bool,
}

fn probed(v: &Value, username: &str) -> Result<Probed, String> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let number = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as usize);
    let origin = text("origin").filter(|o| !o.is_empty()).ok_or("the host didn't say which site the field is on, so no password was made")?;
    let change = v.get("change").and_then(Value::as_bool).unwrap_or(false);
    let field = Field {
        origin,
        field: text("field").unwrap_or_default(),
        username: username.to_string(),
        passwordrules: text("passwordrules"),
        minlength: number("minlength"),
        maxlength: number("maxlength"),
    };
    if !change && field.field.is_empty() {
        return Err("the host found no field for a new password, so no password was made".to_string());
    }
    Ok(Probed { field, change })
}

fn clip_username(raw: Option<&str>) -> String {
    raw.unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(MAX_USERNAME).collect()
}

fn use_request(agent: &str, agent_pane: String, origin: &str, reason: &str, incognito: bool) -> UseRequest {
    UseRequest {
        agent_reason: reason.to_string(),
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: agent_pane,
        entry_label: format!("a new account on {origin}"),
        what: "new account".to_string(),
        origin: origin.to_string(),
        offer_always: false,
        reason: if incognito {
            "this is an Incognito tab, which keeps no sign-ins: the password is kept in Personal".to_string()
        } else {
            format!("{agent} hasn't made an account on this site with you before")
        },
        kind: "login".to_string(),
        account: true,
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    }
}

/// Ask the person; `Ok(true)` when nobody answered and it went ahead.
async fn approve(state: &AppState, block_id: &str, agent: &str, agent_pane: String, origin: &str, reason: &str, incognito: bool) -> Result<bool, (Ended, StatusCode, How, String)> {
    let headline = format!("{agent} wants to create an account on {origin}");
    let (ended, _) = super::vault_ask::ask(state, block_id, use_request(agent, agent_pane, origin, reason, incognito), ANSWER_WAIT, &headline).await;
    match ended {
        Ended::Once | Ended::Always => Ok(false),
        Ended::WentAhead => Ok(true),
        ended => {
            let (how, code) = if ended == Ended::NoAnswer { (How::TimedOut, StatusCode::REQUEST_TIMEOUT) } else { (How::Declined, StatusCode::FORBIDDEN) };
            let why = ended.refusal("no password was made");
            Err((ended, code, how, why))
        }
    }
}

/// An Activity line for a use that ended before any entry was made.
fn refused_line(state: &AppState, origin: &str, agent: &str, outcome: &str, reason: &str) {
    let label = format!("a new account on {origin}");
    crate::backend::vault::record_line(&state.identity_store, crate::backend::vault::Line { entry_id: "", label: &label, actor: agent, action: MADE, target: origin, outcome, reason });
    super::vault_handlers::publish(state);
}

/// `POST /api/v1/ui/browser/generate_password`.
pub(crate) async fn handle_generate_password(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<AccountRequest>,
) -> Response {
    let reason = match crate::backend::vault::history::require_reason(req.reason.as_deref()) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let Some(ref_) = req.ref_.as_deref().map(str::trim).filter(|r| !r.is_empty()) else {
        return err_response(StatusCode::BAD_REQUEST, "give ref: the new-password field of the sign-up form, from a snapshot or a find".to_string());
    };
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent = req.auth.agent_id.chars().take(64).collect::<String>();
    let Some(block) = state.mstore.get::<crate::backend::obj::Block>(&block_id).ok().flatten() else {
        return err_response(StatusCode::NOT_FOUND, format!("no pane {block_id}"));
    };
    let (profile, incognito) = super::browser_signin::profile_of(&block);
    let refuse = |e: String| {
        note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(How::Refused(e.clone())).record(&state);
        err_response(StatusCode::BAD_REQUEST, e)
    };
    let probe = match super::ui_handlers::proxy_value(&state, "signup_probe", json!({ "block_id": block_id, "ref": ref_ }), HOST_WAIT).await {
        Ok(v) => v,
        Err((code, e)) => return err_response(code, e),
    };
    let found = match probed(&probe, &clip_username(req.username.as_deref())) {
        Ok(f) => f,
        Err(e) => return refuse(e),
    };
    let origin = found.field.origin.clone();
    if found.change {
        return refuse(format!(
            "this form asks for the current password too: it changes the password of an account that already exists on {origin}, and a changed password locks the person out of it. No password was made: ask the person to change it"
        ));
    }
    if !super::browser_signin_fill::fillable(&origin) {
        return refuse(format!("{origin} isn't https, so no password was made for it"));
    }
    // Checked before the person is asked: a field whose rules leave no strong password gets none.
    if let Err(e) = super::browser_signup::new_password(&found.field) {
        return refuse(e);
    }
    tracing::info!(agent = %agent, block_id = %block_id, origin = %origin, "[account] an agent asked for a password for a new account");
    let agent_pane = super::vault_ask::agent_pane(&state, caller.as_deref(), &req.auth);
    let went_ahead = match approve(&state, &block_id, &agent, agent_pane, &origin, &reason, incognito).await {
        Ok(w) => w,
        Err((ended, code, how, why)) => {
            refused_line(&state, &origin, &agent, ended.outcome(), &reason);
            note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(how).record(&state);
            return err_response(code, why);
        }
    };
    match super::browser_signin::notice(&state, &block_id, &agent, &origin, "", "account", NOTICE).await {
        Some(true) => {}
        Some(false) => {
            refused_line(&state, &origin, &agent, "stopped", &reason);
            note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(How::Declined).record(&state);
            return err_response(StatusCode::FORBIDDEN, "the person stopped it: no password was made".to_string());
        }
        None => return refuse(BUSY.to_string()),
    }
    if let Err((code, e)) = super::browser_signin::still_allowed(&state, &req.auth.agent_id, req.pane.as_deref(), &block_id) {
        note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(How::Refused(e.clone())).record(&state);
        return err_response(code, e);
    }
    let (kept_in, extra) = if incognito { (PERSONAL.to_string(), Some(signups::FROM_INCOGNITO)) } else { (profile, None) };
    match super::browser_signup::make_for(&state, &block_id, found.field, kept_in.clone(), &agent, &reason, extra).await {
        Ok((site, entry)) => {
            note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(How::Allowed).record(&state);
            let mut out = json!({
                "entry": entry,
                "site": site,
                "profile": kept_in,
                "password": "[secret: hidden]",
            });
            if went_ahead {
                out["approval"] = json!(crate::backend::vault::uses::WENT_AHEAD);
            }
            (StatusCode::OK, Json(json!({ "ok": true, "data": out }))).into_response()
        }
        Err(e) => {
            note(&block_id, &agent, Kind::Fill).detail("a new account's password").how(How::Refused(e.clone())).record(&state);
            err_response(StatusCode::BAD_GATEWAY, e)
        }
    }
}

#[cfg(test)]
#[path = "browser_account_tests.rs"]
mod tests;
