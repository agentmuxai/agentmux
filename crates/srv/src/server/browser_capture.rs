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
use crate::backend::vault::pace::{admit, refund, Act};

use super::browser_actions::{note, How, Kind};
use super::browser_capture_read::{begin, check_shape, read_page, what_to_read};
use agentmux_common::capture_shape::CaptureShape;
use super::ui_handlers::{err_response, proxy_value, target_block_id};
use super::AppState;

pub(super) const HOST_WAIT: Duration = Duration::from_secs(30);
/// The longest the host waits for a download after its click (2 minutes), and
/// time for the click and the reply.
pub(super) const DOWNLOAD_WAIT: Duration = Duration::from_secs(150);

#[derive(Debug, Deserialize)]
pub(crate) struct CaptureRequest {
    pub(super) auth: UiAutomationAuth,
    #[serde(default)]
    pub(super) pane: Option<String>,
    #[serde(rename = "ref", default)]
    pub(super) ref_: Option<String>,
    /// A card shown on the page: each part's key (`number`, `exp`, `exp_month`, `exp_year`, `cvc`, `name`, `zip`) and the
    /// ref of the element that shows it. Saved whole as a new card (`as`), instead of `ref`.
    #[serde(default)]
    pub(super) card: Option<BTreeMap<String, String>>,
    /// A new entry: its kind (`api-key` by default), label, and plain fields.
    #[serde(rename = "as", default)]
    pub(super) into: Option<CaptureAs>,
    /// Or an existing entry, whose value this replaces.
    #[serde(default)]
    pub(super) entry: Option<String>,
    /// The file the element downloads, instead of the element's text.
    #[serde(default)]
    pub(super) download: bool,
    /// How long to wait for the download after the click (at most 2 minutes).
    #[serde(default)]
    pub(super) timeout_ms: Option<u64>,
    /// Why the agent keeps what the page shows: one line of 8 to 300 characters, required.
    #[serde(default)]
    pub(super) reason: Option<String>,
    /// Text with no reference of its own: the value shown beside this label. Instead of `ref`.
    #[serde(default)]
    pub(super) near: Option<String>,
    /// What the value must be, or nothing is saved: a short prefix, one of the named shapes, a length range.
    #[serde(default)]
    pub(super) prefix: Option<String>,
    #[serde(default)]
    pub(super) shape: Option<String>,
    #[serde(default)]
    pub(super) min_length: Option<u32>,
    #[serde(default)]
    pub(super) max_length: Option<u32>,
    /// The reference is the page's own copy button: its value is taken from the page's copy, never the clipboard.
    #[serde(default)]
    pub(super) copy: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CaptureAs {
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) label: String,
    #[serde(default)]
    pub(super) fields: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) expires: Option<String>,
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

/// `POST /api/v1/ui/browser/capture`.
/// `POST /api/v1/ui/browser/capture`: a card the page shows is saved whole, anything else is one value.
pub(crate) async fn handle_capture(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<CaptureRequest>,
) -> Response {
    if req.card.is_some() {
        return super::browser_capture_card::capture_card(state, caller, req).await;
    }
    handle_browser_capture(State(state), caller, Json(req)).await
}

/// What a capture read from the page: the value, the page's origin, the file's name when a file was taken, and the element's
/// reference (text found beside a label has one now, which marks it once it is saved).
struct Taken {
    value: Zeroizing<String>,
    origin: String,
    file: Option<String>,
    ref_: String,
}

