// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/agentmux/browser/snapshot` and `/agentmux/browser/act`: read a browser
//! pane as an accessibility snapshot with element references, and click,
//! fill, select or check by reference
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §4).
//!
//! Elements are acted on in an **isolated JavaScript world**
//! (`Page.createIsolatedWorld`): it shares the page's DOM but not its
//! JavaScript, so page scripts can't replace the helper functions below or
//! the DOM prototypes they use. Clicks are real mouse events at the element's
//! centre and typing is `Input.insertText`, both trusted input the page
//! can't tell from a person's.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::cdp::CdpSession;
use super::snapshot::{self, DomFacts};
use super::types::ApiResponse;
use crate::state::AppState;

/// The references one snapshot handed out for one pane, and the page they
/// belong to. A new snapshot or a navigation makes them stale.
#[derive(Debug, Clone, Default)]
pub struct RefTable {
    pub url: String,
    pub refs: HashMap<String, i64>,
}

/// Per-host store of each pane's latest reference table, by block id.
/// Bounded: past `MAX_TABLES` panes, the least recently snapshotted is
/// dropped (its agent just takes a new snapshot).
#[derive(Default)]
pub struct RefTables(std::sync::Mutex<(u64, HashMap<String, (u64, RefTable)>)>);

const MAX_TABLES: usize = 64;

impl RefTables {
    fn put(&self, block_id: &str, table: RefTable) {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        g.0 += 1;
        let stamp = g.0;
        g.1.insert(block_id.to_string(), (stamp, table));
        while g.1.len() > MAX_TABLES {
            let oldest = g.1.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    g.1.remove(&k);
                }
                None => break,
            }
        }
    }
    fn get(&self, block_id: &str) -> Option<RefTable> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).1.get(block_id).map(|(_, t)| t.clone())
    }
}

#[derive(Debug, Deserialize)]
pub struct SnapshotReq {
    pub block_id: String,
    /// A reference from the previous snapshot: narrow to that element's subtree.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotData {
    pub url: String,
    pub snapshot: String,
    pub refs: usize,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
pub struct ActReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    /// `click`, `fill`, `select` or `check`.
    pub action: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub option: Option<String>,
    #[serde(default)]
    pub checked: Option<bool>,
    /// The user approved this committing action in the pane (srv sets it
    /// only after their click, spec §5.4).
    #[serde(default)]
    pub approved: bool,
    /// The `needs_approval` the user approved. The click is made only if the
    /// form still looks the same; the page could have changed it meanwhile.
    #[serde(default)]
    pub approved_as: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ActData {
    /// The element's state after acting: value, checked, validation message.
    pub after: Value,
}

// ── Functions run on the element (`this`) in the isolated world ────────────

/// Scroll into view and report the centre, in viewport coordinates.
const CLICK_POINT: &str = r#"function () {
  this.scrollIntoView({ block: "center", inline: "center" });
  const r = this.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return { error: "the element is not visible" };
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}"#;

/// Focus and select the current contents, so inserted text replaces them.
const PREPARE_FILL: &str = r#"function () {
  if (this.disabled || (this.matches && this.matches(":disabled"))) return { error: "the field is disabled" };
  if (this.readOnly) return { error: "the field is read-only" };
  this.scrollIntoView({ block: "center", inline: "center" });
  this.focus();
  if (typeof this.select === "function") {
    this.select();
  } else if (this.isContentEditable) {
    const range = document.createRange();
    range.selectNodeContents(this);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  } else {
    return { error: "this element can't be filled; click it, or fill the text field inside it" };
  }
  return { ok: true };
}"#;

