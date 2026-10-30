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
    // Which element has focus, by its node: fields alike in tag, id, name and
    // label (a row of one-digit code inputs) are still told apart.
    let id = match focused_node(cdp, ctx).await {
        Some(node) => format!("node:{node}"),
        None => [facts.local_name.as_str(), facts.attrs.get("id").map(String::as_str).unwrap_or(""), facts.attrs.get("name").map(String::as_str).unwrap_or(""), facts.label.as_str()].join("|"),
    };
    shown.insert("element".to_string(), json!(id));
    redact_secret(Value::Object(shown), &facts)
}

/// The element that really has focus (through shadow roots and same-origin
/// frames), as a page object, or null.
const DEEP_ACTIVE: &str = r#"(() => {
  let doc = document;
  let el = doc.activeElement;
  for (let depth = 0; el && depth < 16; depth++) {
    if (el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el.tagName === "IFRAME" || el.tagName === "FRAME") {
      let inner = null;
      try { inner = el.contentDocument; } catch (e) { inner = null; }
      if (!inner) return null;
      doc = inner;
      el = doc.activeElement;
      continue;
    }
    break;
  }
  return el || null;
})()"#;

/// The backend node id of the focused element, if it can be read.
async fn focused_node(cdp: &mut CdpSession, ctx: i64) -> Option<i64> {
    let v = cdp
        .call("Runtime.evaluate", json!({ "expression": DEEP_ACTIVE, "contextId": ctx, "returnByValue": false }))
        .await
        .ok()?;
    let object_id = v.pointer("/result/objectId")?.as_str()?.to_string();
    let node = cdp.call("DOM.describeNode", json!({ "objectId": object_id })).await.ok()?;
    node.pointer("/node/backendNodeId").and_then(Value::as_i64)
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
    /// How many of the boxes hold a Vault value or a secret the page shows once
    /// (spec §15.1), not just a password field: `eval` is refused while any does.
    pub vault_values: usize,
    /// Every document's contents were read, hidden frames included. A frame
    /// that couldn't be read is hidden whole, which keeps a screenshot
    /// `complete`, but whether it holds a Vault value is unknown: `eval` is
    /// refused then too.
    pub read_all: bool,
}

/// One frame's scan: its boxes in its own viewport, its width, and how many hold a
/// Vault value; `None` when it couldn't be read.
type FrameScan = Option<(Vec<[f64; 4]>, Option<f64>, usize)>;

/// What one shown frame adds to the pane's scan. `plain`: its contents can be
/// placed on the page by `offset`; otherwise the frame is hidden whole (`bounds`).
fn add_frame(out: &mut SecretBoxes, scan: FrameScan, plain: bool, (ox, oy): (f64, f64), bounds: [f64; 4]) {
    match scan {
        Some((boxes, _, _)) if boxes.is_empty() => {}
        Some((boxes, _, vault)) if plain => {
            out.boxes.extend(boxes.into_iter().map(|[x, y, w, h]| [x + ox, y + oy, w, h]));
            out.vault_values += vault;
        }
        // Its own <iframe> is scaled or rotated (its fields can't be placed by
        // an offset): hide the whole frame.
        Some((_, _, vault)) => {
            out.boxes.push(bounds);
            out.vault_values += vault;
        }
        // Couldn't look inside: hide the whole frame; what it holds is unknown.
        None => {
            out.boxes.push(bounds);
            out.read_all = false;
        }
    }
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
      // A read-only field's value: where a page shows a secret once (spec §15.1).
      const value = el.readOnly ? String(el.value == null ? "" : el.value) : "";
      out.push({ localName: tag, attributes: attrs, label, value, x: r.left, y: r.top, w: r.width, h: r.height });
    }
  };
  visit(document);
  // Text where a page shows a secret once: code, pre, kbd and samp, and the
  // nearest container of a button named like "Copy"; and any element holding a
  // captured Vault value. Only words that could be one leave the page (the host
  // tests the exact shape), at most WORDS per element: one with more is boxed whole.
  const WORDS = 20;
  const shown = [];
  const could = (w) => w.length >= 20 && w.length <= 512 && !/^[0-9a-fA-F]+$/.test(w) && !w.includes("://")
    && [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((c) => c.test(w)).length >= 2;
  const box = (el, tainted) => {
    if (shown.length >= MAX) { truncated = true; return; }
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return;
    const words = tainted ? [] : String(el.textContent || "").split(/\s+/).filter(could);
    shown.push({ words: words.slice(0, WORDS), many: words.length > WORDS, tainted, x: r.left, y: r.top, w: r.width, h: r.height });
  };
  for (const el of document.querySelectorAll("[data-agentmux-vault]")) {
    if (el.localName !== "input" && el.localName !== "textarea" && !el.isContentEditable) box(el, true);
  }
  for (const el of document.querySelectorAll("code, pre, kbd, samp")) box(el, false);
  for (const b of document.querySelectorAll("button, a, [role=button]")) {
    const name = ((b.textContent || "") + " " + (b.getAttribute("aria-label") || "") + " " + (b.getAttribute("title") || "")).toLowerCase();
    if (!name.includes("copy")) continue;
    let c = b.parentElement;
    for (let i = 0; c && c.children.length <= 1 && i < 4; i++) c = c.parentElement;
    if (c) box(c, false);
  }
  return { fields: out, shown, width: window.innerWidth, truncated };
})"#;

