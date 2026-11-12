// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `fill` with `vault: "<card id>"` (or `"card"`, the default card): one question
//! for the person, then every card field the form has is filled
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! The host says which fields of the form look like parts of a card and how each takes its
//! text (`card_form`); the person is asked once, naming the card and the parts; then each field
//! is filled through the same `vault_fill` as a single field, which checks it again. The answer
//! lists the parts filled and the parts the form wanted that were left, by name; a value is in
//! no answer, error or record. A card that has expired is asked about every time: no "Always"
//! is offered for it and none already given counts.

use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::*;
use crate::backend::vault::card_fill::{self, FormField, Shaped};
use crate::backend::vault::card_target::{self, CardTarget};
use crate::backend::vault::seal::Parts;
use crate::server::caller::Caller;

/// A field of the form the host matched to a part of a card.
struct CardField {
    ref_: String,
    fingerprint: String,
    shape: FormField,
}

/// The slots that make a form a card form: a ZIP or a name alone is a single-field fill's.
const MAIN_SLOTS: [&str; 5] = ["card-number", "card-expiry", "card-exp-month", "card-exp-year", "card-cvc"];

fn parse_fields(form: &Value) -> Vec<CardField> {
    let text = |f: &Value, k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut out: Vec<CardField> = Vec::new();
    for f in form.get("fields").and_then(Value::as_array).into_iter().flatten() {
        let slot = text(f, "slot");
        if card_fill::part_of_slot(&slot).is_none() || out.iter().any(|o| o.shape.slot == slot) {
            continue;
        }
        let shape = FormField {
            slot,
            select: f.get("select").and_then(Value::as_bool).unwrap_or(false),
            max_length: f.get("max_length").and_then(Value::as_u64).map(|m| m as usize),
            placeholder: text(f, "placeholder"),
            pattern: text(f, "pattern"),
        };
        out.push(CardField { ref_: text(f, "ref"), fingerprint: text(f, "fingerprint"), shape });
    }
    out
}

