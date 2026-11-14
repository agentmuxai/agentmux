// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `request_value` with kind `card`: no card is saved for this form, so the person types the whole card in
//! the agent's question, and every card field the form has is filled from it
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §5 item 3).
//!
//! The question is the person's own form, never the agent's: number, expiry, security code, name and ZIP, and a box to
//! keep the card in the Vault under the label the agent gave. The form sends what was typed as one JSON text in the
//! answer's `value`; here it is made into the card's parts and settled like any card (digits, month and year), filled
//! through the same `vault_fill` as a saved card, and kept as one new entry only when the person ticked the box and the
//! page took it. The agent is told which parts were filled and, when kept, the new entry's id; never a value.

use std::collections::BTreeMap;

use super::*;

/// What the person's form may send: each key and the card part it is.
const TYPED_KEYS: [(&str, &str); 5] = [("number", "number"), ("exp", "expiry"), ("cvc", "cvc"), ("name", "name"), ("zip", "zip")];

/// The card the person typed, from the form's JSON text: the parts, settled, and the card's plain fields. A part left
/// empty is not in it; the number is required. Nothing in an error says what was typed.
fn typed_parts(text: &str) -> Result<(Parts, BTreeMap<String, String>), String> {
    let sent: BTreeMap<String, String> = serde_json::from_str(text).map_err(|_| "the card form didn't send a card".to_string())?;
    let mut parts = Parts::default();
    for (key, value) in &sent {
        let Some((_, part)) = TYPED_KEYS.iter().find(|(k, _)| k == key) else {
            return Err("the card form sent a part a card doesn't have".into());
        };
        if !value.trim().is_empty() {
            parts.0.insert(part.to_string(), value.trim().to_string());
        }
    }
    if !parts.0.contains_key("number") {
        return Err("the card needs its number".into());
    }
    let mut fields = BTreeMap::new();
    let typed_expiry = parts.0.contains_key("expiry");
    crate::backend::vault::card::settle(crate::backend::vault::card::KIND, &mut fields, &mut parts)?;
    // An expiry settle could not read as a month and a year is left as typed: it is refused, not filled.
    if typed_expiry && !parts.0.contains_key("exp_month") {
        return Err("the expiry isn't a month and a year, like 03/30".into());
    }
    Ok((parts, fields))
}

/// The question the person answers: the card form, for the parts this page's form asks for.
fn typed_question(agent: &str, pane: String, label: &str, form: &CardForm, agent_reason: &str) -> UseRequest {
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: pane,
        entry_label: label.to_string(),
        what: "card".to_string(),
        origin: form.origin.clone(),
        page: form.page.clone(),
        field: format!("the card fields: {}", form.names.join(", ")),
        offer_always: false,
        reason: "there is no card saved for this, so you type it here".to_string(),
        agent_reason: agent_reason.to_string(),
        needs_value: true,
        kind: "card".to_string(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    }
}

/// Keeps the card the person typed as one entry, under the label the agent gave.
async fn keep(state: &AppState, label: &str, parts: Parts) -> Result<String, String> {
    let (store, label) = (state.identity_store.clone(), label.to_string());
    let made = tokio::task::spawn_blocking(move || crate::backend::vault::create(&store, crate::backend::vault::NewEntry { kind: "card".into(), label, fields: Default::default(), tags: vec![], parts })).await;
    match made {
        Ok(Ok(id)) => {
            crate::server::vault_handlers::publish(state);
            Ok(id)
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(e.to_string()),
    }
}

/// Takes the call when the agent asks for a card the person types (`request_value`, kind `card`).
pub(super) async fn typed_card(state: &AppState, caller: Option<&Caller>, req: &VaultFillRequest, block_id: &str, agent: &str) -> Response {
    let label = req.request_label.clone().unwrap_or_default();
    let target = match crate::backend::vault::request_target("card", &label) {
        Ok(t) => t,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let form = match read_form(state, block_id, &req.ref_, "ask for one field's value with request_value and that value's own kind").await {
        Ok(f) => f,
        Err(response) => return response,
    };
    // The host names the site of the form's frame from its own frame map; without it the person can't be told where the card goes.
    if form.origin.is_empty() {
        record(state, block_id, &target, agent, "(an unknown site)", "refused", &req.reason.clone().unwrap_or_default());
        return err_response(StatusCode::BAD_REQUEST, "AgentMux can't tell which site this form is on, so the person is not asked for a card".to_string());
    }
    let site = format!("{} (card form)", form.grant_key);
    let agent_reason = req.reason.clone().unwrap_or_default();
    let question = typed_question(agent, crate::server::vault_ask::agent_pane(state, caller, &req.auth), target.label.as_str(), &form, &agent_reason);
    let headline = if form.page.is_empty() { format!("{agent} needs your card for the form on {}", form.origin) } else { format!("{agent} needs your card for a card field from {} on {}", form.origin, form.page) };
    let (ended, reply) = crate::server::vault_ask::ask(state, block_id, question, ANSWER_WAIT, &headline).await;
    if !ended.allowed() {
        record(state, block_id, &target, agent, &site, ended.outcome(), &agent_reason);
        let code = if ended == Ended::NoAnswer { StatusCode::REQUEST_TIMEOUT } else { StatusCode::FORBIDDEN };
        return err_response(code, ended.refusal("nothing was typed"));
    }
    let save = reply.as_ref().is_some_and(|a| a.save);
    let typed = reply.and_then(|a| a.value);
    let (parts, _) = match typed.as_deref().map(|v| typed_parts(v)) {
        Some(Ok(p)) => p,
        Some(Err(e)) => {
            record(state, block_id, &target, agent, &site, "failed", &agent_reason);
            return err_response(StatusCode::BAD_REQUEST, format!("{e}: nothing was typed"));
        }
        None => return err_response(StatusCode::BAD_REQUEST, "the person didn't type a card: nothing was typed".to_string()),
    };
    drop(typed);
    let done = fill_all(state, block_id, &form.origin, &form.fields, &parts).await;
    if done.filled.is_empty() {
        record(state, block_id, &target, agent, &site, "failed", &agent_reason);
        return err_response(StatusCode::BAD_REQUEST, format!("nothing was filled: {}", why_nothing(&done)));
    }
    let mut shown = target.clone();
    shown.what = format!("card ({})", done.filled.join(", "));
    record(state, block_id, &shown, agent, &site, "done (typed by the person)", &agent_reason);
    // Kept only if the person asked, and only now that the page has taken some of it.
    let kept = if save { Some(keep(state, &target.label, parts).await) } else { None };
    let mut out = answer(kept.as_ref().and_then(|k| k.as_ref().ok()).map(String::as_str).unwrap_or(""), &form.origin, &form.page, &done, ended == Ended::WentAhead);
    if let Some(o) = out.as_object_mut() {
        o.insert("typed_by_the_person".into(), json!(true));
        o.insert("kept_in_vault".into(), json!(matches!(kept, Some(Ok(_)))));
        if let Some(Err(e)) = &kept {
            o.insert("kept_note".into(), json!(format!("it was typed in but couldn't be kept in the Vault: {e}")));
        }
    }
    (StatusCode::OK, Json(json!({ "ok": true, "data": out }))).into_response()
}

#[cfg(test)]
#[path = "typed_tests.rs"]
mod tests;