/// The secret fields' boxes in one document (`frame_id`, or the main frame),
/// in that document's viewport. `None`: it couldn't be read completely.
async fn secret_boxes_in(cdp: &mut CdpSession, frame_id: Option<&str>) -> Option<(Vec<[f64; 4]>, Option<f64>, usize)> {
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
    let shown = result.get("shown").and_then(Value::as_array).cloned().unwrap_or_default();
    let rect = |f: &Value| -> Option<[f64; 4]> {
        let n = |k: &str| f.get(k).and_then(|v| v.as_f64());
        Some([n("x")?, n("y")?, n("w")?, n("h")?])
    };
    let mut boxes = Vec::new();
    let mut vault_values = 0;
    for f in fields {
        let facts = DomFacts::from_described(f);
        let filled = !facts.attr(super::vault_fill::TAINT_ATTR).is_empty();
        let once = f.get("value").and_then(Value::as_str).is_some_and(agentmux_common::shown_once::has_shown_once);
        if snapshot::is_secret(&facts) || once {
            boxes.extend(rect(f));
            vault_values += usize::from(filled || once);
        }
    }
    for s in &shown {
        let flag = |k: &str| s.get(k).and_then(Value::as_bool).unwrap_or(false);
        let words = s.get("words").and_then(Value::as_array);
        let once = words.is_some_and(|w| w.iter().filter_map(Value::as_str).any(agentmux_common::shown_once::looks_shown_once));
        if flag("tainted") || once {
            boxes.extend(rect(s));
            vault_values += 1;
        } else if flag("many") {
            // Too many words to send: hidden whole in the image, but not counted
            // as a secret (a long code listing mustn't block eval).
            boxes.extend(rect(s));
        }
    }
    Some((boxes, result.get("width").and_then(|w| w.as_f64()), vault_values))
}

/// Whether a `DOM.getDocument { depth: -1, pierce: true }` tree has a text
/// field inside a closed shadow root, which the page script can't see (so
/// it can't be checked for being a secret field).
fn closed_shadow_has_fields(node: &Value, inside_closed: bool) -> bool {
    let name = node.get("nodeName").and_then(Value::as_str).unwrap_or("").to_ascii_uppercase();
    let editable = node
        .get("attributes")
        .and_then(Value::as_array)
        .is_some_and(|a| a.chunks(2).any(|kv| kv.first().and_then(Value::as_str) == Some("contenteditable") && kv.get(1).and_then(Value::as_str) != Some("false")));
    if inside_closed && (name == "INPUT" || name == "TEXTAREA" || editable) {
        return true;
    }
    let shadows = node.get("shadowRoots").and_then(Value::as_array).into_iter().flatten();
    for root in shadows {
        let closed = root.get("shadowRootType").and_then(Value::as_str) == Some("closed");
        if closed_shadow_has_fields(root, inside_closed || closed) {
            return true;
        }
    }
    let children = node.get("children").and_then(Value::as_array).into_iter().flatten();
    if children.into_iter().any(|c| closed_shadow_has_fields(c, inside_closed)) {
        return true;
    }
    // A same-process frame's document comes along under its <iframe>.
    node.get("contentDocument").is_some_and(|d| closed_shadow_has_fields(d, false))
}

