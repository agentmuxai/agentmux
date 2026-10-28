// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The routes of the `Sensitive` tool (`SPEC_SUPER_AGENT_2026_10_25.md` §4): an
//! agent asks for a sensitive operation, the super agent approves or denies it,
//! and an approved one starts its countdown, which the person can veto.
//!
//!   POST /api/v1/sensitive/{request,approve,deny,cancel,status,list}  { agent_id, ... }
//!
//! The agent's token decides who acts; the body's `agent_id` must be a name that
//! agent answers to. Every message these send is from AgentMux, names the
//! requester, and says whose request it is: nothing in one is the operator's voice.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::caller::Caller;
use super::AppState;
use crate::backend::reactive::sanitize::quote_in_system_note;
use crate::backend::sensitive_ops;
use crate::backend::sensitive_requests::{self as requests, Request};
use crate::sagas::sensitive_fallback as fallback;
use crate::sagas::sensitive_window as window;
use agentmux_common::time::now_ms;

pub(crate) async fn handle_sensitive(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Path(action): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let forbid = |e: &str| (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response();
    let Some(uid) = caller.as_deref().and_then(Caller::uid).map(str::to_string) else {
        return forbid("FORBIDDEN: this acts for an agent, so the request must carry that agent's token");
    };
    let claimed = text(&body, "agent_id");
    let mstore = state.mstore.clone();
    let proven = {
        let uid = uid.clone();
        tokio::task::spawn_blocking(move || super::actor::proven_slug(&mstore, &uid, &claimed)).await.ok().flatten()
    };
    let Some(slug) = proven else {
        return forbid("FORBIDDEN: the token's agent doesn't answer to that agent_id");
    };
    match act(&state, &uid, &slug, &action, &body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = match e.split(':').next().unwrap_or("") {
                "FORBIDDEN" => StatusCode::FORBIDDEN,
                "INVALID_ARG" => StatusCode::BAD_REQUEST,
                "LIMIT" => StatusCode::TOO_MANY_REQUESTS,
                _ if e.starts_with("no request") => StatusCode::NOT_FOUND,
                _ => StatusCode::CONFLICT,
            };
            (code, Json(json!({ "error": e }))).into_response()
        }
    }
}

fn text(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// The facts of a request: an object of strings.
fn facts_of(body: &Value) -> Result<BTreeMap<String, String>, String> {
    let Some(obj) = body.get("facts") else {
        return Ok(BTreeMap::new());
    };
    let obj = obj.as_object().ok_or("INVALID_ARG: facts is an object of text values")?;
    obj.iter()
        .map(|(k, v)| match v.as_str() {
            Some(s) => Ok((k.clone(), s.to_string())),
            None => Err(format!("INVALID_ARG: fact '{}' is text", k.chars().take(40).collect::<String>())),
        })
        .collect()
}

/// Do `action` for the agent, and leave an audit row for the steps of a request: the row names the
/// agent that did it, a refused approval or denial included.
async fn act(state: &AppState, uid: &str, slug: &str, action: &str, body: &Value) -> Result<Value, String> {
    let result = act_inner(state, uid, slug, action, body).await;
    if matches!(action, "request" | "approve" | "deny" | "cancel") {
        let id = result.as_ref().ok().and_then(|v| v["id"].as_str()).map(str::to_string).unwrap_or_else(|| text(body, "id"));
        // A request refused before it was filed has nothing to attach a row to.
        if let Some(request) = requests::registry().get(&id) {
            let outcome = match &result {
                Ok(v) if action == "request" && v["joined"] == true => "joined".to_string(),
                Ok(_) => serde_json::to_value(request.status).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                Err(_) => "refused".to_string(),
            };
            window::audit_step(state, slug, &request, &format!("sensitive.{action}"), result.is_ok(), result.as_ref().err().map(String::as_str), &outcome);
        }
    }
    result
}

async fn act_inner(state: &AppState, uid: &str, slug: &str, action: &str, body: &Value) -> Result<Value, String> {
    let id = text(body, "id");
    match action {
        "request" => {
            let filed = requests::registry().request(uid, slug, &text(body, "op"), &facts_of(body)?, &text(body, "reason"), now_ms())?;
            let request = requests::registry().get(&filed.id).ok_or("no request (it ended just now)")?;
            let review = if filed.joined {
                "joined a request for the same thing that was already open".to_string()
            } else {
                fallback::watch(state, &filed.id);
                ask_the_super_agent(state, &request)
            };
            Ok(json!({ "id": filed.id, "joined": filed.joined, "status": request.status, "summary": request.summary, "review": review }))
        }
        "approve" => {
            require_super(state, uid)?;
            let request = must_get(&id)?;
            let spec = sensitive_ops::spec(&request.op).ok_or("INVALID_ARG: unknown operation")?;
            if !window::has_runner(&request.op) {
                return Err(format!("{} can't be run from here yet, so it isn't approved", spec.label));
            }
            requests::registry().approve(&id, slug, now_ms())?;
            let view = window::open(state, &id, Duration::from_secs(spec.window_secs))?;
            tell_askers(&request, &approved_note(&request, slug, spec.window_secs));
            Ok(json!({ "id": id, "status": "pending", "deadline_ms": view.deadline_ms, "window_secs": spec.window_secs }))
        }
        "deny" => {
            require_super(state, uid)?;
            let request = must_get(&id)?;
            let denied = requests::registry().deny(&id, slug, &text(body, "reason"), now_ms())?;
            tell_askers(&denied, &denied_note(&denied, slug));
            Ok(json!({ "id": id, "status": "denied", "requester": request.requester_slug }))
        }
        "cancel" => {
            let request = requests::registry().cancel(&id, uid, now_ms())?;
            // The last agent that asked withdrew it while its countdown ran: stop the banner.
            if request.status == requests::Status::Cancelled {
                window::cancelled(state, &id);
            }
            Ok(json!({ "id": id, "status": request.status }))
        }
        "status" => {
            let request = must_get(&id)?;
            if !request.asked_by(uid) && !is_super(state, uid) {
                return Err("FORBIDDEN: you can see the requests you asked for".to_string());
            }
            Ok(shown(&request))
        }
        "list" => {
            let all = is_super(state, uid);
            Ok(json!({ "requests": requests::registry().list(uid, all).iter().map(shown).collect::<Vec<_>>() }))
        }
        other => Err(format!("INVALID_ARG: unknown action '{}'; one of: request, approve, deny, cancel, status, list", other.chars().take(20).collect::<String>())),
    }
}

fn must_get(id: &str) -> Result<Request, String> {
    if id.is_empty() {
        return Err("INVALID_ARG: this needs the request's id".to_string());
    }
    requests::registry().get(id).ok_or_else(|| format!("no request {}", id.chars().take(40).collect::<String>()))
}

/// The uid of the agent holding the `super` seat, if any.
fn super_uid(state: &AppState) -> Option<String> {
    let defs = state.mstore.agent_def_list().unwrap_or_default();
    crate::backend::agent_seat::super_holder(&state.mstore, &defs)
}

fn is_super(state: &AppState, uid: &str) -> bool {
    super_uid(state).as_deref() == Some(uid)
}

fn require_super(state: &AppState, uid: &str) -> Result<(), String> {
    if is_super(state, uid) {
        Ok(())
    } else {
        Err("FORBIDDEN: only the super agent approves or denies a sensitive request".to_string())
    }
}

/// Tell an agent something, from AgentMux. Not delivered (the agent isn't running) is not an error.
fn tell(uid: &str, note: &str) -> bool {
    match crate::backend::reactive::get_global_handler().deliver_system_note(uid, note) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!(error = %e, "sensitive request: note not delivered");
            false
        }
    }
}