/// Set the value through the native setter of the element's own prototype
/// (the HTMLInputElement setter throws on a textarea) and fire input/change,
/// for widgets that ignore inserted text. React's value tracker sees the
/// change because the setter isn't its wrapped one.
const SET_VALUE: &str = r#"function (v) {
  const proto = Object.getPrototypeOf(this);
  const d = Object.getOwnPropertyDescriptor(proto, "value");
  if (d && d.set) { d.set.call(this, v); } else if (this.isContentEditable) { this.textContent = v; } else { this.value = v; }
  this.dispatchEvent(new Event("input", { bubbles: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return true;
}"#;

/// Choose an option of a native <select> by its label or value.
const SELECT_OPTION: &str = r#"function (want) {
  if (this.tagName !== "SELECT") return { error: "not a <select>; click the control to open it, then click the option" };
  if (this.disabled || this.matches(":disabled")) return { error: "the dropdown is disabled" };
  const w = String(want).trim().toLowerCase();
  const opts = Array.from(this.options);
  const hit = opts.find(o => o.label.trim().toLowerCase() === w || o.text.trim().toLowerCase() === w)
    || opts.find(o => o.value.toLowerCase() === w);
  if (!hit) return { error: "no option " + JSON.stringify(want) + "; options: " + JSON.stringify(opts.slice(0, 40).map(o => o.label.trim() || o.value)) };
  if (hit.disabled || hit.matches(":disabled")) return { error: "that option is disabled" };
  // By index, not value: two options can share a value, and setting the
  // value would pick the first of them, not the one matched by its label.
  this.selectedIndex = opts.indexOf(hit);
  if (this.selectedOptions[0] !== hit) return { error: "the page didn't take that option" };
  this.dispatchEvent(new Event("input", { bubbles: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
}"#;

/// Does clicking this element commit something (submit a form, or a button
/// named like Send/Pay/Delete)? And what would the user be approving: the
/// form's non-secret values (spec §5.4).
const COMMIT_INFO: &str = r#"function () {
  const t = (this.tagName || "").toLowerCase();
  const type = (this.getAttribute("type") || "").toLowerCase();
  const form = this.form || (this.closest && this.closest("form"));
  const submits = !!form && ((t === "button" && (type === "" || type === "submit")) || (t === "input" && (type === "submit" || type === "image")));
  const name = (this.getAttribute("aria-label") || this.innerText || this.value || "").trim().replace(/\s+/g, " ").slice(0, 120);
  const verb = /\b(submit|send|pay|buy|purchase|order|checkout|check out|delete|remove|publish|post|confirm|sign up|sign|agree|accept|transfer|unsubscribe)\b/i.test(name);
  const fields = [];
  const secret = el => {
    const ty = (el.type || "").toLowerCase();
    const ac = (el.getAttribute("autocomplete") || "").toLowerCase();
    const nm = ((el.name || "") + " " + (el.id || "")).toLowerCase();
    return ty === "password" || /one-time-code|cc-|password/.test(ac) || /pass(word|code)?|otp|one.?time|cvc|cvv|card.?num|security.?code/.test(nm);
  };
  // A form's own properties can be shadowed by controls named after them
  // (<input name="action">, <input name="elements">): read them through the
  // prototype's getters, which the page can't shadow that way.
  const formGet = (prop) => Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, prop).get.call(form);
  let action = "";
  // Outside a form, a committing link's destination is its href (read
  // through the prototype, like the form's): shown in the banner and
  // checked again after approval.
  const link = !form && this.closest ? this.closest("a[href], area[href]") : null;
  if (link) {
    action = String(Object.getOwnPropertyDescriptor(link.tagName === "AREA" ? HTMLAreaElement.prototype : HTMLAnchorElement.prototype, "href").get.call(link));
  }
  if (form) {
    const own = (t === "button" || t === "input") && this.hasAttribute("formaction");
    action = String(own ? Object.getOwnPropertyDescriptor(t === "button" ? HTMLButtonElement.prototype : HTMLInputElement.prototype, "formAction").get.call(this) : formGet("action"));
    for (const el of Array.from(formGet("elements"))) {
      const ty = (el.type || "").toLowerCase();
      if (["hidden", "submit", "button", "image", "reset", "fieldset"].includes(ty) || el.tagName === "FIELDSET") continue;
      let v;
      if (secret(el)) v = "[secret]";
      else if (ty === "checkbox" || ty === "radio") { if (!el.checked) continue; v = el.value || "on"; }
      else if (ty === "file") v = Array.from(el.files || []).map(f => f.name).join(", ");
      else if (el.tagName === "SELECT") v = Array.from(el.selectedOptions).map(o => o.label.trim() || o.value).join(", ");
      else v = el.value;
      // A label's own text, without the text of controls nested in it (a
      // <select>'s options would otherwise read as part of its label).
      let labelText = "";
      if (el.labels && el.labels[0]) {
        const c = el.labels[0].cloneNode(true);
        c.querySelectorAll("select, input, textarea, button, option").forEach(n => n.remove());
        labelText = c.textContent.replace(/\s+/g, " ").trim();
      }
      const label = labelText || el.getAttribute("aria-label") || el.name || el.id || ty;
      fields.push([String(label).replace(/\s+/g, " ").slice(0, 80), String(v).slice(0, 200)]);
      if (fields.length >= 40) break;
    }
  }
  // Everything the click would send, hidden controls included (an amount or
  // a recipient id can live in one), for the check after approval. srv keeps
  // it out of the banner. Every control counts, however many there are (a
  // page could pad the form to push a changed one past a cutoff). Secret
  // values go back separately (`secrets`); the host folds all of it into one
  // keyed digest before anything leaves it. The element's own markup counts
  // too: a click handler can read its target from a data attribute.
  const sent = [];
  const secrets = [];
  if (form) {
    for (const el of Array.from(formGet("elements"))) {
      const ty = (el.type || "").toLowerCase();
      let v;
      if (secret(el)) { secrets.push(String(el.value)); v = "[secret]"; }
      else if (ty === "checkbox" || ty === "radio") v = el.checked ? (el.value || "on") : null;
      else if (ty === "file") v = Array.from(el.files || []).map(f => f.name + ":" + f.size).join(",");
      else if (el.tagName === "SELECT") v = Array.from(el.selectedOptions).map(o => o.value).join(",");
      else v = el.value;
      // name and id apart: only a named control is submitted, so adding a
      // name to an id-only one changes what is sent.
      sent.push([el.tagName, ty, el.getAttribute("name"), el.id || null, el.disabled ? 1 : 0, v == null ? null : String(v)]);
    }
  }
  const sub = ["formmethod", "formenctype", "formtarget", "name", "value"].map(a => this.getAttribute(a));
  const fingerprint = JSON.stringify([action, form ? [formGet("method"), formGet("enctype"), formGet("target")] : null, sub, this.outerHTML, link ? link.outerHTML : null, sent]);
  return { committing: submits || verb, name, action, fields, fingerprint, secrets };
}"#;

/// The element's state, for read-back after every action.
const READ_BACK: &str = r#"function () {
  const out = { tag: this.tagName.toLowerCase() };
  if (this.type) out.type = this.type;
  if (this.isContentEditable) { out.value = this.textContent; out.contentEditable = true; }
  else if ("value" in this && this.tagName !== "BUTTON") out.value = this.value;
  if (this.tagName === "SELECT") out.selected = Array.from(this.selectedOptions).map(o => o.label.trim() || o.value);
  if ("checked" in this && (this.type === "checkbox" || this.type === "radio")) out.checked = this.checked;
  const aria = this.getAttribute("aria-checked");
  if (aria !== null && out.checked === undefined) out.checked = aria === "true";
  if (this.validationMessage) out.validationMessage = this.validationMessage;
  const invalid = this.getAttribute("aria-invalid");
  if ((invalid && invalid !== "false") || (this.validity && !this.validity.valid)) out.invalid = true;
  return out;
}"#;

/// The focused element's DOM facts, for the secret-field guard on typing.
/// Descends through shadow roots and same-origin frames to the element that
/// really has focus. A focused frame it can't look into (cross-origin) comes
/// back as `uninspectable`: the guard fails closed on it.
const FOCUSED_FACTS: &str = r#"(() => {
  let doc = document;
  let el = doc.activeElement;
  for (let depth = 0; el && depth < 16; depth++) {
    if (el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el.tagName === "IFRAME" || el.tagName === "FRAME") {
      let inner = null;
      try { inner = el.contentDocument; } catch (e) { inner = null; }
      if (!inner) return { uninspectable: true, localName: el.localName, attributes: [] };
      doc = inner;
      el = doc.activeElement;
      continue;
    }
    break;
  }
  if (!el) return null;
  const attrs = [];
  for (const a of el.attributes) { attrs.push(a.name, a.value); }
  return { localName: el.localName, attributes: attrs };
})()"#;