/// Whether the documents reached through `cdp` hold a field the scan can't
/// see (in a closed shadow root); `None` when the tree couldn't be read.
async fn has_unseen_fields(cdp: &mut CdpSession) -> Option<bool> {
    let doc = cdp.call("DOM.getDocument", json!({ "depth": -1, "pierce": true })).await.ok()?;
    Some(closed_shadow_has_fields(doc.get("root")?, false))
}

/// [`SecretBoxes`] for the whole pane: the top frame and every iframe.
pub async fn secret_boxes(root: &mut CdpSession) -> SecretBoxes {
    let mut out = SecretBoxes { boxes: Vec::new(), css_width: None, complete: true, vault_values: 0, read_all: true };
    match secret_boxes_in(root, None).await {
        Some((boxes, width, vault)) => {
            out.boxes.extend(boxes);
            out.css_width = width;
            out.vault_values += vault;
        }
        // The page itself couldn't be checked: there is nothing to hide whole
        // short of the screenshot itself.
        None => {
            out.complete = false;
            out.read_all = false;
        }
    }
    if out.css_width.is_none() {
        out.css_width = root
            .call("Runtime.evaluate", json!({ "expression": "window.innerWidth", "returnByValue": true }))
            .await
            .ok()
            .and_then(|v| v.pointer("/result/value").and_then(|w| w.as_f64()));
    }
    // Fields in closed shadow roots are out of the page script's reach: the
    // page and its same-process frames here, frames in their own process below.
    if has_unseen_fields(root).await != Some(false) {
        out.complete = false;
    }
    let (laid_out, unplaced) = frames::layout_counting_unplaced(root).await;
    if unplaced > 0 {
        // Frames whose parent isn't known can be neither checked nor hidden.
        out.complete = false;
    }
    for fl in laid_out {
        // Its <iframe> (or an outer one) wasn't found, has no box, or sits
        // in a transformed outer frame: nothing in it can be placed, or even
        // hidden, on the page.
        let geometry = match frames::chain_geometry(root, &fl.chain).await {
            frames::Placement::Shown(g) if fl.placeable && g.outer_plain => g,
            // Not drawn (a hidden tracking or captcha frame): nothing of it
            // can appear in the image, but it may still hold a Vault value.
            frames::Placement::NotRendered if fl.placeable => {
                let mut on = root.in_session(fl.raw.session.as_deref());
                match secret_boxes_in(&mut on, Some(&fl.raw.id)).await {
                    Some((_, _, vault)) => out.vault_values += vault,
                    None => out.read_all = false,
                }
                continue;
            }
            _ => {
                out.complete = false;
                continue;
            }
        };
        let mut on = root.in_session(fl.raw.session.as_deref());
        if fl.is_session_root && has_unseen_fields(&mut on).await != Some(false) {
            out.complete = false;
        }
        let scan = secret_boxes_in(&mut on, Some(&fl.raw.id)).await;
        add_frame(&mut out, scan, geometry.plain, geometry.offset, geometry.bounds);
    }
    out
}

