// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `fill` with the agent's own text into a password, one-time-code, card, bank or taxpayer-ID field
//! (docs/reports/REPORT_AGENT_AUTONOMY_BLOCKERS_2026_11_03.md): who may, how the value is set, and
//! what is answered and logged. Split out of `act.rs`.

use serde_json::{json, Value};

use super::act::{call_on, redact_secret, secret_refusal, READ_BACK};
use super::cdp::CdpSession;
use super::frames::FrameRef;
use super::snapshot::{self, DomFacts};

/// May a secret be typed into a document of `origin`? Only when the pane is limited to sites and
/// the origin is on that list, over https (or to this machine). An `about:` or `srcdoc` document has
/// no origin of its own to name, so it never is (and `fill_if_secret` gives such a frame none).
fn secret_origin_ok(allowed: Option<&[String]>, origin: &str) -> bool {
    let Some(list) = allowed else { return false };
    let secure = origin.starts_with("https://")
        || ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|h| origin == *h || origin.starts_with(&format!("{h}:")));
    secure && agentmux_common::allowed_origins::target_origin(origin).is_some() && agentmux_common::allowed_origins::allows(list, origin)
}

/// The secret-field rule for `fill` with the agent's own text. `Ok(None)`: an ordinary field.
/// `Ok(Some(kind))`: a password, code, card, bank or ID field the agent may fill, because the pane is
/// limited to sites and its document's origin is on that list ([`secret_origin_ok`]); `kind` goes in
/// the pane's Log, never the value. `Err`: refused. A field the Vault filled stays refused, and so
/// does any secret field in a pane with no list of sites (the person's own tabs, where nothing says
/// the page is one the agent was sent to).
fn secret_fill_gate(facts: &DomFacts, origin_ok: bool) -> Result<Option<&'static str>, String> {
    let Some(why) = snapshot::secret_reason(facts) else { return Ok(None) };
    if why == super::secret::FILLED_FROM_VAULT {
        return Err(secret_refusal(why));
    }
    if !origin_ok {
        return Err(format!(
            "this is a password, one-time-code, card, bank or taxpayer ID field ({why}): an agent fills one only in a pane opened with allowed_origins \
             that lists this page's site over https, and this page isn't on that list (or the pane has none). Open the pane with this site in allowed_origins, \
             or ask the user to fill it in the pane."
        ));
    }
    Ok(Some(super::secret::kind_of_reason(why)))
}

/// Sets a secret field's value in one synchronous call on the field itself, so nothing depends on
/// where the focus is (a page can move it, even into another site's frame, between a check and a
/// keystroke): still in a document of `origin`, not disabled or read-only, a field with a value, and
/// no field in the document holds a Vault value (a "confirm" field beside one would tell, by the
/// page's own check, whether a guess matches it). The value goes in through the native setter with
/// input and change events, as [`SET_VALUE`] does. It answers whether the field kept the value, never
/// the value.
const SET_SECRET: &str = r#"function (origin, v, attr) {
  const w = this.ownerDocument && this.ownerDocument.defaultView;
  // An about:blank or srcdoc document has the origin of the page that made it, but no site of its own.
  if (!this.isConnected || !w || w.origin !== origin || !/^https?:$/.test(w.location.protocol)) return { error: "origin" };
  if (this.disabled || this.readOnly) return { error: "the field is disabled or read-only" };
  // A Vault-filled field anywhere the page can compare it with: this document, its open shadow roots, and
  // the same-origin frames above and below it (a frame of another origin can't be read, nor can it read this).
  const inFrame = (f) => { try { return !!f.contentDocument && tainted(f.contentDocument); } catch (_) { return false; } };
  const tainted = (root) => !!root.querySelector("[" + attr + "]") ||
    Array.from(root.querySelectorAll("*")).some((e) => (e.shadowRoot && tainted(e.shadowRoot)) || ((e.localName === "iframe" || e.localName === "frame") && inFrame(e)));
  let top = w;
  try { while (top.parent && top.parent !== top && top.parent.document) top = top.parent; } catch (_) {}
  if (tainted(top.document)) return { error: "vault" };
  const d = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(this), "value");
  const rich = !(d && d.set) && this.isContentEditable;
  if (!(d && d.set) && !rich) return { error: "this element can't be filled; click it, or fill the text field inside it" };
  if (rich) { this.textContent = v; } else { d.set.call(this, v); }
  this.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true, same: (rich ? this.textContent : this.value) === v };
}"#;

/// `fill` with the agent's `text`, when `facts` is a secret field: `Ok(None)` for an ordinary one (the
/// caller types it), `Ok(Some(answer))` once it is set, `Err` when refused. The document's origin is the
/// frame's own (its site was also checked against the list, `refuse_if_blocked`), else the page's `page`.
pub(super) async fn fill_if_secret(
    cdp: &mut CdpSession,
    obj: &str,
    facts: &DomFacts,
    text: &str,
    frame: Option<&FrameRef>,
    page: &str,
    allowed: Option<&[String]>,
) -> Result<Option<Value>, String> {
    // A frame with no web address of its own (about:blank, srcdoc) shows its parent's origin: not one it can be trusted by.
    let origin = match frame {
        Some(f) if agentmux_common::allowed_origins::target_origin(&f.url).is_some() => f.origin.clone(),
        Some(_) => String::new(),
        None => agentmux_common::allowed_origins::target_origin(page).unwrap_or_default(),
    };
    let Some(kind) = secret_fill_gate(facts, secret_origin_ok(allowed, &origin))? else { return Ok(None) };
    // The browser counts maxlength in UTF-16 code units; a secret's length is not said back.
    if let Some(max) = snapshot::max_length(facts).filter(|max| text.encode_utf16().count() > *max) {
        return Err(format!("this field holds at most {max} characters and the text is longer: shorten it and fill again (nothing was set)"));
    }
    fill_secret(cdp, obj, facts, kind, &origin, text).await.map(Some)
}

