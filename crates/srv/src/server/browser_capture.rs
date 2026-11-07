// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `capture`: a secret a web page shows once goes straight into the Vault
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! The host reads the element a reference names and marks it as holding a Vault
//! value; srv stores what it read as a new entry, or as the new value of one the
//! agent names, and answers with the entry's id. The value never reaches the agent.
//! The entry keeps the page's origin, which the host takes from its own frame map,
//! as its `service` or `site`, and the Activity says "captured from <origin>".
//!
//! With `download`, the reference is the button or link that downloads a file (a
//! GitHub App's private key): the host clicks it and takes the file off the network
//! before it becomes a download, so it is never written to disk, and srv stores its
//! text (a `note` by default). A click that commits something (a form's submit)
//! waits for the person's approval in the pane, as an `act` click does.
//!
//! A new entry asks nobody: saving a value takes nothing out of the Vault, and the
//! page already showed it. Replacing an existing entry's value asks the person each
//! time, as a capture from a command does (§4.2).

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule, UiAutomationAuth};

use crate::backend::vault::capture::{self, Found, Target};
use crate::backend::vault::uses::{Ended, UseRequest};

use super::browser_actions::{note, How, Kind};
use super::browser_click_approval::{approve_click, ClickRefused};
use super::ui_handlers::{err_response, proxy_value, target_block_id};
use super::AppState;

const HOST_WAIT: Duration = Duration::from_secs(30);
/// How long the person has to allow a capture that replaces a value.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
/// The longest the host waits for a download after its click (2 minutes), and
/// time for the click and the reply.
const DOWNLOAD_WAIT: Duration = Duration::from_secs(150);

#[derive(Debug, Deserialize)]
pub(crate) struct CaptureRequest {
    auth: UiAutomationAuth,
    #[serde(default)]
    pane: Option<String>,
    #[serde(rename = "ref", default)]
    ref_: Option<String>,
    /// A new entry: its kind (`api-key` by default), label, and plain fields.
    #[serde(rename = "as", default)]
    into: Option<CaptureAs>,
    /// Or an existing entry, whose value this replaces.
    #[serde(default)]
    entry: Option<String>,
    /// The file the element downloads, instead of the element's text.
    #[serde(default)]
    download: bool,
    /// How long to wait for the download after the click (at most 2 minutes).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Why the agent keeps what the page shows: one line of 8 to 300 characters, required.
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CaptureAs {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
    #[serde(default)]
    expires: Option<String>,
}

/// The plain field a kind keeps a page's origin in.
fn origin_field(kind: &str) -> Option<&'static str> {
    match kind {
        "api-key" => Some("service"),
        "login" => Some("site"),
        _ => None,
    }
}

/// The capture rule the request describes, checked like a command's.
fn rule_of(req: &CaptureRequest) -> Result<CaptureRule, String> {
    let into = match (&req.entry, &req.into) {
        (Some(_), Some(_)) => return Err("name an existing entry (entry) or a new one (as), not both".into()),
        (Some(id), None) => CaptureInto { id: Some(id.clone()), ..Default::default() },
        (None, Some(a)) => {
            // A file is kept as a note's text; a value as an API key's.
            let kind = a.kind.clone().unwrap_or_else(|| if req.download { "note" } else { "api-key" }.into());
            CaptureInto { kind: Some(kind), label: Some(a.label.clone()), ..Default::default() }
        }
        (None, None) => return Err("say where it goes: as {kind, label} for a new entry, or entry for an existing one".into()),
    };
    Ok(CaptureRule { into, from: CaptureFrom::Stdout("all".into()) })
}

/// Who is asking to replace an entry's value with what a page shows, and why.
struct Asking<'a> {
    state: &'a AppState,
    block_id: &'a str,
    agent: &'a str,
    agent_pane: &'a str,
    origin: &'a str,
    reason: &'a str,
}

