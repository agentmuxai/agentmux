// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `generate_password` on a form that changes a password
//! (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §7.3).
//!
//! A form with a `current-password` field changes an account that already exists, and a wrong change
//! locks the person out of it. So, unlike a new account:
//! - the person is asked **every time**, in the Vault's own prompt. There is no "Always", the standing
//!   `accounts` permission does not apply, and silence is a no;
//! - the new password is a **pending version** of the saved sign-in for that site and user
//!   (`backend::vault::changes`), not an entry of its own and not its value: the saved password stays
//!   the one that works until the site was sent the form with the new one, when the page's listener
//!   reports it (`credential.ChangeSent`, [`sent`]) and the old one is kept 7 days for the person to put back;
//! - an Incognito tab keeps no sign-ins, so there is nothing saved to change there.
//!
//! The agent fills the current password with `signin` (a saved sign-in, as it asks today), fills the
//! rest itself and clicks the form's button, which asks as any committing click does.

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use zeroize::Zeroizing;

use crate::backend::vault::signins::Space;
use crate::backend::vault::uses::{Ended, UseRequest};
use crate::backend::vault::{changes, Line};

use super::browser_actions::{note, How, Kind};
use super::browser_signup::Field;
use super::ui_handlers::err_response;
use super::AppState;

const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);
#[cfg(not(test))]
const NOTICE: Duration = Duration::from_secs(10);
#[cfg(test)]
const NOTICE: Duration = Duration::from_millis(150);
const HOST_WAIT: Duration = Duration::from_secs(20);
/// What the Activity line is called when the person was asked, and when the password was typed.
const ASKED: &str = "asked to change the password";
const FILLED: &str = "filled on password change";
/// The detail of the pane's action note.
const DETAIL: &str = "a changed password";
const BUSY: &str = "another notice is showing on this pane: no password was made; try again when it is done";

/// What the handler knows of the request and the pane when the host says the form is a password change.
pub(crate) struct Asked<'a> {
    pub block_id: &'a str,
    pub agent: &'a str,
    pub agent_id: &'a str,
    pub pane: Option<&'a str>,
    pub agent_pane: String,
    pub reason: &'a str,
    pub profile: String,
    pub incognito: bool,
    /// The form's field for the new password, as the host marked it, and the user name.
    pub field: Field,
}

fn use_request(a: &Asked<'_>, origin: &str) -> UseRequest {
    UseRequest {
        agent_reason: a.reason.to_string(),
        id: uuid::Uuid::new_v4().to_string(),
        agent: a.agent.to_string(),
        block_id: a.agent_pane.clone(),
        entry_label: format!("your password on {origin}"),
        what: "password change".to_string(),
        origin: origin.to_string(),
        // Asked every time, with no Always: a changed password locks the person out of the account if it is wrong.
        offer_always: false,
        reason: "a changed password stops the old one working on the site; the Vault keeps the old one for 7 days".to_string(),
        kind: "login".to_string(),
        // Silence is a no for an account, and no standing permission or grant answers for the person.
        account: true,
        change: true,
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    }
}

/// An Activity line for the saved sign-in `entry` that this change is about.
fn line(state: &AppState, entry: &str, label: &str, a: &Asked<'_>, action: &str, outcome: &str) {
    let l = Line { entry_id: entry, label, actor: a.agent, action, target: &a.field.origin, outcome, reason: a.reason };
    crate::backend::vault::record_line(&state.identity_store, l);
    super::vault_handlers::publish(state);
}

fn refuse(state: &AppState, a: &Asked<'_>, why: String) -> Response {
    note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(How::Refused(why.clone())).record(state);
    err_response(StatusCode::BAD_REQUEST, why)
}

