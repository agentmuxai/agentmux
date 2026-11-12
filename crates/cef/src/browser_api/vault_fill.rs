// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filling a form field from the Vault
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6).
//!
//! Two routes, both for srv, which holds the Vault and decides:
//!
//! - `/agentmux/browser/field_info`: where a reference is (its frame's origin) and
//!   which kinds of Vault value it looks like, so srv can ask the person.
//! - `/agentmux/browser/vault_fill`: type the value srv sends into that field, if
//!   it is still where and what the person approved, and never into a password
//!   or one-time-code field. The field is then marked (`data-agentmux-vault`), so
//!   every snapshot, find, read-back and screenshot treats it as secret. The
//!   answer says the field was filled, never what it holds.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{call_on, describe, fail, isolated_context, page_url, resolve, unauthorized, PREPARE_FILL};
use super::frames;
use super::snapshot::{self, DomFacts};
use super::types::ApiResponse;
use super::vault_slots;
use crate::state::AppState;

/// The attribute that marks a field holding a Vault value. `secret.rs` reads it.
pub const TAINT_ATTR: &str = "data-agentmux-vault";

#[derive(Debug, Deserialize)]
pub struct FieldReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
}

#[derive(Debug, Deserialize)]
pub struct FillReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    /// The value: held only for the length of this request.
    pub text: String,
    /// The origin and field the person approved. The fill is made only if the
    /// field still has them: the page could have changed it while they read.
    pub expect_origin: String,
    pub expect_fingerprint: String,
    /// `"month"` or `"year"`: the field is a `<select>`, and `text` names the option to choose
    /// (`03`, `2027`), as a number or a name.
    #[serde(default)]
    pub choose: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FieldInfo {
    /// The origin of the frame the field is in (the page's own origin for the top frame).
    pub origin: String,
    /// The page's URL.
    pub page: String,
    /// What the person is shown for the field: its label or name.
    pub label: String,
    /// Changes when the field's markup does.
    pub fingerprint: String,
    /// Which kinds of Vault value it looks like (`vault_slots`).
    pub slots: Vec<&'static str>,
    /// A password or one-time-code field: never filled from the Vault.
    pub password_like: bool,
    /// Whether it takes typed text, or is a `<select>` a card's month or year can be chosen in.
    pub fillable: bool,
    /// A `<select>`: it is chosen in, not typed in.
    pub select: bool,
}

#[derive(Debug, Serialize)]
pub struct FillData {
    pub filled: bool,
    /// The page's own validation message, if it rejected the value.
    pub validation: String,
    /// The page shortened or reformatted what was typed.
    pub changed_by_page: bool,
}

/// What a person reads as the field's name, for the approval prompt.
pub(super) fn display_label(f: &DomFacts) -> String {
    [f.label.trim(), f.attr("aria-label").trim(), f.attr("placeholder").trim(), f.attr("name").trim(), f.attr("id").trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("(unnamed field)")
        .chars()
        .take(120)
        .collect()
}

/// Identifies a field by its markup, so a changed one isn't mistaken for it.
pub(super) fn fingerprint(f: &DomFacts) -> String {
    [f.local_name.as_str(), f.attr("type"), f.attr("name"), f.attr("id"), f.attr("autocomplete"), f.attr("aria-label"), f.attr("placeholder"), f.label.trim()]
        .join("\u{1f}")
}

pub(super) fn takes_text(f: &DomFacts) -> bool {
    match f.local_name.as_str() {
        "textarea" => true,
        "input" => !matches!(
            f.attr("type").to_ascii_lowercase().as_str(),
            "checkbox" | "radio" | "file" | "submit" | "button" | "image" | "reset" | "range" | "color" | "hidden"
        ),
        _ => false,
    }
}

pub(super) fn is_select(f: &DomFacts) -> bool {
    f.local_name == "select"
}

/// Where a reference is and what it is, from the pane's latest snapshot.
pub(super) struct Located {
    pub facts: DomFacts,
    pub origin: String,
    pub page: String,
}

pub(super) async fn locate(
    state: &Arc<AppState>,
    block_id: &str,
    ref_: &str,
    cdp: &mut super::cdp::CdpSession,
) -> Result<(Located, i64, Option<frames::FrameRef>), String> {
    let table = state
        .browser_api
        .ref_tables
        .get(block_id)
        .ok_or("no snapshot of this pane yet: take one with Browser's snapshot action first, or find the element")?;
    let &backend = table.refs.get(ref_).ok_or_else(|| format!("unknown reference {ref_:?}; take a new snapshot"))?;
    let frame = table.frames.get(ref_).cloned();
    let now = page_url(cdp).await;
    if !table.url.is_empty() && now != table.url {
        return Err(format!("the page changed since the snapshot (now {now}); take a new snapshot"));
    }
    if let Some(f) = &frame {
        frames::check_frame(cdp, f, ref_).await?;
        frames::refuse_if_blocked(cdp, state.site_limits.list(block_id).as_deref(), f, ref_).await?;
    }
    let mut in_frame = cdp.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let facts = describe(&mut in_frame, backend).await.ok_or_else(|| format!("the element for {ref_} is gone from the page; take a new snapshot"))?;
    let origin = match &frame {
        Some(f) => f.origin.clone(),
        None => agentmux_common::allowed_origins::target_origin(&now).unwrap_or_default(),
    };
    Ok((Located { facts, origin, page: now }, backend, frame))
}

/// `POST /agentmux/browser/field_info`.
pub async fn field_info_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FieldReq>,
) -> (StatusCode, Json<ApiResponse<FieldInfo>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-field").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let located = locate(&state, &req.block_id, &req.ref_, &mut cdp).await;
    let _ = cdp.close().await;
    match located {
        Ok((l, _, _)) => (
            StatusCode::OK,
            Json(ApiResponse::ok(FieldInfo {
                origin: l.origin,
                page: l.page,
                label: display_label(&l.facts),
                fingerprint: fingerprint(&l.facts),
                slots: vault_slots::slots(&l.facts),
                password_like: vault_slots::password_like(&l.facts),
                fillable: takes_text(&l.facts) || is_select(&l.facts),
                select: is_select(&l.facts),
            })),
        ),
        Err(e) => fail(e),
    }
}