/// Asks the person to let `entry` (id, label, part, kind) be replaced by what the page shows. True when nobody
/// answered and it went ahead; a refusal is the answer to give the agent, after the denied try is recorded.
async fn ask_to_replace(who: &Asking<'_>, entry: &(String, String, &'static str, String)) -> Result<bool, Response> {
    let (id, label, part, kind) = entry;
    let (agent, origin) = (who.agent, who.origin);
    let request = UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: who.agent_pane.to_string(),
        entry_id: id.clone(),
        entry_label: label.clone(),
        what: part.to_string(),
        origin: origin.to_string(),
        field: String::new(),
        offer_always: false,
        reason: "what the page shows replaces the value".into(),
        agent_reason: who.reason.to_string(),
        kind: kind.clone(),
        replaces: true,
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    };
    let headline = format!("{agent} wants to replace {label} with what {origin} shows");
    let (ended, _) = super::vault_ask::ask(who.state, who.block_id, request, ANSWER_WAIT, &headline).await;
    if !ended.allowed() {
        let outcome = ended.outcome();
        let target = format!("from {origin}");
        let line = crate::backend::vault::Line { entry_id: id, label, actor: agent, action: capture::CAPTURED, target: &target, outcome, reason: who.reason };
        crate::backend::vault::record_line(&who.state.identity_store, line);
        super::vault_handlers::publish(who.state);
        return Err(err_response(StatusCode::FORBIDDEN, format!("the person didn't allow it ({outcome}): nothing was saved")));
    }
    Ok(ended == Ended::WentAhead)
}

/// `POST /api/v1/ui/browser/capture`.
pub(crate) async fn handle_browser_capture(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<CaptureRequest>,
) -> Response {
    let Some(ref_) = req.ref_.clone().filter(|r| !r.trim().is_empty()) else {
        let what = if req.download { "the button or link that downloads the file" } else { "the element that shows the secret" };
        return err_response(StatusCode::BAD_REQUEST, format!("capture needs the ref of {what} (from snapshot or find)"));
    };
    let reason = match crate::backend::vault::history::require_reason(req.reason.as_deref()) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent: String = req.auth.agent_id.chars().take(64).collect();
    let agent_pane = super::vault_ask::agent_pane(&state, caller.as_deref(), &req.auth);
    let rule = match rule_of(&req) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let (store, a) = (state.identity_store.clone(), agent.clone());
    let mut plan = match tokio::task::spawn_blocking(move || capture::plan(&store, &[rule], &a, "")).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return err_response(StatusCode::BAD_REQUEST, e),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    plan.reason = reason.clone();
    // Replacing the person's value asks them, naming the page once it is known.
    let replaces = match &plan.rules[0].into {
        Target::Existing { id, label, part, kind, .. } => Some((id.clone(), label.clone(), *part, kind.clone())),
        Target::New { .. } => None,
    };

    // Held until the approved click is made, if one waits for the person.
    let mut _approved_click = None;
    let read = if req.download {
        let body = |approved: Option<&Value>| json!({ "block_id": block_id, "ref": ref_, "timeout_ms": req.timeout_ms, "approved_as": approved });
        match proxy_value(&state, "capture_download", body(None), DOWNLOAD_WAIT).await {
            Ok(first) => match first.get("needs_approval").cloned() {
                Some(ask) => {
                    let log = note(&block_id, &agent, Kind::Capture).target(&ref_);
                    match approve_click(&state, &block_id, &agent, &ask).await {
                        Ok((waiting, _)) => {
                            _approved_click = Some(waiting);
                            proxy_value(&state, "capture_download", body(Some(&ask)), DOWNLOAD_WAIT).await
                        }
                        Err(ClickRefused::NotAsked(code, e)) => return err_response(code, e),
                        Err(ClickRefused::Asked(how, code, e)) => {
                            log.how(how.unwrap_or(How::Refused(e.clone()))).record(&state);
                            return err_response(code, format!("nothing was clicked or saved: {e}"));
                        }
                    }
                }
                None => Ok(first),
            },
            Err(e) => Err(e),
        }
    } else {
        proxy_value(&state, "capture_value", json!({ "block_id": block_id, "ref": ref_ }), HOST_WAIT).await
    };
    let read = match read {
        Ok(v) => v,
        Err((code, e)) => return err_response(code, e),
    };
    let file = read.get("name").and_then(Value::as_str).map(|n| n.chars().take(200).collect::<String>());
    let (value, origin) = match take_read(read) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    if origin.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "AgentMux can't tell which site this page is, so nothing was saved".into());
    }
    let mut went_ahead = false;
    if let Some(entry) = &replaces {
        let asking = Asking { state: &state, block_id: &block_id, agent: &agent, agent_pane: &agent_pane, origin: &origin, reason: &reason };
        match ask_to_replace(&asking, entry).await {
            Ok(ahead) => went_ahead = ahead,
            Err(refused) => return refused,
        }
    }

    let (store, given, from) = (state.identity_store.clone(), req.into, origin.clone());
    let stored = tokio::task::spawn_blocking(move || store_captured(&store, plan, value, &from, given.as_ref())).await;
    super::vault_handlers::publish(&state);
    // Only now that it is stored is the element marked: a refused capture leaves no
    // mark (one that would keep eval refused on the tab for nothing).
    let mut marked = false;
    // A downloaded file is on no element to mark.
    if matches!(stored, Ok(Ok(_))) && !req.download {
        match proxy_value(&state, "capture_mark", json!({ "block_id": block_id, "ref": ref_ }), HOST_WAIT).await {
            Ok(_) => marked = true,
            Err((_, e)) => tracing::warn!(error = %e, "[vault] a captured element couldn't be marked"),
        }
    }
    // The pane's own Log: what was captured, from where, never the value.
    let log = note(&block_id, &agent, Kind::Capture).target(&ref_);
    match &stored {
        Ok(Ok((_, label))) => {
            let what = file.as_deref().map(|f| format!("{label} (the file {f})")).unwrap_or_else(|| label.clone());
            log.detail(&format!("{what}, captured from {origin}")).how(How::Allowed).record(&state)
        }
        Ok(Err(e)) => log.how(How::Refused(e.chars().take(160).collect())).record(&state),
        Err(_) => {}
    }
    match stored {
        Ok(Ok((id, label))) => {
            let note = if file.is_some() { DOWNLOADED_NOTE } else { captured_note(marked) };
            let out = json!({ "ok": true, "id": id, "label": label, "captured_from": origin, "file": file, "marked": marked, "note": note });
            (StatusCode::OK, Json(super::vault_ask::with_approval(out, went_ahead))).into_response()
        }
        Ok(Err(e)) => err_response(StatusCode::BAD_REQUEST, format!("nothing was saved: {e}")),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// What the agent is told once a downloaded file is saved.
const DOWNLOADED_NOTE: &str = "Saved in the Vault; you never see the file. It was never written to disk: give it to a command with Terminal run's secret_files.";

/// What the agent is told about the page once the value is saved: what keeps
/// hiding it there depends on whether the element could be marked.
fn captured_note(marked: bool) -> &'static str {
    if marked {
        "Saved in the Vault; you never see the value. While it is on the page, screenshots black it out and eval is refused on this tab; snapshots keep hiding it while it shows as a secret, and always in a field."
    } else {
        "Saved in the Vault; you never see the value. The page changed before the element could be marked, so only a value with a secret's shape stays hidden on it: don't read the element again, and navigate away when you're done."
    }
}

