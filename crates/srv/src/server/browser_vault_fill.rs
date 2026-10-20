// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `fill` with `vault`: an agent names a Vault entry, the person says
//! where it may go, and the host types it
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6).
//!
//! The value never reaches the agent. It goes from the sealed row to the host in
//! the request that types it, and the agent's answer says only that the field was
//! filled. Every use is in the Vault's Activity: who, which entry, which site and
//! field, and what became of it; never the value.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::vault::uses::{self, Decision, UseRequest};
use crate::backend::vault::{self, FillTarget};
use agentmux_common::api_types::UiAutomationAuth;

use super::browser_actions::{note, How, Kind};
use super::ui_handlers::{err_response, proxy_value, target_block_id};
use super::AppState;

/// How long the person has to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);
const HOST_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub(crate) struct VaultFillRequest {
    auth: UiAutomationAuth,
    #[serde(default)]
    pane: Option<String>,
    #[serde(rename = "ref")]
    ref_: String,
    /// `"id"` or `"id.part"`.
    vault: String,
}

/// What the host says about the field.
struct Field {
    origin: String,
    label: String,
    fingerprint: String,
    slots: Vec<String>,
    password_like: bool,
    fillable: bool,
}

fn parse_field(v: &Value) -> Result<Field, String> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Ok(Field {
        origin: s("origin"),
        label: s("label"),
        fingerprint: s("fingerprint"),
        slots: v.get("slots").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        password_like: v.get("password_like").and_then(Value::as_bool).unwrap_or(false),
        fillable: v.get("fillable").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// What happens before the host types: refused, allowed without asking, or asked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    Refuse(&'static str),
    Go,
    Ask { offer_always: bool, reason: &'static str },
}

/// The table in spec §6.3: a granted site and a field that looks like the entry's
/// kind fill without asking; anything else asks, and "Always" is offered only
/// when the field looks like the entry's kind.
pub(crate) fn plan(origin_known: bool, password_like: bool, fillable: bool, granted: bool, fits: bool) -> Plan {
    if !origin_known {
        return Plan::Refuse("AgentMux can't tell which site this field is on, so nothing was typed");
    }
    if password_like {
        return Plan::Refuse("this is a password or one-time-code field: the Vault doesn't fill those; ask the person to fill it");
    }
    if !fillable {
        return Plan::Refuse("this isn't a field that takes typed text");
    }
    match (granted, fits) {
        (true, true) => Plan::Go,
        (true, false) => Plan::Ask { offer_always: false, reason: "the field doesn't look like what this entry is" },
        (false, true) => Plan::Ask { offer_always: true, reason: "the first time this entry is used on this site" },
        (false, false) => Plan::Ask { offer_always: false, reason: "a new site, and the field doesn't look like what this entry is" },
    }
}

fn record(state: &AppState, block_id: &str, target: &FillTarget, agent: &str, where_: &str, outcome: &str) {
    crate::backend::vault::record(&state.identity_store, &target.id, &target.label, agent, "filled a web form", where_, outcome);
    // The pane's own Log shows it too, with what was filled and never its value.
    let how = match outcome {
        "done" => How::Allowed,
        "denied" => How::Declined,
        "timed out" => How::TimedOut,
        other => How::Refused(other.to_string()),
    };
    note(block_id, agent, Kind::Fill).detail(&format!("{} from the Vault", target.what)).how(how).record(state);
}

/// Takes a use request down, and tells every window, however the wait ends: the
/// agent's call can be dropped (it disconnected, or was stopped) while the person
/// still has the prompt open, and the prompt must go then too.
struct Waiting {
    id: String,
    state: AppState,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        uses::uses().forget(&self.id);
        super::vault_handlers::publish(&self.state);
    }
}

/// `POST /api/v1/ui/browser/vault_fill`.
pub(crate) async fn handle_vault_fill(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<VaultFillRequest>,
) -> Response {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent = req.auth.agent_id.chars().take(64).collect::<String>();
    // The entry and part, by name only: nothing here reads the value yet.
    let store = state.identity_store.clone();
    let wanted = req.vault.clone();
    let target = match tokio::task::spawn_blocking(move || vault::fill_target(&store, &wanted)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return err_response(StatusCode::BAD_REQUEST, e),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    tracing::info!(agent = %agent, block_id = %block_id, entry = %target.id, "[vault] a web form fill was asked for");

    let field = match proxy_value(&state, "field_info", json!({ "block_id": block_id, "ref": req.ref_ }), HOST_WAIT).await {
        Ok(v) => match parse_field(&v) {
            Ok(f) => f,
            Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
        },
        Err((code, e)) => return err_response(code, e),
    };
    let site = format!("{} ({})", field.origin, field.label);

    let store = state.identity_store.clone();
    let (entry, origin) = (target.id.clone(), field.origin.clone());
    let granted = tokio::task::spawn_blocking(move || store.vault_grant_check(&entry, &origin).unwrap_or(false)).await.unwrap_or(false);
    let fits = target.slot.is_some_and(|s| field.slots.iter().any(|x| x == s));

    let mut always = false;
    match plan(!field.origin.is_empty(), field.password_like, field.fillable, granted, fits) {
        Plan::Refuse(why) => {
            record(&state, &block_id, &target, &agent, &site, "refused");
            return err_response(StatusCode::BAD_REQUEST, why.to_string());
        }
        Plan::Go => {}
        Plan::Ask { offer_always, reason } => {
            let use_id = uuid::Uuid::new_v4().to_string();
            let request = UseRequest {
                id: use_id.clone(),
                agent: agent.clone(),
                entry_id: target.id.clone(),
                entry_label: target.label.clone(),
                what: target.what.clone(),
                origin: field.origin.clone(),
                field: field.label.clone(),
                offer_always,
                reason: reason.to_string(),
                requested_ms: agentmux_common::time::now_ms_u64(),
            };
            let answer = uses::uses().ask(request);
            let _waiting = Waiting { id: use_id.clone(), state: state.clone() };
            super::vault_handlers::publish(&state);
            let _asking = crate::backend::user_attention::Asking::start(
                &state.broker,
                &state.mstore,
                &block_id,
                "vault",
                &format!("{agent} wants to fill your {} on {}", target.what, field.origin),
            );
            let decision = tokio::time::timeout(ANSWER_WAIT, answer).await;
            match decision {
                Ok(Ok(Decision::Once)) => {}
                Ok(Ok(Decision::Always)) => always = true,
                Ok(Ok(Decision::Deny)) | Ok(Err(_)) => {
                    record(&state, &block_id, &target, &agent, &site, "denied");
                    return err_response(StatusCode::FORBIDDEN, "the person said no: nothing was typed".to_string());
                }
                Err(_) => {
                    record(&state, &block_id, &target, &agent, &site, "timed out");
                    return err_response(StatusCode::REQUEST_TIMEOUT, "the person didn't answer in time: nothing was typed; ask again when they are there".to_string());
                }
            }
        }
    }
    if always {
        let (store, entry, origin, created) = (state.identity_store.clone(), target.id.clone(), field.origin.clone(), target.created_at);
        let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&entry, &origin, agentmux_common::time::now_ms(), created)).await;
    }

    // The value leaves the sealed row only now, for the host's request.
    let store = state.identity_store.clone();
    let (entry, part, created) = (target.id.clone(), target.part, target.created_at);
    let value = match tokio::task::spawn_blocking(move || vault::value_for_fill(&store, &entry, created, part)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            record(&state, &block_id, &target, &agent, &site, "failed");
            return err_response(StatusCode::BAD_REQUEST, e);
        }
        Err(e) => {
            record(&state, &block_id, &target, &agent, &site, "failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    };
    let body = json!({
        "block_id": block_id,
        "ref": req.ref_,
        "text": value.as_str(),
        "expect_origin": field.origin,
        "expect_fingerprint": field.fingerprint,
    });
    let filled = proxy_value(&state, "vault_fill", body, HOST_WAIT).await;
    // Whatever the host says, the value isn't in it: an error that quotes it (a
    // parser's, a page's) is cleaned before the agent can see it.
    let filled = filled.map_err(|(code, e)| (code, if value.is_empty() { e } else { e.replace(value.as_str(), "[hidden]") }));
    // The page's own validation message is page-controlled text: it can echo the
    // value it was given, reformatted (dashes stripped, spaces added) so that no
    // replacement of the original finds it. The agent is told only that there is one.
    let invalid = filled.as_ref().ok().and_then(|d| d.get("validation").and_then(Value::as_str)).is_some_and(|v| !v.is_empty());
    drop(value);
    match filled {
        Ok(data) => {
            record(&state, &block_id, &target, &agent, &site, "done");
            let store = state.identity_store.clone();
            let entry = target.id.clone();
            let _ = tokio::task::spawn_blocking(move || store.vault_touch(&entry, agentmux_common::time::now_ms())).await;
            let mut out = json!({
                "filled_from_vault": target.id,
                "part": target.part,
                "value": "[secret: hidden]",
                "site": field.origin,
                "field": field.label,
            });
            if let Some(o) = out.as_object_mut() {
                if invalid {
                    o.insert("validation".into(), json!("the page flagged the value as invalid (its message isn't shown, since a page can put the value in it)"));
                }
                if data.get("changed_by_page").and_then(Value::as_bool) == Some(true) {
                    o.insert("note".into(), json!("the page shortened or reformatted the value"));
                }
            }
            (StatusCode::OK, Json(json!({ "ok": true, "data": out }))).into_response()
        }
        Err((code, e)) => {
            record(&state, &block_id, &target, &agent, &site, "failed");
            err_response(code, e)
        }
    }
}

#[cfg(test)]
mod tests;