/// The attributes of the approval's fingerprint, rechecked live by [`CHECK_AND_SET`].
const FINGERPRINT_ATTRS: &[&str] = &["type", "name", "id", "autocomplete", "aria-label", "placeholder"];

/// Checks `this` is still the approved field, marks it as holding a Vault value
/// and sets its value, in one synchronous call on the field itself: still in a
/// document of the approved origin, the same element and attributes the
/// approval's fingerprint saw after focusing, and editable. The page gets no
/// turn between the checks and the value, and nothing is typed wherever the
/// focus happens to be: a page could move it, even into another site's frame,
/// between a focus check and `Input.insertText`. The value goes in through the
/// native setter of the element's own prototype with input and change events,
/// as `act::SET_VALUE` does, the input event composed so it leaves a shadow root as
/// a typed one would (a date input's value is already in its format).
/// It answers whether the field still holds the value after the page's handlers
/// ran, and its validation message, never the value.
const CHECK_AND_SET: &str = r#"function (origin, v, attr, tag, attrs) {
  // The document's own origin: an about:blank or srcdoc frame's URL has none,
  // and it runs with its parent's, as `frames::effective_origin` says.
  const w = this.ownerDocument && this.ownerDocument.defaultView;
  if (!this.isConnected || !w || w.origin !== origin) return { error: "origin" };
  if (this.localName !== tag) return { error: "changed" };
  for (const [k, want] of Object.entries(attrs)) {
    if ((this.getAttribute(k) || "") !== want) return { error: "changed" };
  }
  if (this.disabled || this.readOnly) return { error: "changed" };
  // A field with a value setter, or a rich-text field, whose text is its value.
  const d = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(this), "value");
  const rich = !(d && d.set) && this.isContentEditable;
  if (!(d && d.set) && !rich) return { error: "changed" };
  this.setAttribute(attr, "1");
  if (rich) { this.textContent = v; } else { d.set.call(this, v); }
  this.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { same: (rich ? this.textContent : this.value) === v, validation: this.validationMessage || "" };
}"#;

