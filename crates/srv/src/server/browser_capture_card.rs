// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `capture` of a card: a page that shows a card once (a virtual-card page, a bank's "show card details")
//! goes into the Vault as one new `card` entry, each part read from the element the agent names for it
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md, §5 item 4).
//!
//! The host reads each element's whole text or value (a card's number shows as "4242 4242 4242 4242", an expiry as
//! "03/30"), srv settles them like any card (digits, month and year, brand, reminder date) and stores one entry in one
//! transaction, then the host marks every element it read as holding a Vault value. The parts never reach the agent;
//! the answer is the entry's id and the names of the parts. The parts must all be on one site: the host says which
//! site each frame is, never the page.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule};

use super::browser_actions::{note, How, Kind};
use super::browser_capture::{captured_note, take_read, CaptureAs, CaptureRequest, HOST_WAIT};
use super::ui_handlers::{err_response, proxy_value, target_block_id};
use super::AppState;
use crate::backend::vault::capture::{self, Target};

/// The capture rule a card request describes: a new card, each part named, none of them read from an output. Checked
/// like a command's (`capture::plan`): known parts, the number, the expiry given one way.
pub(super) fn card_rule(refs: &BTreeMap<String, String>, given: Option<&CaptureAs>) -> Result<CaptureRule, String> {
    let Some(given) = given else {
        return Err("say where the card goes: as {kind: \"card\", label}".into());
    };
    if given.kind.as_deref().is_some_and(|k| k.trim() != "card") {
        return Err("card is for a card: leave kind out, or give \"card\"".into());
    }
    if let Some((key, _)) = refs.iter().find(|(_, r)| r.trim().is_empty()) {
        return Err(format!("card.{key} needs the ref of the element that shows it"));
    }
    let from = CaptureFrom::Parts(refs.keys().map(|k| (k.clone(), CaptureFrom::Stdout("all".into()))).collect());
    Ok(CaptureRule { into: CaptureInto { kind: Some("card".into()), label: Some(given.label.clone()), ..Default::default() }, from })
}

/// What each part's element read: the value, and the origin of the frame it is in, checked to be one site.
pub(super) fn gathered(reads: Vec<(String, Zeroizing<String>, String)>) -> Result<(BTreeMap<String, Zeroizing<String>>, String), String> {
    let mut origin = String::new();
    let mut values = BTreeMap::new();
    for (key, value, from) in reads {
        if from.is_empty() {
            return Err("AgentMux can't tell which site this page is, so nothing was saved".into());
        }
        if origin.is_empty() {
            origin = from;
        } else if origin != from {
            return Err(format!("the card's parts are on more than one site ({origin} and {from}), so nothing was saved: capture a card whose parts are on one site"));
        }
        values.insert(key, value);
    }
    Ok((values, origin))
}

pub(super) async fn capture_card(state: AppState, caller: Option<axum::Extension<crate::server::caller::Caller>>, req: CaptureRequest) -> Response {
    let Some(refs) = req.card.clone() else {
        return err_response(StatusCode::BAD_REQUEST, "card names the elements that show the card's parts".into());
    };
    if req.entry.is_some() || req.download || req.ref_.is_some() {
        return err_response(StatusCode::BAD_REQUEST, "a card is saved whole as a new entry: give card and as, not ref, entry or download".into());
    }
    let reason = match crate::backend::vault::history::require_reason(req.reason.as_deref()) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent: String = req.auth.agent_id.chars().take(64).collect();
    let rule = match card_rule(&refs, req.into.as_ref()) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let (store, a) = (state.identity_store.clone(), agent.clone());
    let mut plan = match tokio::task::spawn_blocking(move || capture::plan(&store, &[rule], &a, "")).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return err_response(StatusCode::BAD_REQUEST, e),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    plan.reason = reason;

    let mut reads = Vec::with_capacity(refs.len());
    for (key, r) in &refs {
        let read = proxy_value(&state, "capture_value", json!({ "block_id": block_id, "ref": r, "whole": true }), HOST_WAIT).await;
        let read = match read {
            Ok(v) => v,
            Err((code, e)) => return err_response(code, format!("card.{key}: {e}; nothing was saved")),
        };
        match take_read(read) {
            Ok((value, origin)) => reads.push((key.clone(), value, origin)),
            Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
        }
    }
    let (values, origin) = match gathered(reads) {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let parts: Vec<String> = values.keys().cloned().collect();

    let (store, given, from) = (state.identity_store.clone(), req.into, origin.clone());
    let stored = tokio::task::spawn_blocking(move || store_card(&store, plan, values, &from, given.as_ref())).await;
    super::vault_handlers::publish(&state);
    // Only now that it is stored are the elements marked: a refused capture leaves no mark.
    let mut marked = 0;
    if matches!(stored, Ok(Ok(_))) {
        for r in refs.values() {
            match proxy_value(&state, "capture_mark", json!({ "block_id": block_id, "ref": r }), HOST_WAIT).await {
                Ok(_) => marked += 1,
                Err((_, e)) => tracing::warn!(error = %e, "[vault] a captured card element couldn't be marked"),
            }
        }
    }
    let log = note(&block_id, &agent, Kind::Capture).target(&format!("card, {} parts", parts.len()));
    match &stored {
        Ok(Ok((_, label))) => log.detail(&format!("{label} (a card), captured from {origin}")).how(How::Allowed).record(&state),
        Ok(Err(e)) => log.how(How::Refused(e.chars().take(160).collect())).record(&state),
        Err(_) => {}
    }
    match stored {
        Ok(Ok((id, label))) => {
            let out = json!({ "ok": true, "id": id, "label": label, "captured_from": origin, "parts": parts, "marked": marked == refs.len(), "note": captured_note(marked == refs.len()) });
            (StatusCode::OK, Json(out)).into_response()
        }
        Ok(Err(e)) => err_response(StatusCode::BAD_REQUEST, format!("nothing was saved: {e}")),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Stores the card: the fields and expiry date the agent gave go with it. Returns the entry's id and label.
pub(super) fn store_card(
    store: &crate::backend::storage::store::Store,
    mut plan: capture::Capture,
    values: BTreeMap<String, Zeroizing<String>>,
    origin: &str,
    given: Option<&CaptureAs>,
) -> Result<(String, String), String> {
    plan.cmd = format!("from {origin}");
    if let (Target::New { fields, .. }, Some(given)) = (&mut plan.rules[0].into, given) {
        fields.extend(given.fields.clone());
        if let Some(date) = given.expires.as_ref().filter(|d| !d.trim().is_empty()) {
            fields.insert("expires".into(), date.clone());
        }
    }
    let found = capture::card_found(values)?;
    let ids = plan.store(store, &[found])?;
    let Target::New { label, .. } = &plan.rules[0].into else {
        return Err("a card is a new entry".into());
    };
    Ok((ids.into_iter().next().unwrap_or_default(), label.clone()))
}

#[cfg(test)]
#[path = "browser_capture_card_tests.rs"]
mod tests;
