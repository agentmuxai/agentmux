// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a browser pane holds, read for the agent's tools rather than acted
//! on: what has focus after `key` (the read-back), and where its secret
//! fields are, so a screenshot can black them out
//! (docs/specs/PLAN_BROWSER_PANE_FORM_FILLING_FIXES_2026_10_17.md §2.2, §4.1).

use serde_json::{json, Value};

use super::act::{isolated_context, redact_secret};
use super::cdp::CdpSession;
use super::frames;
use super::snapshot::{self, DomFacts};

/// The focused element's state, for the read-back after `key` (the same
/// fields as a fill's). Descends shadow roots and same-origin frames, as the
/// secret guard does; a focused frame from another site comes back as
/// `uninspectable`.
const FOCUSED_STATE: &str = r#"(() => {
  let doc = document;
  let el = doc.activeElement;
  for (let depth = 0; el && depth < 16; depth++) {
    if (el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el.tagName === "IFRAME" || el.tagName === "FRAME") {
      let inner = null;
      try { inner = el.contentDocument; } catch (e) { inner = null; }
      if (!inner) return { uninspectable: true };
      doc = inner;
      el = doc.activeElement;
      continue;
    }
    break;
  }
  if (!el || el === doc.body || el === doc.documentElement) return { nothing: true };
  const attrs = [];
  for (const a of el.attributes) { attrs.push(a.name, a.value); }
  const byIds = (el.getAttribute("aria-labelledby") || "").split(/\s+/).map(i => { const t = i && el.ownerDocument.getElementById(i); return t ? t.textContent : ""; }).join(" ");
  const label = ((el.labels ? Array.from(el.labels).map(l => l.textContent).join(" ") : "") + " " + byIds).replace(/\s+/g, " ").trim();
  const out = { localName: el.localName, attributes: attrs, label, tag: el.tagName.toLowerCase() };
  if (el.type) out.type = el.type;
  if (el.isContentEditable) { out.value = el.textContent; out.contentEditable = true; }
  else if ("value" in el && el.tagName !== "BUTTON") out.value = el.value;
  if ("checked" in el && (el.type === "checkbox" || el.type === "radio")) out.checked = el.checked;
  if (el.validationMessage) out.validationMessage = el.validationMessage;
  return out;
})()"#;

/// What has focus now, as an agent may see it: the fields a fill's read-back
/// has, a secret field's value hidden. `Value::Null` when it can't be read.
pub async fn focused_state(cdp: &mut CdpSession) -> Value {
    let Ok(ctx) = isolated_context(cdp, None).await else {
        return Value::Null;
    };
    let raw = cdp
        .call("Runtime.evaluate", json!({ "expression": FOCUSED_STATE, "contextId": ctx, "returnByValue": true }))
        .await
        .ok()
        .and_then(|v| v.pointer("/result/value").cloned())
        .unwrap_or(Value::Null);
    if raw.get("uninspectable").is_some() || raw.get("nothing").is_some() || raw.is_null() {
        return raw;
    }
    let facts = DomFacts::from_described(&raw);
    let mut shown = serde_json::Map::new();
    for key in ["tag", "type", "value", "checked", "validationMessage", "contentEditable"] {
        if let Some(v) = raw.get(key) {
            shown.insert(key.to_string(), v.clone());
        }
    }
    if !facts.label.is_empty() {
        shown.insert("label".to_string(), json!(facts.label));
    }
    let id = [facts.local_name.as_str(), facts.attrs.get("id").map(String::as_str).unwrap_or(""), facts.attrs.get("name").map(String::as_str).unwrap_or(""), facts.label.as_str()].join("|");
    shown.insert("element".to_string(), json!(id));
    redact_secret(Value::Object(shown), &facts)
}

/// Every field in the pane that is a secret field (passwords, one-time codes,
/// card and tax-ID numbers), with its box on the page in the main frame's CSS
/// pixels, so a screenshot can black them out before an agent sees it
/// (frames spec §5.1, plan §4.1). A frame that can't be looked into is
/// returned whole: fail closed. `css_width` is the page's viewport width, to
/// scale the boxes to the image.
pub struct SecretBoxes {
    pub boxes: Vec<[f64; 4]>,
    pub css_width: Option<f64>,
    /// Every document was checked, or hidden whole. False when the top
    /// document couldn't be read, or a frame could neither be read nor
    /// placed: the screenshot must then not be given out.
    pub complete: bool,
}

/// Fields scanned per document. Past it the list is incomplete, which counts
/// as not being able to read the document (it is hidden, or the screenshot
/// refused), never as "no secret fields".
const MAX_SECRET_CANDIDATES: usize = 2000;

