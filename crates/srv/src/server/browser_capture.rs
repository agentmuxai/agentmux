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
use crate::backend::vault::uses::{Decision, UseRequest};

use super::browser_actions::{note, How, Kind};
use super::browser_click_approval::{approve_click, ClickRefused};
use super::ui_handlers::{err_response, proxy_value, target_block_id};
use super::AppState;

const HOST_WAIT: Duration = Duration::from_secs(30);
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
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent: String = req.auth.agent_id.chars().take(64).collect();
    let rule = match rule_of(&req) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let (store, a) = (state.identity_store.clone(), agent.clone());
    let plan = match tokio::task::spawn_blocking(move || capture::plan(&store, &[rule], &a, "")).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return err_response(StatusCode::BAD_REQUEST, e),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    // Replacing the person's value asks them, naming the page once it is known.
    let replaces = match &plan.rules[0].into {
        Target::Existing { id, label, part, .. } => Some((id.clone(), label.clone(), *part)),
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
    if let Some((id, label, part)) = &replaces {
        let request = UseRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent: agent.clone(),
            entry_id: id.clone(),
            entry_label: label.clone(),
            what: part.to_string(),
            origin: origin.clone(),
            field: String::new(),
            offer_always: false,
            reason: "what the page shows replaces the value".into(),
            needs_value: false,
            kind: String::new(),
            command: String::new(),
            cwd: String::new(),
            replaces: true,
            requested_ms: agentmux_common::time::now_ms_u64(),
        };
        match super::shell_secrets::ask(&state, &block_id, request, &format!("{agent} wants to replace {label} with what {origin} shows")).await {
            Some(Decision::Once | Decision::Always) => {}
            other => {
                let outcome = if other.is_none() { "timed out" } else { "denied" };
                crate::backend::vault::record(&state.identity_store, id, label, &agent, capture::CAPTURED, &format!("from {origin}"), outcome);
                super::vault_handlers::publish(&state);
                return err_response(StatusCode::FORBIDDEN, format!("the person didn't allow it ({outcome}): nothing was saved"));
            }
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
        Ok(Ok((id, label))) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "id": id,
                "label": label,
                "captured_from": origin,
                "file": file,
                "marked": marked,
                "note": if file.is_some() { DOWNLOADED_NOTE } else { captured_note(marked) },
            })),
        )
            .into_response(),
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