/// Reads the element and checks what came back, or the answer for the agent when it can't be saved.
async fn read_for_capture(state: &AppState, req: &CaptureRequest, block_id: &str, agent: &str, ref_: &str, shape: Option<&CaptureShape>) -> Result<(Taken, Option<super::browser_attention::Waiting>), Response> {
    let (read, approved_click) = match read_page(state, req, block_id, agent, (ref_, shape)).await {
        Ok(r) => r,
        Err(refused) => return Err(super::browser_capture_read::counted(refused, shape, agent)),
    };
    let file = read.get("name").and_then(Value::as_str).map(|n| n.chars().take(200).collect::<String>());
    let ref_ = read.get("ref").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| ref_.to_string());
    let (value, origin) = take_read(read).map_err(|e| err_response(StatusCode::BAD_GATEWAY, e))?;
    check_shape(shape, &value, agent).map_err(|e| err_response(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    if origin.is_empty() {
        return Err(err_response(StatusCode::BAD_REQUEST, "AgentMux can't tell which site this page is, so nothing was saved".into()));
    }
    Ok((Taken { value, origin, file, ref_ }, approved_click))
}

async fn handle_browser_capture(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<CaptureRequest>,
) -> Response {
    let (ref_, shape) = match what_to_read(&req) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
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
    if let Err(e) = begin(shape.as_ref(), &agent) {
        return err_response(StatusCode::TOO_MANY_REQUESTS, e);
    }
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
    // Nobody is asked to allow the write, replacing a value included: the history line is the record, the old value is kept
    // 7 days, and the agent's pace bounds how much it does in an hour.
    let act = match &plan.rules[0].into {
        Target::Existing { id, .. } => Act::Replace(id.clone()),
        Target::New { .. } => Act::New,
    };
    let spent = [act];
    if let Err(e) = admit(&agent, &spent) {
        if let (Target::Existing { id, label, .. }, true) = (&plan.rules[0].into, crate::backend::vault::pace::may_note_refusal(&agent)) {
            let line = crate::backend::vault::Line { entry_id: id, label, actor: &agent, action: capture::CAPTURED, target: "from a page", outcome: "refused: pace", reason: &reason };
            crate::backend::vault::record_line(&state.identity_store, line);
        }
        return err_response(StatusCode::TOO_MANY_REQUESTS, e);
    }

    // Held until the approved click is made, if one waits for the person.
    let (taken, _approved_click) = match read_for_capture(&state, &req, &block_id, &agent, &ref_, shape.as_ref()).await {
        Ok(r) => r,
        Err(refused) => {
            // Nothing was stored: a capture that failed does not use up the agent's pace.
            refund(&agent, &spent);
            return refused;
        }
    };
    let Taken { value, origin, file, ref_ } = taken;
    let (store, given, from) = (state.identity_store.clone(), req.into, origin.clone());
    let stored = tokio::task::spawn_blocking(move || store_captured(&store, plan, value, &from, given.as_ref())).await;
    if !matches!(stored, Ok(Ok(_))) {
        refund(&agent, &spent);
    }
    super::vault_handlers::publish(&state);
    // Only now that it is stored is the element marked: a refused capture leaves no
    // mark (one that would keep eval refused on the tab for nothing).
    let mut marked = false;
    // A downloaded file, and what a copy button copied, are on no element to mark.
    if matches!(stored, Ok(Ok(_))) && !req.download && !req.copy {
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
            let note = if file.is_some() { DOWNLOADED_NOTE } else if req.copy { COPIED_NOTE } else { captured_note(marked) };
            let out = json!({ "ok": true, "id": id, "label": label, "captured_from": origin, "file": file, "marked": marked, "copied": req.copy, "note": note });
            (StatusCode::OK, Json(out)).into_response()
        }
        Ok(Err(e)) => err_response(StatusCode::BAD_REQUEST, format!("nothing was saved: {e}")),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// What the agent is told once a downloaded file is saved.
const DOWNLOADED_NOTE: &str = "Saved in the Vault; you never see the file. It was never written to disk: give it to a command with Terminal run's secret_files.";

/// What the agent is told once a value taken from the page's copy button is saved.
const COPIED_NOTE: &str = "Saved in the Vault; you never see the value. It was taken from the page's own copy action: the system clipboard was never written, and nothing was left on it.";

/// What the agent is told about the page once the value is saved: what keeps
/// hiding it there depends on whether the element could be marked.
pub(super) fn captured_note(marked: bool) -> &'static str {
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
    let ids = plan.store(store, &[Found { value, printed: None, also: Vec::new() }])?;
    let label = match &plan.rules[0].into {
        Target::New { label, .. } | Target::Existing { label, .. } => label.clone(),
    };
    Ok((ids.into_iter().next().unwrap_or_default(), label))
}

/// The value and origin from the host's answer, the value moved out of the JSON
/// into memory that is wiped when dropped.
pub(super) fn take_read(mut v: Value) -> Result<(Zeroizing<String>, String), String> {
    let origin = v.get("origin").and_then(Value::as_str).unwrap_or("").to_string();
    match v.get_mut("value").map(Value::take) {
        Some(Value::String(s)) => Ok((Zeroizing::new(s), origin)),
        _ => Err("the browser didn't send the element's value".into()),
    }
}

#[cfg(test)]
#[path = "browser_capture_tests.rs"]
mod tests;