/// `generate_password` on a password-change form: ask, count down, make the pending password, type it.
pub(crate) async fn handle(state: &AppState, a: Asked<'_>) -> Response {
    let origin = a.field.origin.clone();
    if a.incognito {
        return refuse(state, &a, format!("this is an Incognito tab, which keeps no sign-ins, so there is no saved password for {origin} to change: ask the person"));
    }
    if !super::browser_signin_fill::fillable(&origin) {
        return refuse(state, &a, format!("{origin} isn't https, so no password was made for it"));
    }
    // Before the person is asked: a field whose rules leave no strong password gets none.
    if let Err(e) = super::browser_signup::new_password(&a.field) {
        return refuse(state, &a, e);
    }
    let space = Space { profile: a.profile.clone(), origin: origin.clone(), realm: String::new(), proxy: false };
    let (store, who, user) = (state.identity_store.clone(), a.agent.to_string(), a.field.username.clone());
    let sp = space.clone();
    let saved = match tokio::task::spawn_blocking(move || changes::saved_login(&store, &sp, &user, &who)).await {
        Ok(Ok(row)) => row,
        Ok(Err(e)) => return refuse(state, &a, e),
        Err(e) => return refuse(state, &a, e.to_string()),
    };
    tracing::info!(agent = %a.agent, block_id = %a.block_id, origin = %origin, "[change] an agent asked to change a saved password");
    if let Err((ended, code, how, why)) = approve(state, &a, &origin).await {
        line(state, &saved.id, &saved.label, &a, ASKED, ended.outcome());
        note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(how).record(state);
        return err_response(code, why);
    }
    match super::browser_signin::notice(state, a.block_id, a.agent, &origin, &a.field.username, "change", NOTICE).await {
        Some(true) => {}
        Some(false) => {
            line(state, &saved.id, &saved.label, &a, ASKED, "stopped");
            note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(How::Declined).record(state);
            return err_response(StatusCode::FORBIDDEN, "the person stopped it: no password was made".to_string());
        }
        None => return refuse(state, &a, BUSY.to_string()),
    }
    if let Err((code, e)) = super::browser_signin::still_allowed(state, a.agent_id, a.pane, a.block_id) {
        note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(How::Refused(e.clone())).record(state);
        return err_response(code, e);
    }
    match make(state, &a, &saved).await {
        Ok(()) => {
            note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(How::Allowed).record(state);
            let out = json!({
                "entry": saved.id,
                "site": origin,
                "profile": a.profile,
                "password": "[secret: hidden]",
                "kept": "the saved password stays the one that works until the form is sent with the new one; the old one is then kept 7 days",
            });
            (StatusCode::OK, Json(json!({ "ok": true, "data": out }))).into_response()
        }
        Err(e) => {
            note(a.block_id, a.agent, Kind::Fill).detail(DETAIL).how(How::Refused(e.clone())).record(state);
            err_response(StatusCode::BAD_GATEWAY, e)
        }
    }
}

/// Ask the person; only "Allow once" goes on.
async fn approve(state: &AppState, a: &Asked<'_>, origin: &str) -> Result<(), (Ended, StatusCode, How, String)> {
    let headline = format!("{} wants to change your password on {origin}", a.agent);
    let (ended, _) = super::vault_ask::ask(state, a.block_id, use_request(a, origin), ANSWER_WAIT, &headline).await;
    match ended {
        Ended::Once => Ok(()),
        other => Err(super::browser_account::not_allowed(other, "an agent changes a password")),
    }
}

/// Make the password, keep it as the saved sign-in's pending change, then type it into the page.
async fn make(state: &AppState, a: &Asked<'_>, saved: &crate::backend::storage::vault::VaultRow) -> Result<(), String> {
    let password = super::browser_signup::new_password(&a.field)?;
    let (store, row, value, who) = (state.identity_store.clone(), saved.clone(), password.clone(), a.agent.to_string());
    let change = tokio::task::spawn_blocking(move || changes::start(&store, &row, &value, &who)).await.map_err(|e| e.to_string())??;
    let body = json!({ "block_id": a.block_id, "origin": a.field.origin, "field": a.field.field, "password": password.as_str() });
    let filled = super::ui_handlers::proxy_value(state, "signup_fill", body, HOST_WAIT).await;
    drop(password);
    match filled {
        Ok(_) => {
            line(state, &saved.id, &saved.label, a, FILLED, "done");
            Ok(())
        }
        Err((_, e)) => {
            // No page saw it: nothing to keep.
            let store = state.identity_store.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || changes::abandon(&store, &change)).await.map_err(|e| e.to_string()).and_then(|r| r) {
                tracing::warn!(error = %e, "[change] couldn't drop a password change that was never typed");
            }
            line(state, &saved.id, &saved.label, a, FILLED, "failed");
            Err(format!("the password wasn't typed, so none was kept: {e}"))
        }
    }
}

/// A password-change form of `origin` was sent in pane `block_id` with `password`. The password an agent
/// made for it becomes the saved sign-in's, the old one kept 7 days; any other is the person's own, offered
/// "Update password?" as a sign-in is. Never logged.
pub(crate) fn sent(state: &AppState, block_id: &str, origin: &str, username: Zeroizing<String>, password: Zeroizing<String>) {
    if let Some(profile) = super::browser_signin_offer::profile_of(state, block_id) {
        let space = Space { profile, origin: origin.to_string(), realm: String::new(), proxy: false };
        match changes::confirm_sent(&state.identity_store, &space, &password) {
            Ok(Some(_)) => {
                super::vault_handlers::publish(state);
                return;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "[change] couldn't confirm the changed password"),
        }
    }
    super::browser_signin_offer::offer(state, block_id, origin, username, Some(password));
}