// ── CDP steps ───────────────────────────────────────────────────────────────

async fn call(cdp: &mut CdpSession, method: &str, params: Value) -> Result<Value, String> {
    cdp.call(method, params).await.map_err(|e| format!("CDP {method}: {e}"))
}

/// A fresh isolated world in the pane's main frame.
async fn isolated_context(cdp: &mut CdpSession) -> Result<i64, String> {
    let tree = call(cdp, "Page.getFrameTree", json!({})).await?;
    let frame_id = tree
        .pointer("/frameTree/frame/id")
        .and_then(|v| v.as_str())
        .ok_or("CDP Page.getFrameTree: no main frame")?
        .to_string();
    let world = call(
        cdp,
        "Page.createIsolatedWorld",
        json!({ "frameId": frame_id, "worldName": "agentmux-agent" }),
    )
    .await?;
    world
        .get("executionContextId")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "CDP Page.createIsolatedWorld: no context id".to_string())
}

async fn resolve(cdp: &mut CdpSession, backend: i64, ctx: i64, r: &str) -> Result<String, String> {
    let v = cdp
        .call("DOM.resolveNode", json!({ "backendNodeId": backend, "executionContextId": ctx }))
        .await
        .map_err(|_| format!("the element for {r} is gone from the page; take a new snapshot"))?;
    v.pointer("/object/objectId")
        .and_then(|o| o.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("the element for {r} is gone from the page; take a new snapshot"))
}