/// Stores a captured value: a new entry starts with the page's origin as its
/// `service` or `site` (unless the agent gave one), the fields and expiry date the
/// agent gave; an existing one gets it as its new value, the old one kept 7 days.
/// Returns the entry's id and label.
pub(crate) fn store_captured(
    store: &crate::backend::storage::store::Store,
    mut plan: capture::Capture,
    value: Zeroizing<String>,
    origin: &str,
    given: Option<&CaptureAs>,
) -> Result<(String, String), String> {
    plan.cmd = format!("from {origin}");
    if let (Target::New { kind, fields, .. }, Some(given)) = (&mut plan.rules[0].into, given) {
        fields.extend(given.fields.clone());
        if let Some(date) = given.expires.as_ref().filter(|d| !d.trim().is_empty()) {
            fields.insert("expires".into(), date.clone());
        }
        if let Some(f) = origin_field(kind) {
            fields.entry(f.to_string()).or_insert_with(|| origin.to_string());
        }
    }
    let ids = plan.store(store, &[Found { value, printed: None }])?;
    let label = match &plan.rules[0].into {
        Target::New { label, .. } | Target::Existing { label, .. } => label.clone(),
    };
    Ok((ids.into_iter().next().unwrap_or_default(), label))
}

/// The value and origin from the host's answer, the value moved out of the JSON
/// into memory that is wiped when dropped.
fn take_read(mut v: Value) -> Result<(Zeroizing<String>, String), String> {
    let origin = v.get("origin").and_then(Value::as_str).unwrap_or("").to_string();
    match v.get_mut("value").map(Value::take) {
        Some(Value::String(s)) => Ok((Zeroizing::new(s), origin)),
        _ => Err("the browser didn't send the element's value".into()),
    }
}

#[cfg(test)]
#[path = "browser_capture_tests.rs"]
mod tests;
