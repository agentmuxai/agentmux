// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The card fields of a form
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! `/agentmux/browser/card_form`: given the reference of one field, every field in
//! the same frame that looks like a part of a card (`vault_slots`), in the page's
//! order, with what it takes (a choice, a length, a placeholder, a pattern). srv
//! asks the person once for the lot and fills them one by one through `vault_fill`;
//! each of those checks the field again. Password and one-time-code fields are never
//! listed. Nothing here reads a value.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;

use super::act::{call_on, describe, describe_markup, fail, isolated_context, resolve, unauthorized};
use super::snapshot;
use super::types::ApiResponse;
use super::vault_fill::{display_label, fingerprint, is_select, locate, takes_text, FieldReq};
use super::vault_slots;
use crate::state::AppState;

/// The most fields of a page looked at: a form has a handful, a page a few hundred.
const MAX_LOOKED_AT: usize = 400;

/// The slots a whole-card fill uses.
const CARD_SLOTS: [&str; 7] = [
    vault_slots::CARD_NUMBER,
    vault_slots::CARD_EXPIRY,
    vault_slots::CARD_EXP_MONTH,
    vault_slots::CARD_EXP_YEAR,
    vault_slots::CARD_CVC,
    vault_slots::CARD_NAME,
    vault_slots::CARD_ZIP,
];

#[derive(Debug, Serialize)]
pub struct CardField {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub label: String,
    pub fingerprint: String,
    pub slot: &'static str,
    pub select: bool,
    /// The `type` of an `<input>` (`text`, `month`, `number`), lower-cased; empty for a `<select>`.
    pub input_type: String,
    pub max_length: Option<usize>,
    pub placeholder: String,
    pub pattern: String,
}

#[derive(Debug, Serialize)]
pub struct CardForm {
    pub origin: String,
    pub page: String,
    pub fields: Vec<CardField>,
}

/// The place of a field's `<form>` among its document's forms, or -1 when it is in none.
const FORM_OF: &str = r#"function () {
  const form = this.form || (this.closest && this.closest("form"));
  return form ? Array.prototype.indexOf.call(this.ownerDocument.forms, form) : -1;
}"#;

async fn form_of(cdp: &mut super::cdp::CdpSession, ctx: i64, backend: i64, r: &str) -> i64 {
    let Ok(obj) = resolve(cdp, backend, ctx, r).await else { return -1 };
    call_on(cdp, &obj, FORM_OF, vec![]).await.ok().and_then(|v| v.as_i64()).unwrap_or(-1)
}

/// Whether a field is in the referenced field's form: always, when that one is in no form.
fn same_form(own: i64, other: i64) -> bool {
    own < 0 || own == other
}

/// `e12` before `e102`, and `f1e3` before `f1e10`: references in the order the snapshot numbered
/// them, by the number after the last `e` (the part before it names the frame, the same for all of them).
fn order_of(r: &str) -> (usize, String) {
    let number = r.rsplit_once('e').and_then(|(_, n)| n.parse().ok()).unwrap_or(usize::MAX);
    (number, r.to_string())
}

/// `POST /agentmux/browser/card_form`.
pub async fn card_form_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<FieldReq>) -> (StatusCode, Json<ApiResponse<CardForm>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-card-form").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let found = card_form(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match found {
        Ok(form) => (StatusCode::OK, Json(ApiResponse::ok(form))),
        Err(e) => fail(e),
    }
}

async fn card_form(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &FieldReq) -> Result<CardForm, String> {
    let (located, own, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let table = state.browser_api.ref_tables.get(&req.block_id).ok_or("no snapshot of this pane yet: take one first")?;
    let mut refs: Vec<(String, i64)> = table.refs.iter().filter(|(r, _)| table.frames.get(*r) == frame.as_ref()).map(|(r, b)| (r.clone(), *b)).collect();
    refs.sort_by_key(|(r, _)| order_of(r));
    refs.truncate(MAX_LOOKED_AT);

    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    // A page can hold more than one payment form: only the fields in the referenced field's own form
    // count (a field in no form, or a reference in none, leaves the whole frame to choose from).
    let ctx = isolated_context(&mut in_frame, frame.as_ref().map(|f| f.frame_id.as_str())).await?;
    let own_form = form_of(&mut in_frame, ctx, own, &req.ref_).await;
    let mut fields = Vec::new();
    for (r, backend) in refs {
        // The markup first: most of a page's references are not form fields at all.
        let Some(markup) = describe_markup(&mut in_frame, backend).await else { continue };
        if !matches!(markup.local_name.as_str(), "input" | "select" | "textarea") || !(takes_text(&markup) || is_select(&markup)) {
            continue;
        }
        let Some(facts) = describe(&mut in_frame, backend).await else { continue };
        if vault_slots::password_like(&facts) {
            continue;
        }
        let Some(slot) = vault_slots::slots(&facts).into_iter().find(|s| CARD_SLOTS.contains(s)) else { continue };
        if !same_form(own_form, form_of(&mut in_frame, ctx, backend, &r).await) {
            continue;
        }
        fields.push(CardField {
            ref_: r,
            label: display_label(&facts),
            fingerprint: fingerprint(&facts),
            slot,
            select: is_select(&facts),
            input_type: if is_select(&facts) { String::new() } else { facts.attr("type").trim().to_ascii_lowercase() },
            max_length: snapshot::max_length(&facts),
            placeholder: facts.attr("placeholder").trim().chars().take(40).collect(),
            pattern: facts.attr("pattern").trim().chars().take(120).collect(),
        });
    }
    Ok(CardForm { origin: located.origin, page: located.page, fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fields_of_the_referenced_form_count() {
        assert!(same_form(0, 0));
        assert!(!same_form(0, 1), "a second payment form is not the first");
        assert!(same_form(-1, 3), "a reference in no form leaves the whole frame to choose from");
        assert!(FORM_OF.contains("this.form") && FORM_OF.contains("ownerDocument.forms"));
    }

    #[test]
    fn references_are_in_the_order_the_snapshot_numbered_them() {
        let mut refs = vec!["e102", "e12", "e2"];
        refs.sort_by_key(|r| order_of(r));
        assert_eq!(refs, ["e2", "e12", "e102"]);
        // Inside a frame the first number is the frame's, the same for every field of it.
        let mut framed = vec!["f1e10", "f1e3", "f1e2"];
        framed.sort_by_key(|r| order_of(r));
        assert_eq!(framed, ["f1e2", "f1e3", "f1e10"]);
    }
}