async fn call_on(cdp: &mut CdpSession, object_id: &str, func: &str, args: Vec<Value>) -> Result<Value, String> {
    let v = call(
        cdp,
        "Runtime.callFunctionOn",
        json!({
            "objectId": object_id,
            "functionDeclaration": func,
            "arguments": args.into_iter().map(|a| json!({ "value": a })).collect::<Vec<_>>(),
            "returnByValue": true,
        }),
    )
    .await?;
    if let Some(exc) = v.get("exceptionDetails") {
        let msg = exc
            .pointer("/exception/description")
            .and_then(|d| d.as_str())
            .or_else(|| exc.get("text").and_then(|t| t.as_str()))
            .unwrap_or("unknown exception");
        return Err(format!("page script error: {msg}"));
    }
    let out = v.pointer("/result/value").cloned().unwrap_or(Value::Null);
    if let Some(e) = out.get("error").and_then(|e| e.as_str()) {
        return Err(e.to_string());
    }
    Ok(out)
}

async fn describe(cdp: &mut CdpSession, backend: i64) -> Option<DomFacts> {
    let v = cdp.call("DOM.describeNode", json!({ "backendNodeId": backend })).await.ok()?;
    v.get("node").map(DomFacts::from_described)
}

async fn page_url(cdp: &mut CdpSession) -> String {
    cdp.call("Runtime.evaluate", json!({ "expression": "location.href", "returnByValue": true }))
        .await
        .ok()
        .and_then(|v| v.pointer("/result/value").and_then(|u| u.as_str()).map(str::to_string))
        .unwrap_or_default()
}

async fn click_at(cdp: &mut CdpSession, x: f64, y: f64) -> Result<(), String> {
    for event_type in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let (button, buttons) = if event_type == "mouseMoved" { ("none", 0) } else { ("left", 1) };
        call(
            cdp,
            "Input.dispatchMouseEvent",
            json!({ "type": event_type, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": 1 }),
        )
        .await?;
    }
    Ok(())
}

async fn click_element(cdp: &mut CdpSession, object_id: &str) -> Result<(), String> {
    let p = call_on(cdp, object_id, CLICK_POINT, vec![]).await?;
    let x = p.get("x").and_then(|v| v.as_f64()).ok_or("no click point")?;
    let y = p.get("y").and_then(|v| v.as_f64()).ok_or("no click point")?;
    click_at(cdp, x, y).await
}

/// The secret-field guard for typing into whatever has focus (spec §5.1),
/// used by `dispatch_key`. `Err` names the refusal.
pub async fn refuse_typing_into_secret_field(cdp: &mut CdpSession) -> Result<(), String> {
    let ctx = isolated_context(cdp).await?;
    let v = call(
        cdp,
        "Runtime.evaluate",
        json!({ "expression": FOCUSED_FACTS, "contextId": ctx, "returnByValue": true }),
    )
    .await?;
    let raw = v.pointer("/result/value").filter(|x| !x.is_null());
    if raw.and_then(|r| r.get("uninspectable")).and_then(|u| u.as_bool()) == Some(true) {
        return Err("the focused field is inside a frame from another site, which AgentMux can't check: \
                    it could be a password or card field (payment forms often are), so typing there is refused. \
                    Ask the user to fill it in the pane."
            .to_string());
    }
    if raw.map(DomFacts::from_described).as_ref().is_some_and(snapshot::is_secret) {
        return Err(SECRET_REFUSAL.to_string());
    }
    Ok(())
}

const SECRET_REFUSAL: &str = "this is a password, one-time-code or card field: AgentMux never lets an agent type into one. \
     Ask the user to fill it in the pane themselves, then take a new snapshot.";

// ── Routes ──────────────────────────────────────────────────────────────────

fn unauthorized<T>() -> (StatusCode, Json<ApiResponse<T>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err("unauthorized: missing or invalid bearer token")))
}