/// `fill` with the agent's text into a secret field: set in one call (see [`SET_SECRET`]), answered
/// without the value or its length.
async fn fill_secret(cdp: &mut CdpSession, obj: &str, facts: &DomFacts, kind: &str, origin: &str, text: &str) -> Result<Value, String> {
    let set = match call_on(cdp, obj, SET_SECRET, vec![json!(origin), json!(text), json!(super::vault_fill::TAINT_ATTR)]).await {
        Ok(v) => v,
        Err(e) if e == "origin" => return Err(format!("the field is no longer in a document of {origin}, so nothing was set; take a new snapshot")),
        Err(e) if e == "vault" => {
            return Err("this page holds a field filled from the Vault, and filling another secret field here could let a guess be checked against it, \
                        so it is refused: ask the user to fill it in the pane"
                .to_string())
        }
        Err(e) => return Err(e),
    };
    let kept = set.get("same").and_then(|v| v.as_bool()) == Some(true);
    let after = call_on(cdp, obj, READ_BACK, vec![]).await?;
    let mut answer = secret_fill_answer(after, facts, kind, kept);
    if let Some(o) = answer.as_object_mut() {
        o.insert("filledBy".to_string(), json!("set"));
    }
    Ok(answer)
}

/// What a fill into a secret field answers: that it was filled, as which kind, and whether the page
/// kept what was typed. `secret_fill` is also what the pane's Log records (srv); the value, and so
/// its length, is in neither.
pub(super) fn secret_fill_answer(after: Value, facts: &DomFacts, kind: &str, kept: bool) -> Value {
    let mut after = redact_secret(after, facts);
    if let Some(o) = after.as_object_mut() {
        // The browser's own message can count the characters ("you are currently using 9").
        o.remove("validationMessage");
        o.insert("secret_fill".to_string(), json!(kind));
        if !kept {
            o.insert(
                "warning".to_string(),
                json!("the field doesn't hold what was typed: the page changed it (a format mask or a script); check the page's own message"),
            );
        }
    }
    after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(tag: &str, attrs: &[(&str, &str)]) -> DomFacts {
        DomFacts { local_name: tag.into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), label: String::new() }
    }

    #[test]
    fn a_secret_field_is_filled_only_in_a_document_of_a_listed_https_origin() {
        let list = vec!["https://appstoreconnect.apple.com".to_string(), "http://localhost:3000".to_string()];
        let on = |origin: &str| secret_origin_ok(Some(&list), origin);
        assert!(on("https://appstoreconnect.apple.com"));
        assert!(on("http://localhost:3000"));
        // off the list, plain http, a document with no origin of its own, or a pane with no list
        assert!(!on("https://evil.example"));
        assert!(!on("http://appstoreconnect.apple.com"));
        assert!(!on("about:blank") && !on("") && !on("null"));
        assert!(!secret_origin_ok(None, "https://appstoreconnect.apple.com"));
        let pw = facts("input", &[("type", "password")]);
        let tin = DomFacts { label: "Taxpayer ID number".into(), ..facts("input", &[("type", "text")]) };
        assert_eq!(secret_fill_gate(&pw, true), Ok(Some("password")));
        assert_eq!(secret_fill_gate(&tin, true), Ok(Some("taxpayer ID")));
        let why = secret_fill_gate(&pw, false).unwrap_err();
        assert!(why.contains("allowed_origins"), "{why}");
        // an ordinary field is not the gate's business
        assert_eq!(secret_fill_gate(&facts("input", &[("name", "city")]), false), Ok(None));
        assert_eq!(secret_fill_gate(&facts("input", &[("name", "city")]), true), Ok(None));
    }

    #[test]
    fn a_field_the_vault_filled_is_refused_even_on_a_listed_page() {
        let f = facts("input", &[("name", "comment"), (super::super::vault_fill::TAINT_ATTR, "1")]);
        assert!(secret_fill_gate(&f, true).is_err());
    }

    #[test]
    fn a_secret_fill_answers_without_the_value_or_its_length() {
        let pw = facts("input", &[("type", "password")]);
        let after = json!({ "tag": "input", "type": "password", "value": "hunter2-hunter2" });
        let out = secret_fill_answer(after, &pw, "password", true);
        assert_eq!(out["secret_fill"], "password");
        assert_eq!(out["value"], "[secret: hidden]");
        let wire = out.to_string();
        assert!(!wire.contains("hunter2"), "{wire}");
        // the browser's own message counts characters: it is dropped
        let counted = secret_fill_answer(json!({ "value": "x", "validationMessage": "you are currently using 15 characters" }), &pw, "password", true);
        assert!(counted.get("validationMessage").is_none() && !counted.to_string().contains("15"), "{counted}");
        // a page that changed it is said without a count
        let changed = secret_fill_answer(json!({ "value": "hunter" }), &pw, "password", false);
        let wire = changed.to_string();
        assert!(changed["warning"].as_str().unwrap().contains("changed it") && !wire.contains("hunter"), "{wire}");
    }

}
