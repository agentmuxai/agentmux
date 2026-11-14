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
use crate::backend::vault::card_target::{self, Asked, CardTarget};
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
            input_type: text(f, "input_type"),
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
    if req.vault.is_none() && req.request_kind.as_deref().map(str::trim) == Some("card") {
        return Some(typed::typed_card(state, caller, req, block_id, agent).await);
    }
    let wanted = req.vault.clone()?;
    let (store, who) = (state.identity_store.clone(), agent.to_string());
    let found = tokio::task::spawn_blocking(move || {
        let asked = card_target::card_asked(&store, &wanted, Some(&who))?;
        let cards = if matches!(asked, Asked::NotACard) { Vec::new() } else { card_target::card_choices(&store, Some(&who))? };
        Ok::<_, String>((asked, cards))
    })
    .await;
    match found {
        Ok(Ok((Asked::NotACard, _))) => None,
        Ok(Ok((Asked::One(card), cards))) => Some(fill_card(state, caller, req, block_id, agent, reason, Some(*card), cards).await),
        // The cards the person might pick from can be none: all deleted, or none that opens, since `card_asked` looked.
        Ok(Ok((Asked::Pick, cards))) if cards.is_empty() => Some(err_response(StatusCode::BAD_REQUEST, "you have no card saved that can be filled: ask the person to add one in the Vault, or to type it (Vault request_value)".to_string())),
        Ok(Ok((Asked::Pick, cards))) => Some(fill_card(state, caller, req, block_id, agent, reason, None, cards).await),
        Ok(Err(e)) => Some(err_response(StatusCode::BAD_REQUEST, e)),
        Err(e) => Some(err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// What the person is asked: the card named (or none), and the cards they may pick from when they may.
struct Question<'a> {
    card: Option<&'a CardTarget>,
    picker: &'a [CardTarget],
    origin: &'a str,
    /// The page's origin when the fields are in another site's frame, else empty.
    page: &'a str,
    names: &'a [&'a str],
    offer_always: bool,
    why: &'a str,
}