fn fail<T>(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<T>>) {
    (StatusCode::OK, Json(ApiResponse::err(msg.into())))
}

/// Facts are looked up where the accessibility tree can't tell a secret field
/// or a file input apart from an ordinary one: value-bearing fields first
/// (a field without facts has its value hidden, `snapshot::render`), then
/// buttons (a file input's role).
const MAX_DESCRIBES: usize = 600;

/// `POST /agentmux/browser/snapshot`.
pub async fn snapshot_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SnapshotReq>,
) -> (StatusCode, Json<ApiResponse<SnapshotData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "snapshot").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let scope = match &req.scope {
        Some(r) => {
            let table = state.browser_api.ref_tables.get(&req.block_id).unwrap_or_default();
            let now = page_url(&mut cdp).await;
            match table.refs.get(r).copied() {
                Some(b) if table.url == now => Some(b),
                Some(_) => {
                    let _ = cdp.close().await;
                    return fail(format!("the page changed since {r} was handed out; take a snapshot without scope first"));
                }
                None => {
                    let _ = cdp.close().await;
                    return fail(format!("unknown reference {r:?}; take a snapshot without scope first"));
                }
            }
        }
        None => None,
    };
    let _ = cdp.call("Accessibility.enable", json!({})).await;
    let tree = match call(&mut cdp, "Accessibility.getFullAXTree", json!({})).await {
        Ok(t) => t,
        Err(e) => {
            let _ = cdp.close().await;
            return fail(e);
        }
    };
    let nodes: Vec<Value> = tree.get("nodes").and_then(|n| n.as_array()).cloned().unwrap_or_default();
    let mut facts = HashMap::new();
    let role_of = |n: &Value| n.pointer("/role/value").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // A scoped snapshot renders only its subtree, so it looks up facts only there.
    let in_scope: Vec<&Value> = match scope {
        Some(b) => snapshot::subtree(&nodes, b),
        None => nodes.iter().collect(),
    };
    let wanted = in_scope
        .iter()
        .copied()
        .filter(|n| snapshot::VALUE_ROLES.contains(&role_of(n).as_str()))
        .chain(in_scope.iter().copied().filter(|n| role_of(n) == "button"));
    for n in wanted {
        if facts.len() >= MAX_DESCRIBES {
            break;
        }
        if let Some(b) = n.get("backendDOMNodeId").and_then(|v| v.as_i64()) {
            if let Some(f) = describe(&mut cdp, b).await {
                facts.insert(b, f);
            }
        }
    }
    let url = page_url(&mut cdp).await;
    let prefix = match &req.scope {
        Some(r) => format!("{r}."),
        None => String::new(),
    };
    let rendered = snapshot::render(&nodes, &facts, scope, &prefix);
    // A scoped snapshot adds its references to the full snapshot's table, so
    // those stay usable; but never across a navigation.
    let mut table = match scope {
        Some(_) => state.browser_api.ref_tables.get(&req.block_id).unwrap_or_default(),
        None => RefTable::default(),
    };
    if table.url != url {
        table = RefTable::default();
    }
    table.url = url.clone();
    table.refs.extend(rendered.refs.iter().cloned());
    state.browser_api.ref_tables.put(&req.block_id, table);
    let _ = cdp.close().await;
    (
        StatusCode::OK,
        Json(ApiResponse::ok(SnapshotData {
            url,
            refs: rendered.refs.len(),
            truncated: rendered.truncated,
            snapshot: rendered.text,
        })),
    )
}
/// `POST /agentmux/browser/act`.
pub async fn act_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ActReq>,
) -> (StatusCode, Json<ApiResponse<ActData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let Some(table) = state.browser_api.ref_tables.get(&req.block_id) else {
        return fail("no snapshot of this pane yet: take one with BrowserSnapshot first");
    };
    let Some(&backend) = table.refs.get(&req.ref_) else {
        return fail(format!("unknown reference {:?}; take a new snapshot", req.ref_));
    };
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "act").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = act(&mut cdp, &table, backend, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(after) => (StatusCode::OK, Json(ApiResponse::ok(ActData { after }))),
        Err(e) => fail(e),
    }
}