/// `POST /agentmux/browser/vault_fill`.
pub async fn vault_fill_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FillReq>,
) -> (StatusCode, Json<ApiResponse<FillData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-fill").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = fill(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

async fn fill(state: &Arc<AppState>, root: &mut super::cdp::CdpSession, req: &FillReq) -> Result<FillData, String> {
    let (located, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    check_still_approved(&located, req)?;
    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let cdp = &mut in_frame;
    let frame_id = frame.as_ref().map(|f| f.frame_id.as_str());
    let ctx = isolated_context(cdp, frame_id).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    if let Some(kind) = req.choose.as_deref() {
        return choose(cdp, &obj, &located, req, kind).await;
    }

    let length = req.text.encode_utf16().count();
    if let Some(max) = snapshot::max_length(&located.facts).filter(|max| length > *max) {
        return Err(format!("this field holds at most {max} characters and the value is longer: nothing was typed"));
    }
    let date = date_type(&located.facts);
    // The date parser's own errors quote what it couldn't read, which here is the
    // Vault value: say it without it.
    let typed = match &date {
        Some(ty) => super::dates::to_input_value(ty, &req.text)
            .map_err(|_| format!("the value isn't a date this {ty} field can take (a date of birth is best kept as YYYY-MM-DD): nothing was typed"))?,
        None => req.text.clone(),
    };

    let prepared = call_on(cdp, &obj, PREPARE_FILL, vec![]).await?;
    if let Some(e) = prepared.get("error").and_then(Value::as_str) {
        return Err(e.to_string());
    }
    // Focusing ran the page's own handlers: the field must still be what was
    // approved. That is checked here, and again, with the value set in the same
    // call, on the field itself.
    let after_focus = describe(cdp, backend).await.ok_or("the field is gone from the page")?;
    let attrs: serde_json::Map<String, Value> = FINGERPRINT_ATTRS.iter().map(|k| (k.to_string(), json!(after_focus.attr(k)))).collect();
    let tag = after_focus.local_name.clone();
    check_still_approved(&Located { facts: after_focus, origin: located.origin.clone(), page: located.page.clone() }, req)?;
    let seen = call_on(cdp, &obj, CHECK_AND_SET, vec![json!(req.expect_origin), json!(typed), json!(TAINT_ATTR), json!(tag), Value::Object(attrs)]).await?;
    match seen.get("error").and_then(Value::as_str) {
        None => {}
        Some("origin") => return Err(format!("the field is no longer in {}, so nothing was typed", req.expect_origin)),
        Some(_) => return Err("the field changed when it took the focus, so nothing was typed; ask again to show the person what it is now".to_string()),
    }
    Ok(FillData {
        filled: true,
        validation: seen.get("validation").and_then(Value::as_str).unwrap_or("").chars().take(300).collect(),
        changed_by_page: seen.get("same").and_then(Value::as_bool) != Some(true),
    })
}

/// Checks `this` is still the approved `<select>` (as [`CHECK_AND_SET`] does a text field) and
/// chooses the option the text names: a month by its number or name (`3`, `03`, `March`, `Mar`),
/// a year by its four digits or its last two. Nothing is chosen when no option matches, and the
/// answer says whether the select holds the choice after the page's handlers ran, never which.
const CHOOSE: &str = r#"function (origin, v, kind, attr, tag, attrs) {
  const w = this.ownerDocument && this.ownerDocument.defaultView;
  if (!this.isConnected || !w || w.origin !== origin) return { error: "origin" };
  if (this.localName !== tag || tag !== "select") return { error: "changed" };
  if (this.disabled) return { error: "changed" };
  this.scrollIntoView({ block: "center", inline: "center" });
  this.focus();
  for (const [k, want] of Object.entries(attrs)) {
    if ((this.getAttribute(k) || "") !== want) return { error: "changed" };
  }
  const want = Number(v);
  const names = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
  const fits = (text) => {
    const words = String(text).toLowerCase().match(/[a-z]+|\d+/g) || [];
    return words.some((t) => {
      if (/^\d+$/.test(t)) return kind === "month" ? Number(t) === want : Number(t) === want || (t.length === 2 && Number(t) === want % 100);
      return kind === "month" && t.length >= 3 && names[want - 1] && names[want - 1].startsWith(t);
    });
  };
  const options = Array.from(this.options);
  let at = options.findIndex((o) => !o.disabled && fits(o.value));
  if (at < 0) at = options.findIndex((o) => !o.disabled && fits(o.label || o.textContent));
  if (at < 0) return { error: "nooption" };
  this.setAttribute(attr, "1");
  this.selectedIndex = at;
  this.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { same: this.selectedIndex === at, validation: this.validationMessage || "" };
}"#;

/// A month or year chosen in a `<select>`.
async fn choose(cdp: &mut super::cdp::CdpSession, obj: &str, located: &Located, req: &FillReq, kind: &str) -> Result<FillData, String> {
    if !matches!(kind, "month" | "year") || !is_select(&located.facts) {
        return Err("only a month or a year is chosen in a list, and only in a list".into());
    }
    let attrs: serde_json::Map<String, Value> = FINGERPRINT_ATTRS.iter().map(|k| (k.to_string(), json!(located.facts.attr(k)))).collect();
    let tag = located.facts.local_name.clone();
    let seen = call_on(cdp, obj, CHOOSE, vec![json!(req.expect_origin), json!(req.text), json!(kind), json!(TAINT_ATTR), json!(tag), Value::Object(attrs)]).await?;
    match seen.get("error").and_then(Value::as_str) {
        None => {}
        Some("origin") => return Err(format!("the field is no longer in {}, so nothing was chosen", req.expect_origin)),
        Some("nooption") => return Err(format!("the list has no option for the card's expiry {kind}: nothing was chosen")),
        Some(_) => return Err("the field changed when it took the focus, so nothing was chosen; ask again to show the person what it is now".to_string()),
    }
    Ok(FillData {
        filled: true,
        validation: seen.get("validation").and_then(Value::as_str).unwrap_or("").chars().take(300).collect(),
        changed_by_page: seen.get("same").and_then(Value::as_bool) != Some(true),
    })
}

fn date_type(f: &DomFacts) -> Option<String> {
    let ty = f.attr("type").trim().to_ascii_lowercase();
    (f.local_name == "input" && super::dates::is_date_input(&ty)).then_some(ty)
}

/// The field is where and what the person approved, and one the Vault may fill.
fn check_still_approved(l: &Located, req: &FillReq) -> Result<(), String> {
    if l.origin != req.expect_origin {
        return Err(format!("the field is now in {} and not the site the person approved, so nothing was typed", l.origin));
    }
    if fingerprint(&l.facts) != req.expect_fingerprint {
        return Err("the field changed while the person was reading the approval, so nothing was typed; ask again to show them what it is now".into());
    }
    if vault_slots::password_like(&l.facts) {
        return Err("this is a password or one-time-code field: the Vault doesn't fill those; ask the person to fill it".into());
    }
    let chosen = req.choose.is_some() && is_select(&l.facts);
    if !chosen && !takes_text(&l.facts) {
        return Err("this isn't a field that takes typed text".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn facts(tag: &str, attrs: &[(&str, &str)], label: &str) -> DomFacts {
        DomFacts {
            local_name: tag.into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(),
            label: label.into(),
        }
    }

    fn located(f: DomFacts) -> Located {
        Located { facts: f, origin: "https://irs.gov".into(), page: "https://irs.gov/form".into() }
    }

    fn request_for(l: &Located) -> FillReq {
        FillReq { block_id: "b".into(), ref_: "e1".into(), text: "x".into(), expect_origin: l.origin.clone(), expect_fingerprint: fingerprint(&l.facts), choose: None }
    }

    #[test]
    fn the_value_is_set_on_the_checked_field_not_typed_where_the_focus_is() {
        // In the order: origin, the same element and attributes, editable, marked,
        // then the value, all on `this`.
        let at = |needle: &str| CHECK_AND_SET.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at("w.origin !== origin") < at("this.localName !== tag"));
        // An about:blank or srcdoc frame's URL origin is "null"; its document's is its parent's.
        assert!(!CHECK_AND_SET.contains("location.origin"), "the document's origin, not its URL's");
        assert!(at("this.getAttribute(k)") < at("this.readOnly"));
        assert!(at("this.readOnly") < at("this.setAttribute(attr"));
        assert!(at("this.setAttribute(attr") < at("d.set.call(this, v)"));
        assert!(!CHECK_AND_SET.contains("activeElement"), "where the focus is doesn't decide where the value goes");
        assert!(!include_str!("vault_fill.rs").contains(&["insert", "_text("].concat()), "nothing is typed with Input.insertText");
        // A rich-text field gets the value as its text, and is checked by it:
        // a `value` set on a div changes nothing the person sees.
        assert!(at("this.isContentEditable") < at("this.setAttribute(attr"));
        assert!(CHECK_AND_SET.contains("this.textContent = v") && CHECK_AND_SET.contains("(rich ? this.textContent : this.value) === v"));
        assert!(!CHECK_AND_SET.contains("this.value = v"), "never an expando on an element with no value");
        // A field in a shadow root: the form's listener on its host or the
        // document hears the input event, as it would a typed one.
        for set in [CHECK_AND_SET, super::super::act::SET_VALUE] {
            assert!(set.contains(r#"new Event("input", { bubbles: true, composed: true })"#), "{set}");
        }
        for k in ["type", "name", "id", "autocomplete", "aria-label", "placeholder"] {
            assert!(FINGERPRINT_ATTRS.contains(&k), "{k} is in the approval's fingerprint and is rechecked");
        }
    }

    #[test]
    fn the_label_shown_is_what_a_person_reads_first() {
        assert_eq!(display_label(&facts("input", &[("name", "ssn1")], "Social Security Number")), "Social Security Number");
        assert_eq!(display_label(&facts("input", &[("name", "ssn1"), ("placeholder", "SSN")], "")), "SSN");
        assert_eq!(display_label(&facts("input", &[("name", "ssn1")], "")), "ssn1");
        assert_eq!(display_label(&facts("input", &[], "")), "(unnamed field)");
    }

    #[test]
    fn only_fields_that_take_text_are_filled() {
        assert!(takes_text(&facts("input", &[("type", "text")], "")));
        assert!(takes_text(&facts("input", &[], "")));
        assert!(takes_text(&facts("textarea", &[], "")));
        for ty in ["checkbox", "radio", "file", "submit", "hidden"] {
            assert!(!takes_text(&facts("input", &[("type", ty)], "")), "{ty}");
        }
        assert!(!takes_text(&facts("select", &[], "")));
        assert!(!takes_text(&facts("div", &[], "")));
    }

    #[test]
    fn a_fill_is_made_only_into_the_field_and_site_the_person_approved() {
        let ssn = located(facts("input", &[("name", "ssn")], "Social Security Number"));
        assert!(check_still_approved(&ssn, &request_for(&ssn)).is_ok());

        let mut other_site = request_for(&ssn);
        other_site.expect_origin = "https://evil.example".into();
        assert!(check_still_approved(&ssn, &other_site).unwrap_err().contains("not the site the person approved"));

        let renamed = located(facts("input", &[("name", "comment")], "Comment"));
        assert!(check_still_approved(&renamed, &request_for(&ssn)).unwrap_err().contains("changed while"));
    }

    #[test]
    fn a_password_or_one_time_code_field_is_refused_even_if_approved() {
        for f in [facts("input", &[("type", "password")], ""), facts("input", &[("autocomplete", "one-time-code")], ""), facts("input", &[("name", "otp")], "")] {
            let l = located(f);
            assert!(check_still_approved(&l, &request_for(&l)).unwrap_err().contains("password or one-time-code"));
        }
        let checkbox = located(facts("input", &[("type", "checkbox")], "Agree"));
        assert!(check_still_approved(&checkbox, &request_for(&checkbox)).unwrap_err().contains("takes typed text"));
    }

    // Fields that take a date are set whole; a value that can't be read as one is refused
    // without quoting it, since the date parser's own message would.
    #[test]
    fn a_date_that_cant_be_read_is_refused_without_quoting_it() {
        let ty = date_type(&facts("input", &[("type", "date")], "Date of birth")).expect("a date field");
        assert_eq!(super::super::dates::to_input_value(&ty, "1990-05-06").unwrap(), "1990-05-06");
        let err = super::super::dates::to_input_value(&ty, "05/06/1990").unwrap_err();
        assert!(err.contains("05/06/1990"), "the parser's own error quotes it: {err}");
        assert!(date_type(&facts("input", &[("type", "text")], "")).is_none());
    }

    #[test]
    fn a_list_is_chosen_in_only_when_the_request_says_so_and_is_checked_before_it_is_set() {
        let month = located(facts("select", &[("name", "exp_month")], "Month"));
        let mut req = request_for(&month);
        assert!(check_still_approved(&month, &req).unwrap_err().contains("takes typed text"), "a list is not typed in");
        req.choose = Some("month".into());
        assert!(check_still_approved(&month, &req).is_ok());
        assert!(is_select(&month.facts) && !takes_text(&month.facts));
        // A text field named in a choosing request is still refused as not being a list.
        let text = located(facts("input", &[("name", "exp_month")], "Month"));
        let mut asked = request_for(&text);
        asked.choose = Some("month".into());
        assert!(check_still_approved(&text, &asked).is_ok(), "approval is the same; `choose` refuses it before anything is set");

        let at = |needle: &str| CHOOSE.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at("w.origin !== origin") < at("this.localName !== tag"));
        assert!(at("this.getAttribute(k)") < at("this.setAttribute(attr"));
        assert!(at("this.setAttribute(attr") < at("this.selectedIndex = at"));
        assert!(at("if (at < 0) return { error: \"nooption\" }") < at("this.setAttribute(attr"), "nothing is marked or chosen without a matching option");
        assert!(!CHOOSE.contains("activeElement"));
        assert!(CHOOSE.contains(r#"new Event("input", { bubbles: true, composed: true })"#));
    }

    #[test]
    fn a_filled_field_is_secret_to_every_other_reader() {
        let mut f = facts("input", &[("name", "comment"), (TAINT_ATTR, "1")], "Comment");
        assert_eq!(snapshot::secret_reason(&f), Some("it holds a value filled from the Vault"));
        f.attrs.remove(TAINT_ATTR);
        assert_eq!(snapshot::secret_reason(&f), None);
    }
}