/// The parts the form asks for, by name, once each ("number, expiry, security code").
fn part_names(fields: &[CardField]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for f in fields {
        let n = card_fill::part_name(&f.shape.slot);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

/// Takes the call when `vault` names a whole card, `None` for any other fill.
pub(super) async fn whole_card(state: &AppState, caller: Option<&Caller>, req: &VaultFillRequest, block_id: &str, agent: &str, reason: &str) -> Option<Response> {
    let wanted = req.vault.clone()?;
    let (store, who) = (state.identity_store.clone(), agent.to_string());
    let found = tokio::task::spawn_blocking(move || card_target::card_target(&store, &wanted, Some(&who))).await;
    match found {
        Ok(Ok(Some(card))) => Some(fill_card(state, caller, req, block_id, agent, reason, card).await),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => Some(err_response(StatusCode::BAD_REQUEST, e)),
        Err(e) => Some(err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

fn card_question(agent: &str, pane: String, card: &CardTarget, origin: &str, names: &[&str], ask: (bool, &str), agent_reason: &str) -> UseRequest {
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: pane,
        entry_id: card.fill.id.clone(),
        entry_label: card.fill.label.clone(),
        what: card.describe(),
        origin: origin.to_string(),
        field: format!("the card fields: {}", names.join(", ")),
        offer_always: ask.0,
        reason: ask.1.to_string(),
        agent_reason: agent_reason.to_string(),
        kind: "card".to_string(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    }
}

/// What fills one field: the host is told the text, or the month or year to choose, and the field it was approved for.
async fn fill_field(state: &AppState, block_id: &str, origin: &str, field: &CardField, text: &str, choose: Option<&str>) -> Result<Value, (StatusCode, String)> {
    let body = json!({
        "block_id": block_id,
        "ref": field.ref_,
        "text": text,
        "expect_origin": origin,
        "expect_fingerprint": field.fingerprint,
        "choose": choose,
    });
    proxy_value(state, "vault_fill", body, HOST_WAIT).await
}

/// An error with no part of the card in it: a page's or a parser's message can quote what it was given.
fn without_card(e: &str, parts: &Parts) -> String {
    let mut out = e.to_string();
    for v in parts.0.values().filter(|v| v.chars().count() >= 3) {
        out = out.replace(v.as_str(), "[hidden]");
    }
    out
}

/// What the fill did, by part name: never a value.
#[derive(Default)]
struct Outcome {
    filled: Vec<&'static str>,
    left: Vec<Value>,
    failed: Vec<Value>,
    flagged: bool,
    reformatted: bool,
}

async fn fill_all(state: &AppState, block_id: &str, origin: &str, fields: &[CardField], parts: &Parts) -> Outcome {
    let mut done = Outcome::default();
    for f in fields {
        let part = card_fill::part_name(&f.shape.slot);
        let (text, choose): (Zeroizing<String>, Option<&str>) = match card_fill::shape(&f.shape, parts) {
            Shaped::Left(why) => {
                done.left.push(json!({ "part": part, "why": why }));
                continue;
            }
            Shaped::Text(t) => (t, None),
            Shaped::Choose(t) => (t, Some(if f.shape.slot == "card-exp-month" { "month" } else { "year" })),
        };
        match fill_field(state, block_id, origin, f, &text, choose).await {
            Ok(data) => {
                if !done.filled.contains(&part) {
                    done.filled.push(part);
                }
                // The page's own message can echo the value: the agent is told only that there is one.
                done.flagged |= data.get("validation").and_then(Value::as_str).is_some_and(|v| !v.is_empty());
                done.reformatted |= data.get("changed_by_page").and_then(Value::as_bool) == Some(true);
            }
            Err((_, e)) => done.failed.push(json!({ "part": part, "why": without_card(&e, parts) })),
        }
    }
    done
}

fn answer(card: &CardTarget, origin: &str, done: &Outcome) -> Value {
    let mut out = json!({
        // The entry's id and part names only: the brand, last four and expiry are the person's to see.
        "filled_from_vault": card.fill.id,
        "parts_filled": done.filled,
        "value": "[secret: hidden]",
        "site": origin,
    });
    if let Some(o) = out.as_object_mut() {
        if !done.left.is_empty() {
            o.insert("parts_left".into(), json!(done.left));
        }
        if !done.failed.is_empty() {
            o.insert("parts_failed".into(), json!(done.failed));
        }
        if done.flagged {
            o.insert("validation".into(), json!("the page flagged a value as invalid (its message isn't shown, since a page can put the value in it)"));
        }
        if done.reformatted {
            o.insert("note".into(), json!("the page shortened or reformatted a value"));
        }
    }
    out
}

async fn fill_card(state: &AppState, caller: Option<&Caller>, req: &VaultFillRequest, block_id: &str, agent: &str, reason: &str, mut card: CardTarget) -> Response {
    tracing::info!(agent = %agent, block_id = %block_id, entry = %card.fill.id, "[vault] a whole-card web form fill was asked for");
    let form = match proxy_value(state, "card_form", json!({ "block_id": block_id, "ref": req.ref_ }), HOST_WAIT).await {
        Ok(v) => v,
        Err((code, e)) => return err_response(code, e),
    };
    let origin = form.get("origin").and_then(Value::as_str).unwrap_or("").to_string();
    let fields = parse_fields(&form);
    if !fields.iter().any(|f| MAIN_SLOTS.contains(&f.shape.slot.as_str())) {
        return err_response(
            StatusCode::BAD_REQUEST,
            format!("no card number, expiry or security code field was found with that one: fill a single field with {}.<part> (number, exp_month, exp_year, cvc, name, zip), or take a new snapshot", card.fill.id),
        );
    }
    let names = part_names(&fields);
    let site = format!("{origin} (card form)");

    let store = state.identity_store.clone();
    let (entry, grant_origin) = (card.fill.id.clone(), origin.clone());
    let granted = !card.expired && tokio::task::spawn_blocking(move || store.vault_grant_check(&entry, &grant_origin).unwrap_or(false)).await.unwrap_or(false);
    let mut always = false;
    match plan(!origin.is_empty(), false, true, granted, true) {
        Plan::Refuse(why) => {
            record(state, block_id, &card.fill, agent, &site, "refused", reason);
            return err_response(StatusCode::BAD_REQUEST, why.to_string());
        }
        Plan::Go if !card.expired => {}
        asked => {
            let (offer_always, why) = match asked {
                Plan::Ask { offer_always, reason } if !card.expired => (offer_always, reason),
                _ => (false, "the card has expired"),
            };
            let pane = super::super::vault_ask::agent_pane(state, caller, &req.auth);
            let question = card_question(agent, pane, &card, &origin, &names, (offer_always, why), reason);
            let headline = format!("{agent} wants to fill your {} on {origin}", card.describe());
            let (ended, _) = super::super::vault_ask::ask(state, block_id, question, ANSWER_WAIT, &headline).await;
            if !ended.allowed() {
                record(state, block_id, &card.fill, agent, &site, ended.outcome(), reason);
                let code = if ended == Ended::NoAnswer { StatusCode::REQUEST_TIMEOUT } else { StatusCode::FORBIDDEN };
                return err_response(code, ended.refusal("nothing was typed"));
            }
            always = ended == Ended::Always && !card.expired;
        }
    }
    if always {
        let (store, entry, grant_origin, created) = (state.identity_store.clone(), card.fill.id.clone(), origin.clone(), card.fill.created_at);
        let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&entry, &grant_origin, agentmux_common::time::now_ms(), created)).await;
    }

    // The card leaves the sealed row only now.
    let store = state.identity_store.clone();
    let (entry, created) = (card.fill.id.clone(), card.fill.created_at);
    let parts = match tokio::task::spawn_blocking(move || card_target::parts_for_fill(&store, &entry, created)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            record(state, block_id, &card.fill, agent, &site, "failed", reason);
            return err_response(StatusCode::BAD_REQUEST, e);
        }
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let done = fill_all(state, block_id, &origin, &fields, &parts).await;
    drop(parts);

    card.fill.what = format!("card ({})", if done.filled.is_empty() { "nothing".to_string() } else { done.filled.join(", ") });
    if done.filled.is_empty() {
        record(state, block_id, &card.fill, agent, &site, "failed", reason);
        let why = done.failed.iter().chain(&done.left).map(|v| format!("{}: {}", v["part"].as_str().unwrap_or("?"), v["why"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join("; ");
        return err_response(StatusCode::BAD_REQUEST, format!("nothing was filled: {why}"));
    }
    record(state, block_id, &card.fill, agent, &site, "done", reason);
    let (store, entry) = (state.identity_store.clone(), card.fill.id.clone());
    let _ = tokio::task::spawn_blocking(move || store.vault_touch(&entry, agentmux_common::time::now_ms())).await;
    (StatusCode::OK, Json(json!({ "ok": true, "data": answer(&card, &origin, &done) }))).into_response()
}