async fn act(cdp: &mut CdpSession, table: &RefTable, backend: i64, req: &ActReq) -> Result<Value, String> {
    let now = page_url(cdp).await;
    if !table.url.is_empty() && now != table.url {
        return Err(format!("the page changed since the snapshot (now {now}); take a new snapshot"));
    }
    let facts = describe(cdp, backend).await.ok_or_else(|| {
        format!("the element for {} is gone from the page; take a new snapshot", req.ref_)
    })?;
    let ctx = isolated_context(cdp).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    let after = match req.action.as_str() {
        "click" => {
            if facts.is_file_input() {
                return Err("this is a file input: clicking it opens a file dialog only the user can use; upload with BrowserSetFiles".to_string());
            }
            // Fail closed: a click we can't classify (the page broke the
            // check, say by shadowing form.elements) isn't made.
            let unclassified = |e: String| {
                format!("couldn't tell whether this click submits something, so it wasn't made ({e}); ask the user to click it")
            };
            let info = call_on(cdp, &obj, COMMIT_INFO, vec![]).await.map_err(unclassified)?;
            let committing = info
                .get("committing")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| unclassified("no answer from the page".to_string()))?;
            let name = info.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let ask = json!({
                "what": if name.is_empty() { "Click a button that submits a form".to_string() } else { format!("Click \"{name}\"") },
                "page": now,
                "action": info.get("action").cloned().unwrap_or(Value::Null),
                "fields": info.get("fields").cloned().unwrap_or(json!([])),
                // Not shown: srv strips it from the banner and sends it back
                // with the approved click. One digest of the whole submission,
                // secrets included, keyed with a secret of this host process.
                "fingerprint": submission_digest(info.get("fingerprint"), info.get("secrets")),
            });
            if req.approved {
                // Approved: but the page's own script had the whole wait to
                // change the destination or the values. Click only if it
                // still shows what the user approved.
                if !committing || req.approved_as.as_ref() != Some(&ask) {
                    return Err("the form changed while the user was reading the approval, so nothing was clicked; \
                                call BrowserClick again to show them what it sends now"
                        .to_string());
                }
            } else if committing {
                return Ok(json!({ "needs_approval": ask }));
            }
            click_element(cdp, &obj).await?;
            Ok(call_on(cdp, &obj, READ_BACK, vec![]).await.unwrap_or(Value::Null))
        }
        "fill" => {
            if snapshot::is_secret(&facts) {
                return Err(SECRET_REFUSAL.to_string());
            }
            if facts.is_file_input() {
                return Err("this is a file input: it takes a file upload, not text".to_string());
            }
            let text = req.text.clone().ok_or("fill needs `text`")?;
            call_on(cdp, &obj, PREPARE_FILL, vec![]).await?;
            // Focusing ran the page's own handlers, which could have turned
            // the field into a secret one, or moved focus to one (insertText
            // types wherever focus is): check both before writing.
            if describe(cdp, backend).await.as_ref().is_some_and(snapshot::is_secret) {
                return Err(SECRET_REFUSAL.to_string());
            }
            refuse_typing_into_secret_field(cdp).await?;
            if text.is_empty() {
                call_on(cdp, &obj, SET_VALUE, vec![json!("")]).await?;
            } else {
                call(cdp, "Input.insertText", json!({ "text": text })).await?;
            }
            let mut after = call_on(cdp, &obj, READ_BACK, vec![]).await?;
            let got = after.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if got != text && after.get("contentEditable").is_none() {
                call_on(cdp, &obj, SET_VALUE, vec![json!(text)]).await?;
                after = call_on(cdp, &obj, READ_BACK, vec![]).await?;
            }
            Ok(after)
        }
        "select" => {
            let option = req.option.clone().ok_or("select needs `option`")?;
            call_on(cdp, &obj, SELECT_OPTION, vec![json!(option)]).await?;
            call_on(cdp, &obj, READ_BACK, vec![]).await
        }
        "check" => {
            let want = req.checked.ok_or("check needs `checked`")?;
            let before = call_on(cdp, &obj, READ_BACK, vec![]).await?;
            if before.get("checked").and_then(|v| v.as_bool()) == Some(want) {
                // Nothing was clicked: redact on the facts from before.
                return Ok(redact_secret(before, &facts));
            }
            click_element(cdp, &obj).await?;
            let after = call_on(cdp, &obj, READ_BACK, vec![]).await?;
            if after.get("checked").and_then(|v| v.as_bool()) != Some(want) {
                return Err(format!(
                    "clicked {}, but it is still {}checked; it may be controlled by another element (a label or a custom switch): take a new snapshot",
                    req.ref_,
                    if want { "not " } else { "" }
                ));
            }
            Ok(after)
        }
        other => Err(format!("unknown action {other:?}: use click, fill, select or check")),
    }?;
    // The page's own handlers may have turned the field into a secret one
    // during the action: decide on its facts from before and after.
    let facts_after = describe(cdp, backend).await;
    let after = redact_secret(after, &facts);
    Ok(match facts_after {
        Some(f) => redact_secret(after, &f),
        None => after,
    })
}