/// Lists the fields that could hold a secret, with their facts and boxes in
/// this document's viewport. Descends open shadow roots.
const SECRET_CANDIDATES: &str = r#"((MAX) => {
  const out = [];
  let truncated = false;
  const visit = (root) => {
    for (const el of root.querySelectorAll("*")) {
      if (out.length >= MAX) { truncated = true; return; }
      if (el.shadowRoot) visit(el.shadowRoot);
      const tag = el.localName;
      if (!(tag === "input" || tag === "textarea" || (el.isContentEditable && !(el.parentElement && el.parentElement.isContentEditable)))) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const attrs = [];
      for (const a of el.attributes) { attrs.push(a.name, a.value); }
      const byIds = (el.getAttribute("aria-labelledby") || "").split(/\s+/).map(i => { const t = i && el.ownerDocument.getElementById(i); return t ? t.textContent : ""; }).join(" ");
      const label = ((el.labels ? Array.from(el.labels).map(l => l.textContent).join(" ") : "") + " " + byIds).replace(/\s+/g, " ").trim();
      out.push({ localName: tag, attributes: attrs, label, x: r.left, y: r.top, w: r.width, h: r.height });
    }
  };
  visit(document);
  return { fields: out, width: window.innerWidth, truncated };
})"#;

/// The secret fields' boxes in one document (`frame_id`, or the main frame),
/// in that document's viewport. `None`: it couldn't be read completely.
async fn secret_boxes_in(cdp: &mut CdpSession, frame_id: Option<&str>) -> Option<(Vec<[f64; 4]>, Option<f64>)> {
    let ctx = isolated_context(cdp, frame_id).await.ok()?;
    let expression = format!("{SECRET_CANDIDATES}({MAX_SECRET_CANDIDATES})");
    let v = cdp
        .call("Runtime.evaluate", json!({ "expression": expression, "contextId": ctx, "returnByValue": true }))
        .await
        .ok()?;
    if v.get("exceptionDetails").is_some() {
        return None;
    }
    let result = v.pointer("/result/value")?;
    if result.get("truncated").and_then(|t| t.as_bool()) != Some(false) {
        return None;
    }
    let fields = result.get("fields")?.as_array()?;
    let boxes = fields
        .iter()
        .filter(|f| snapshot::is_secret(&DomFacts::from_described(f)))
        .filter_map(|f| {
            let n = |k: &str| f.get(k).and_then(|v| v.as_f64());
            Some([n("x")?, n("y")?, n("w")?, n("h")?])
        })
        .collect();
    Some((boxes, result.get("width").and_then(|w| w.as_f64())))
}

/// [`SecretBoxes`] for the whole pane: the top frame and every iframe.
pub async fn secret_boxes(root: &mut CdpSession) -> SecretBoxes {
    let mut out = SecretBoxes { boxes: Vec::new(), css_width: None, complete: true };
    match secret_boxes_in(root, None).await {
        Some((boxes, width)) => {
            out.boxes.extend(boxes);
            out.css_width = width;
        }
        // The page itself couldn't be checked: there is nothing to hide whole
        // short of the screenshot itself.
        None => out.complete = false,
    }
    if out.css_width.is_none() {
        out.css_width = root
            .call("Runtime.evaluate", json!({ "expression": "window.innerWidth", "returnByValue": true }))
            .await
            .ok()
            .and_then(|v| v.pointer("/result/value").and_then(|w| w.as_f64()));
    }
    let (laid_out, unplaced) = frames::layout_counting_unplaced(root).await;
    if unplaced > 0 {
        // Frames whose parent isn't known can be neither checked nor hidden.
        out.complete = false;
    }
    for fl in laid_out {
        if !fl.placeable {
            // Its <iframe> wasn't found: its boxes would land in the wrong place.
            out.complete = false;
            continue;
        }
        let mut on = root.in_session(fl.raw.session.as_deref());
        match secret_boxes_in(&mut on, Some(&fl.raw.id)).await {
            Some((boxes, _)) if boxes.is_empty() => {}
            Some((boxes, _)) => match frames::chain_offset(root, &fl.chain).await {
                Ok((ox, oy)) => out.boxes.extend(boxes.into_iter().map(|[x, y, w, h]| [x + ox, y + oy, w, h])),
                // Can't place its fields on the page: hide the whole frame.
                Err(_) => hide_whole_frame(root, &fl.chain, &mut out).await,
            },
            // Couldn't look inside: hide the whole frame.
            None => hide_whole_frame(root, &fl.chain, &mut out).await,
        }
    }
    out
}

/// Hide a frame that couldn't be checked; if it can't even be placed on the
/// page, the scan is incomplete.
async fn hide_whole_frame(root: &mut CdpSession, chain: &[frames::FrameHop], out: &mut SecretBoxes) {
    match frames::frame_box(root, chain).await {
        Some(b) => out.boxes.push(b),
        None => out.complete = false,
    }
}