/// The scans before and after one capture, as one: the boxes of both, and
/// complete only if both were complete and found the same fields in the same
/// places (the page didn't move or change while the image was taken).
pub fn merge_scans(before: SecretBoxes, after: SecretBoxes) -> SecretBoxes {
    let same = before.boxes.len() == after.boxes.len()
        && before.boxes.iter().zip(&after.boxes).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0));
    // A resize between the scans changes how the boxes scale to the image.
    let same_width = match (before.css_width, after.css_width) {
        (Some(a), Some(b)) => (a - b).abs() < 0.5,
        _ => false,
    };
    let complete = before.complete && after.complete && same && same_width;
    let css_width = before.css_width.or(after.css_width);
    let vault_values = before.vault_values.max(after.vault_values);
    let read_all = before.read_all && after.read_all;
    let mut boxes = before.boxes;
    boxes.extend(after.boxes);
    SecretBoxes { boxes, css_width, complete, vault_values, read_all }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(boxes: &[[f64; 4]], complete: bool) -> SecretBoxes {
        SecretBoxes { boxes: boxes.to_vec(), css_width: Some(800.0), complete, vault_values: 0, read_all: true }
    }

    #[test]
    fn a_frame_that_couldnt_be_read_is_hidden_whole_and_leaves_eval_refused() {
        let bounds = [10.0, 20.0, 300.0, 200.0];
        let mut out = scan(&[], true);
        add_frame(&mut out, None, true, (10.0, 20.0), bounds);
        assert_eq!(out.boxes, vec![bounds]);
        assert!(out.complete, "the screenshot is still safe: the frame is hidden whole");
        assert!(!out.read_all, "but what it holds is unknown");

        let mut out = scan(&[], true);
        add_frame(&mut out, Some((vec![[1.0, 2.0, 3.0, 4.0]], None, 1)), false, (10.0, 20.0), bounds);
        assert_eq!((out.boxes.clone(), out.vault_values, out.read_all), (vec![bounds], 1, true), "read, but hidden whole");

        let mut out = scan(&[], true);
        add_frame(&mut out, Some((vec![[1.0, 2.0, 3.0, 4.0]], None, 0)), true, (10.0, 20.0), bounds);
        assert_eq!((out.boxes.clone(), out.vault_values, out.read_all), (vec![[11.0, 22.0, 3.0, 4.0]], 0, true));
    }

    #[test]
    fn two_matching_scans_are_complete_and_keep_their_boxes() {
        let m = merge_scans(scan(&[[1.0, 2.0, 3.0, 4.0]], true), scan(&[[1.2, 2.0, 3.0, 4.0]], true));
        assert!(m.complete);
        assert_eq!(m.boxes.len(), 2);
    }

    #[test]
    fn a_page_that_moved_or_changed_between_scans_is_incomplete_and_hides_both() {
        let moved = merge_scans(scan(&[[1.0, 2.0, 3.0, 4.0]], true), scan(&[[40.0, 2.0, 3.0, 4.0]], true));
        assert!(!moved.complete);
        assert_eq!(moved.boxes.len(), 2, "both positions stay hidden");
        let added = merge_scans(scan(&[], true), scan(&[[1.0, 2.0, 3.0, 4.0]], true));
        assert!(!added.complete, "a field appeared during the capture");
        assert!(!merge_scans(scan(&[], false), scan(&[], true)).complete);
    }

    #[test]
    fn a_field_in_a_closed_shadow_root_is_found_and_an_open_one_is_left_to_the_script() {
        let closed = json!({ "nodeName": "#document", "children": [
            { "nodeName": "PAY-FORM", "shadowRoots": [
                { "nodeName": "#document-fragment", "shadowRootType": "closed", "children": [ { "nodeName": "INPUT" } ] }
            ] }
        ] });
        assert!(closed_shadow_has_fields(&closed, false));
        let open = json!({ "nodeName": "#document", "children": [
            { "nodeName": "X-FIELD", "shadowRoots": [
                { "nodeName": "#document-fragment", "shadowRootType": "open", "children": [ { "nodeName": "INPUT" } ] }
            ] }
        ] });
        assert!(!closed_shadow_has_fields(&open, false), "the page script reaches open roots");
        let editable = json!({ "nodeName": "#document", "children": [ { "nodeName": "X", "shadowRoots": [
            { "shadowRootType": "closed", "children": [ { "nodeName": "DIV", "attributes": ["contenteditable", "true"] } ] }
        ] } ] });
        assert!(closed_shadow_has_fields(&editable, false));
        let framed = json!({ "nodeName": "IFRAME", "contentDocument": closed });
        assert!(closed_shadow_has_fields(&framed, false), "a same-process frame's document is searched too");
    }

    #[test]
    fn a_resize_between_the_scans_is_incomplete() {
        let mut wider = scan(&[[1.0, 2.0, 3.0, 4.0]], true);
        wider.css_width = Some(1200.0);
        assert!(!merge_scans(scan(&[[1.0, 2.0, 3.0, 4.0]], true), wider).complete);
        let mut unknown = scan(&[], true);
        unknown.css_width = None;
        assert!(!merge_scans(scan(&[], true), unknown).complete, "an unknown width can't be checked");
    }
}