/// One digest of everything an approved click would send (the page's
/// `fingerprint` string and its secret values), so the check after approval
/// notices any change during the wait, however large the form, without the
/// values leaving this process. SipHash keyed with a random key made once per
/// host process: nobody without the key can reverse it or aim for a match
/// (the page gets no digest to compare against), and both checks of one
/// approval run in this process.
fn submission_digest(fingerprint: Option<&Value>, secrets: Option<&Value>) -> String {
    use std::hash::{BuildHasher, Hasher};
    static KEY: std::sync::OnceLock<std::collections::hash_map::RandomState> = std::sync::OnceLock::new();
    let mut h = KEY.get_or_init(Default::default).build_hasher();
    let mut put = |s: &str| {
        h.write_usize(s.len());
        h.write(s.as_bytes());
    };
    put(fingerprint.and_then(|v| v.as_str()).unwrap_or(""));
    for s in secrets.and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        put(s.as_str().unwrap_or(""));
    }
    format!("{:016x}", h.finish())
}

/// A secret field's value never reaches the agent, whatever the action
/// (spec §5.1): the read-back keeps the field's shape but not its value.
fn redact_secret(mut after: Value, facts: &DomFacts) -> Value {
    if snapshot::is_secret(facts) {
        if let Some(obj) = after.as_object_mut() {
            if obj.remove("value").is_some() {
                obj.insert("value".to_string(), Value::String("[secret: hidden]".to_string()));
            }
        }
    }
    after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(tag: &str, attrs: &[(&str, &str)]) -> DomFacts {
        DomFacts { local_name: tag.into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn any_change_to_the_submission_changes_the_digest_without_showing_it() {
        let d = |fp: &str, v: Value| submission_digest(Some(&json!(fp)), Some(&v));
        let a = d("form-a", json!(["hunter2", "4111111111111111"]));
        assert_eq!(a, d("form-a", json!(["hunter2", "4111111111111111"])), "stable within a process");
        assert_ne!(a, d("form-a", json!(["hunter3", "4111111111111111"])), "a swapped secret");
        assert_ne!(a, d("form-b", json!(["hunter2", "4111111111111111"])), "a changed control");
        // Boundaries count: ["ab","c"] isn't ["a","bc"].
        assert_ne!(d("", json!(["ab", "c"])), d("", json!(["a", "bc"])));
        assert!(!a.contains("hunter2"));
        // A huge form still yields a short digest: nothing is cut off.
        let big = "x".repeat(2_000_000);
        assert_ne!(d(&big, json!([])), d(&(big.clone() + "y"), json!([])));
        assert_eq!(d(&big, json!([])).len(), 16);
    }

    #[test]
    fn a_secret_fields_value_is_hidden_from_every_read_back() {
        let after = json!({ "tag": "input", "type": "password", "value": "hunter2" });
        let out = redact_secret(after, &facts("input", &[("type", "password")]));
        assert_eq!(out["value"], "[secret: hidden]");
        assert_eq!(out["type"], "password");
    }

    #[test]
    fn an_ordinary_fields_value_is_kept() {
        let after = json!({ "tag": "input", "value": "lark@example.com" });
        let out = redact_secret(after, &facts("input", &[("type", "email")]));
        assert_eq!(out["value"], "lark@example.com");
    }
}

// ── Uploads and waiting (spec §4.2, §5.3) ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetFilesReq {
    pub block_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    /// Absolute paths, already checked by srv against the agent's workspace.
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WaitForReq {
    pub block_id: String,
    /// Wait until the page's visible text contains this.
    #[serde(default)]
    pub text: Option<String>,
    /// Wait until the page URL contains this.
    #[serde(default)]
    pub url_contains: Option<String>,
    /// Wait until the element behind this reference has left the page.
    #[serde(default)]
    pub gone: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct WaitForData {
    pub waited_ms: u64,
    pub url: String,
}

/// The files a file input now holds, for read-back after an upload.
const FILES_READ_BACK: &str = r#"function () {
  return { tag: "input", type: "file", files: Array.from(this.files || []).map(f => ({ name: f.name, size: f.size })) };
}"#;

/// `POST /agentmux/browser/set_files`.
pub async fn set_files_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SetFilesReq>,
) -> (StatusCode, Json<ApiResponse<ActData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    if req.paths.is_empty() {
        return fail("no files given");
    }
    let Some(table) = state.browser_api.ref_tables.get(&req.block_id) else {
        return fail("no snapshot of this pane yet: take one with BrowserSnapshot first");
    };
    let Some(&backend) = table.refs.get(&req.ref_) else {
        return fail(format!("unknown reference {:?}; take a new snapshot", req.ref_));
    };
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "set_files").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = set_files(&mut cdp, &table, backend, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(after) => (StatusCode::OK, Json(ApiResponse::ok(ActData { after }))),
        Err(e) => fail(e),
    }
}

