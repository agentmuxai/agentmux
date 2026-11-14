// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The card fields of a form
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! `/agentmux/browser/card_form`: given the reference of one field, every field in
//! the same frame that looks like a part of a card (`vault_slots`), in the page's
//! order, with what it takes (a choice, a length, a placeholder, a pattern). A field in
//! another site's frame (a payment provider's hosted field) is looked for in the page's
//! other frames of that same site too, since such providers put each field in a frame of
//! its own; the answer says so (`framed`) and names the page's own origin. srv
//! asks the person once for the lot and fills them one by one through `vault_fill`;
//! each of those checks the field again. Password and one-time-code fields are never
//! listed. Nothing here reads a value.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;

use super::act::{call_on, describe, describe_markup, fail, isolated_context, resolve, unauthorized};
use super::frames::{self, FrameRef};
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
    /// The origin the fields are in: their frame's (the same for every field listed).
    pub origin: String,
    pub page: String,
    /// The page's own origin; differs from `origin` when the fields are in another site's frame.
    pub page_origin: String,
    pub framed: bool,
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

/// Whether a field's frame is one to look in: the referenced field's own, or, when that is in another site's frame,
/// any other frame of that same site.
fn looked_in(own: Option<&FrameRef>, of: Option<&FrameRef>, framed: bool) -> bool {
    of == own || (framed && matches!((own, of), (Some(o), Some(f)) if f.origin == o.origin))
}

type Group = (Option<FrameRef>, Vec<(String, i64)>);

/// The references to look at, by frame: the referenced field's own frame first, then the others by their id.
fn groups_of(state: &Arc<AppState>, block_id: &str, own: Option<&FrameRef>, framed: bool) -> Result<Vec<Group>, String> {
    let table = state.browser_api.ref_tables.get(block_id).ok_or("no snapshot of this pane yet: take one first")?;
    let mut groups: Vec<Group> = Vec::new();
    for (r, backend) in table.refs.iter() {
        let of = table.frames.get(r);
        if !looked_in(own, of, framed) {
            continue;
        }
        match groups.iter_mut().find(|(f, _)| f.as_ref().map(|f| &f.frame_id) == of.map(|f| &f.frame_id)) {
            Some((_, refs)) => refs.push((r.clone(), *backend)),
            None => groups.push((of.cloned(), vec![(r.clone(), *backend)])),
        }
    }
    for (_, refs) in &mut groups {
        refs.sort_by_key(|(r, _)| order_of(r));
        refs.truncate(MAX_LOOKED_AT);
    }
    let id_of = |f: &Option<FrameRef>| f.as_ref().map(|f| f.frame_id.clone());
    groups.sort_by_key(|(f, _)| (id_of(f) != own.map(|o| o.frame_id.clone()), id_of(f)));
    Ok(groups)
}

async fn card_form(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &FieldReq) -> Result<CardForm, String> {
    let (located, own, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let page_origin = agentmux_common::allowed_origins::target_origin(&located.page).unwrap_or_default();
    let framed = frame.is_some() && !page_origin.is_empty() && located.origin != page_origin;
    let groups = groups_of(state, &req.block_id, frame.as_ref(), framed)?;
    let allowed = state.site_limits.list(&req.block_id);

    let mut fields = Vec::new();
    for (g_frame, refs) in groups {
        let is_own = g_frame.as_ref().map(|f| &f.frame_id) == frame.as_ref().map(|f| &f.frame_id);
        if !is_own {
            // Another frame of the payment site: the same checks `locate` made for the referenced one. A frame that
            // fails them (it moved on, or the pane's site limits keep the agent out of it) is left out.
            let (Some(f), Some((sample, _))) = (&g_frame, refs.first()) else { continue };
            if frames::check_frame(root, f, sample).await.is_err() || frames::refuse_if_blocked(root, allowed.as_deref(), f, sample).await.is_err() {
                continue;
            }
        }
        let mut in_frame = root.in_session(g_frame.as_ref().and_then(|f| f.session.as_deref()));
        let Ok(ctx) = isolated_context(&mut in_frame, g_frame.as_ref().map(|f| f.frame_id.as_str())).await else { continue };
        // A page can hold more than one payment form: only the fields in the referenced field's own form
        // count (a field in no form, or a reference in none, leaves the whole frame to choose from). Another
        // frame's forms can't be compared with it: there every field counts.
        let own_form = if is_own { form_of(&mut in_frame, ctx, own, &req.ref_).await } else { -1 };
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
    }
    Ok(CardForm { origin: located.origin, page: located.page, page_origin, framed, fields })
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

    fn frame(id: &str, origin: &str) -> FrameRef {
        FrameRef { frame_id: id.into(), session: None, origin: origin.into(), url: format!("{origin}/f"), chain: vec![] }
    }

    #[test]
    fn a_payment_frames_siblings_of_its_site_are_looked_in_and_no_other_frame_is() {
        let (number, expiry, other) = (frame("a", "https://js.stripe.com"), frame("b", "https://js.stripe.com"), frame("c", "https://evil.example"));
        // A field of the page itself: its own frame only, as before.
        assert!(looked_in(None, None, false));
        assert!(!looked_in(None, Some(&number), false), "a page's own field never reaches into a frame");
        // A field in a frame of another site: its own, and the page's other frames of that site.
        assert!(looked_in(Some(&number), Some(&number), true));
        assert!(looked_in(Some(&number), Some(&expiry), true), "Stripe puts each field in a frame of its own");
        assert!(!looked_in(Some(&number), Some(&other), true), "another site's frame is not the payment form");
        assert!(!looked_in(Some(&number), None, true), "nor is the page's own field");
        // A frame of the page's own site is not framed: its own frame only.
        assert!(!looked_in(Some(&number), Some(&expiry), false));
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