/// The agents that asked for it: the first, then those that joined.
fn askers(request: &Request) -> Vec<&str> {
    std::iter::once(request.requester_uid.as_str()).chain(request.also_asked_by.iter().map(|a| a.uid.as_str())).collect()
}

/// Tell every agent that asked for it, those that joined included.
fn tell_askers(request: &Request, note: &str) {
    for uid in askers(request) {
        tell(uid, note);
    }
}

/// Send the super agent the request, and say to the requester what came of it.
fn ask_the_super_agent(state: &AppState, request: &Request) -> String {
    let Some(holder) = super_uid(state) else {
        return "no agent holds the super seat. If none does shortly, the person gets a countdown twice as long, saying no super agent reviewed it, and can veto it".to_string();
    };
    if holder == request.requester_uid {
        return "you are the super agent: approve it yourself with action approve".to_string();
    }
    if tell(&holder, &request_note(request)) {
        "the super agent has been asked; if it doesn't answer in 2 minutes, the person gets a countdown twice as long, saying no super agent reviewed it".to_string()
    } else {
        "the super agent isn't running here. If it doesn't answer in 2 minutes, the person gets a countdown twice as long, saying no super agent reviewed it, and can veto it".to_string()
    }
}

fn who(request: &Request) -> String {
    quote_in_system_note(&request.requester_slug)
}

/// What the super agent is told. The summary is built from the request's facts; the reason is the
/// requester's own words, quoted as theirs.
fn request_note(request: &Request) -> String {
    let secs = sensitive_ops::spec(&request.op).map_or(0, |s| s.window_secs);
    let reason = if request.reason.is_empty() { String::new() } else { format!(" Their reason, as they wrote it: \"{}\".", quote_in_system_note(&request.reason)) };
    format!(
        "[AgentMux] {} asks to {}. This is their request, not the operator's words.{reason} Request {}. \
         Approve or deny it with Sensitive (action approve or deny, id); once you approve, the person has {secs} s to veto it.",
        who(request),
        request.summary,
        request.id
    )
}

fn approved_note(request: &Request, approver: &str, secs: u64) -> String {
    format!(
        "[AgentMux] {} approved your request to {}. The person has {secs} s to veto it; after that it happens.",
        quote_in_system_note(approver),
        request.summary
    )
}

fn denied_note(request: &Request, approver: &str) -> String {
    let why = if request.deny_reason.is_empty() { String::new() } else { format!(" Their reason, as they wrote it: \"{}\".", quote_in_system_note(&request.deny_reason)) };
    format!("[AgentMux] {} denied your request to {}.{why}", quote_in_system_note(approver), request.summary)
}

/// A request as an agent reads it. `reason` and `deny_reason` are the agents' own words.
fn shown(r: &Request) -> Value {
    let mut v = json!({
        "id": r.id,
        "op": r.op,
        "summary": r.summary,
        "status": r.status,
        "requester": r.requester_slug,
        "also_asked_by": r.also_asked_by.iter().map(|a| a.slug.clone()).collect::<Vec<_>>(),
        "reason": r.reason,
        "decided_by": r.decided_by,
        "deny_reason": r.deny_reason,
    });
    if let Some(w) = window::status(&r.id) {
        v["deadline_ms"] = json!(w.deadline_ms);
        if let Some(e) = w.error {
            v["error"] = json!(e);
        }
    }
    v
}

#[cfg(test)]
#[path = "sensitive_handlers_tests.rs"]
mod tests;