async fn set_files(cdp: &mut CdpSession, table: &RefTable, backend: i64, req: &SetFilesReq) -> Result<Value, String> {
    let now = page_url(cdp).await;
    if !table.url.is_empty() && now != table.url {
        return Err(format!("the page changed since the snapshot (now {now}); take a new snapshot"));
    }
    let facts = describe(cdp, backend)
        .await
        .ok_or_else(|| format!("the element for {} is gone from the page; take a new snapshot", req.ref_))?;
    if !facts.is_file_input() {
        return Err(format!("{} is not a file input; the snapshot names file inputs as \"file input\"", req.ref_));
    }
    if req.paths.len() > 1 && !facts.attrs.contains_key("multiple") {
        return Err("this file input takes one file".to_string());
    }
    call(cdp, "DOM.setFileInputFiles", json!({ "files": req.paths, "backendNodeId": backend })).await?;
    let ctx = isolated_context(cdp).await?;
    let obj = resolve(cdp, backend, ctx, &req.ref_).await?;
    call_on(cdp, &obj, FILES_READ_BACK, vec![]).await
}

/// `POST /agentmux/browser/wait_for`.
pub async fn wait_for_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WaitForReq>,
) -> (StatusCode, Json<ApiResponse<WaitForData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let conditions = [req.text.is_some(), req.url_contains.is_some(), req.gone.is_some()];
    if conditions.iter().filter(|c| **c).count() != 1 {
        return fail("give exactly one of `text`, `url_contains` or `gone`");
    }
    let gone_backend = match &req.gone {
        Some(r) => match state.browser_api.ref_tables.get(&req.block_id).and_then(|t| t.refs.get(r).copied()) {
            Some(b) => Some(b),
            None => return fail(format!("unknown reference {r:?}; take a new snapshot")),
        },
        None => None,
    };
    let timeout = std::time::Duration::from_millis(req.timeout_ms.unwrap_or(10_000).clamp(250, 60_000));
    let started = std::time::Instant::now();
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "wait_for").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    loop {
        let url = page_url(&mut cdp).await;
        let met = if let Some(t) = &req.text {
            let expr = format!(
                "!!(document.body && document.body.innerText.includes({}))",
                serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into())
            );
            cdp.call("Runtime.evaluate", json!({ "expression": expr, "returnByValue": true }))
                .await
                .ok()
                .and_then(|v| v.pointer("/result/value").and_then(|b| b.as_bool()))
                .unwrap_or(false)
        } else if let Some(u) = &req.url_contains {
            url.contains(u.as_str())
        } else if let Some(b) = gone_backend {
            describe(&mut cdp, b).await.is_none()
        } else {
            false
        };
        let waited_ms = started.elapsed().as_millis() as u64;
        if met {
            let _ = cdp.close().await;
            return (StatusCode::OK, Json(ApiResponse::ok(WaitForData { waited_ms, url })));
        }
        if started.elapsed() >= timeout {
            let _ = cdp.close().await;
            return fail(format!(
                "still waiting after {:.1} s (page now {url}); take a snapshot to see where it is",
                timeout.as_secs_f64()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