fn card_question(agent: &str, pane: String, q: &Question, agent_reason: &str) -> UseRequest {
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: pane,
        entry_id: q.card.map(|c| c.fill.id.clone()).unwrap_or_default(),
        entry_label: q.card.map(|c| c.fill.label.clone()).unwrap_or_else(|| "a card".to_string()),
        what: q.card.map(CardTarget::describe).unwrap_or_else(|| "card".to_string()),
        origin: q.origin.to_string(),
        page: q.page.to_string(),
        field: format!("the card fields: {}", q.names.join(", ")),
        offer_always: q.offer_always,
        reason: q.why.to_string(),
        agent_reason: agent_reason.to_string(),
        kind: "card".to_string(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        choices: (!q.picker.is_empty()).then(|| q.picker.iter().map(CardTarget::choice).collect()),
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

fn answer(entry: &str, origin: &str, page: &str, done: &Outcome, went_ahead: bool) -> Value {
    let mut out = json!({
        // The entry's id and part names only: the brand, last four and expiry are the person's to see.
        "filled_from_vault": entry,
        "parts_filled": done.filled,
        "value": "[secret: hidden]",
        "site": origin,
    });
    if let Some(o) = out.as_object_mut() {
        if !page.is_empty() {
            o.insert("in_a_frame_on".into(), json!(page));
        }
        if !done.left.is_empty() {
            o.insert("parts_left".into(), json!(done.left));
        }
        if !done.failed.is_empty() {
            o.insert("parts_failed".into(), json!(done.failed));
        }
        if went_ahead {
            o.insert("approval".into(), json!(uses::WENT_AHEAD));
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

/// Whether the person may pick the card: none was named, or they have more than one.
fn picks(card: &Option<CardTarget>, picker: &[CardTarget]) -> bool {
    card.is_none() || picker.len() > 1
}

/// The card the fill is for once the person has answered, and how they answered.
struct Settled {
    card: CardTarget,
    always: bool,
    went_ahead: bool,
}

/// What a use record names when no card was chosen (a refusal, a no, no answer): never a card of the person's.
fn stand_in(card: &Option<CardTarget>, picker: &[CardTarget]) -> crate::backend::vault::FillTarget {
    match (card, picker.first()) {
        (Some(c), _) => c.fill.clone(),
        (None, Some(first)) => {
            let mut fill = first.fill.clone();
            fill.id = String::new();
            fill.label = "a card".to_string();
            fill
        }
        (None, None) => unreachable!("a whole-card fill has a card or a list of cards"),
    }
}

/// Asks the person, unless this card is granted on this site and current: the card they chose (the named one when
/// they chose none), or the response that ends the call.
async fn settle(state: &AppState, caller: Option<&Caller>, req: &VaultFillRequest, call: (&str, &str, &str), place: (&str, &str, &[&str]), (card, picker): (Option<CardTarget>, Vec<CardTarget>)) -> Result<Settled, Response> {
    let ((block_id, agent, reason), (origin, page, names)) = (call, place);
    // A payment frame is asked about, and granted, with the page it is on.
    let grant_key = grant_site(origin, page);
    let site = format!("{grant_key} (card form)");
    let (store, grant_origin) = (state.identity_store.clone(), grant_key);
    let granted_id = card.as_ref().filter(|c| !c.expired).map(|c| c.fill.id.clone());
    let granted = match granted_id {
        Some(entry) => tokio::task::spawn_blocking(move || store.vault_grant_check(&entry, &grant_origin).unwrap_or(false)).await.unwrap_or(false),
        None => false,
    };
    let pick = picks(&card, &picker);
    let planned = plan(!origin.is_empty(), false, true, granted, true);
    if let Plan::Refuse(why) = planned {
        record(state, block_id, &stand_in(&card, &picker), agent, &site, "refused", reason);
        return Err(err_response(StatusCode::BAD_REQUEST, why.to_string()));
    }
    if matches!(planned, Plan::Go) && card.as_ref().is_some_and(|c| !c.expired) {
        return Ok(Settled { card: card.expect("a card was named"), always: false, went_ahead: false });
    }
    let expired = card.as_ref().is_some_and(|c| c.expired);
    let (offer_always, why) = match planned {
        // The person may pick a current card, so "Always" is offered and enforced per card (vault_handlers.rs).
        _ if pick => (true, if card.is_none() { "no card was named, and there is no default card" } else { "you may want a different card" }),
        Plan::Ask { offer_always, reason } if !expired => (offer_always, reason),
        _ => (false, "the card has expired"),
    };
    let asked = Question { card: card.as_ref(), picker: if pick { &picker } else { &[] }, origin, page, names, offer_always, why };
    let question = card_question(agent, super::super::vault_ask::agent_pane(state, caller, &req.auth), &asked, reason);
    let what = card.as_ref().map(CardTarget::describe).unwrap_or_else(|| "card".to_string());
    let headline = if page.is_empty() { format!("{agent} wants to fill your {what} on {origin}") } else { format!("{agent} wants to fill your {what} in a card field from {origin} on {page}") };
    let (ended, answer) = super::super::vault_ask::ask(state, block_id, question, ANSWER_WAIT, &headline).await;
    if !ended.allowed() {
        record(state, block_id, &stand_in(&card, &picker), agent, &site, ended.outcome(), reason);
        let code = if ended == Ended::NoAnswer { StatusCode::REQUEST_TIMEOUT } else { StatusCode::FORBIDDEN };
        return Err(err_response(code, ended.refusal("nothing was typed")));
    }
    // The card they picked is one of those opened for this question, so a card changed since is caught when it is opened.
    let picked = answer.and_then(|a| a.choice);
    let stand = stand_in(&card, &picker);
    let chosen = match (picked, card) {
        (Some(id), Some(c)) if c.fill.id == id => Some(c),
        (Some(id), _) => picker.into_iter().find(|c| c.fill.id == id),
        (None, c) => c,
    };
    let Some(card) = chosen else {
        record(state, block_id, &stand, agent, &site, "failed", reason);
        return Err(err_response(StatusCode::BAD_REQUEST, "no card was chosen: nothing was typed".to_string()));
    };
    Ok(Settled { always: ended == Ended::Always && !card.expired, went_ahead: ended == Ended::WentAhead, card })
}

/// What the host found of a card form: where it is, and the fields it matched to a card's parts.
struct CardForm {
    origin: String,
    /// The page's origin when the fields are in another site's frame, else empty: the question and the grant carry both.
    page: String,
    grant_key: String,
    fields: Vec<CardField>,
    names: Vec<&'static str>,
}

/// Asks the host for the card form the field `ref_` is in, or says why there is none.
async fn read_form(state: &AppState, block_id: &str, ref_: &str, instead: &str) -> Result<CardForm, Response> {
    let form = match proxy_value(state, "card_form", json!({ "block_id": block_id, "ref": ref_ }), HOST_WAIT).await {
        Ok(v) => v,
        Err((code, e)) => return Err(err_response(code, e)),
    };
    let origin = form.get("origin").and_then(Value::as_str).unwrap_or("").to_string();
    let page = framed_in(&origin, form.get("page_origin").and_then(Value::as_str).unwrap_or(""));
    let grant_key = grant_site(&origin, &page);
    let fields = parse_fields(&form);
    if !fields.iter().any(|f| MAIN_SLOTS.contains(&f.shape.slot.as_str())) {
        return Err(err_response(StatusCode::BAD_REQUEST, format!("no card number, expiry or security code field was found with that one: {instead}, or take a new snapshot")));
    }
    let names = part_names(&fields);
    Ok(CardForm { origin, page, grant_key, fields, names })
}

/// Why nothing was filled, by part name: never a value.
fn why_nothing(done: &Outcome) -> String {
    done.failed.iter().chain(&done.left).map(|v| format!("{}: {}", v["part"].as_str().unwrap_or("?"), v["why"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join("; ")
}

async fn fill_card(state: &AppState, caller: Option<&Caller>, req: &VaultFillRequest, block_id: &str, agent: &str, reason: &str, card: Option<CardTarget>, picker: Vec<CardTarget>) -> Response {
    tracing::info!(agent = %agent, block_id = %block_id, entry = %card.as_ref().map(|c| c.fill.id.as_str()).unwrap_or("(the person picks)"), "[vault] a whole-card web form fill was asked for");
    let named = card.as_ref().map(|c| c.fill.id.as_str()).unwrap_or("<id>");
    let instead = format!("fill a single field with {named}.<part> (number, exp_month, exp_year, cvc, name, zip)");
    let CardForm { origin, page, grant_key, fields, names } = match read_form(state, block_id, &req.ref_, &instead).await {
        Ok(f) => f,
        Err(response) => return response,
    };
    let site = format!("{grant_key} (card form)");
    let Settled { mut card, always, went_ahead } = match settle(state, caller, req, (block_id, agent, reason), (&origin, &page, &names), (card, picker)).await {
        Ok(s) => s,
        Err(response) => return response,
    };
    if always {
        let (store, entry, grant_origin, created) = (state.identity_store.clone(), card.fill.id.clone(), grant_key.clone(), card.fill.created_at);
        let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&entry, &grant_origin, agentmux_common::time::now_ms(), created)).await;
    }

    // The card leaves the sealed row only now.
    let store = state.identity_store.clone();
    let (entry, created, sealed) = (card.fill.id.clone(), card.fill.created_at, card.sealed.clone());
    let parts = match tokio::task::spawn_blocking(move || card_target::parts_for_fill(&store, &entry, created, &sealed)).await {
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
        let why = why_nothing(&done);
        return err_response(StatusCode::BAD_REQUEST, format!("nothing was filled: {why}"));
    }
    record(state, block_id, &card.fill, agent, &site, "done", reason);
    let (store, entry) = (state.identity_store.clone(), card.fill.id.clone());
    let _ = tokio::task::spawn_blocking(move || store.vault_touch(&entry, agentmux_common::time::now_ms())).await;
    (StatusCode::OK, Json(json!({ "ok": true, "data": answer(&card.fill.id, &origin, &page, &done, went_ahead) }))).into_response()
}

mod typed;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::vault::{kinds, FillTarget};

    fn a_card() -> CardTarget {
        let kind = kinds::kind("card").expect("the card kind");
        let fill = FillTarget { id: "card-a".into(), kind: kind.name, typed: false, kind_info: kind, label: "A".into(), part: "number", slot: None, what: "card".into(), created_at: 1, version: None };
        CardTarget { fill, brand: "visa", last_four: "4242".into(), expiry: Some((3, 2027)), expired: false, default: false, sealed: vec![] }
    }

    #[test]
    fn a_frame_and_the_page_it_is_on_are_one_site_for_the_grant_and_the_question() {
        assert_eq!(grant_site("https://shop.example", ""), "https://shop.example", "a field of the page: its origin");
        assert_eq!(grant_site("https://shop.example", "https://shop.example"), "https://shop.example", "a frame of the page's own site is the page's");
        assert_eq!(grant_site("https://js.stripe.com", "https://shop.example"), "https://js.stripe.com on https://shop.example");
        assert_ne!(grant_site("https://js.stripe.com", "https://shop.example"), grant_site("https://js.stripe.com", "https://other.example"), "the provider's frame on another page is another grant");
        assert_eq!(framed_in("https://js.stripe.com", "https://shop.example"), "https://shop.example");
        assert_eq!(framed_in("https://shop.example", "https://shop.example"), "");
        assert_eq!(framed_in("", "https://shop.example"), "");
        let framed = card_question("a", String::new(), &Question { card: Some(&a_card()), picker: &[], origin: "https://js.stripe.com", page: "https://shop.example", names: &["number"], offer_always: true, why: "x" }, "");
        assert_eq!((framed.origin.as_str(), framed.page.as_str()), ("https://js.stripe.com", "https://shop.example"));
    }

    #[test]
    fn a_card_never_goes_ahead_on_silence_so_the_answer_carries_no_approval_note_unless_one_did() {
        let done = Outcome { filled: vec!["number"], ..Default::default() };
        assert!(answer("card-a", "https://shop.example", "", &done, false).get("approval").is_none());
        assert_eq!(answer("card-a", "https://shop.example", "", &done, true)["approval"], uses::WENT_AHEAD);
        assert!(!uses::goes_ahead(&card_question("a", String::new(), &Question { card: Some(&a_card()), picker: &[], origin: "https://shop.example", page: "", names: &["number"], offer_always: false, why: "x" }, "")), "silence denies a card");
    }
}
