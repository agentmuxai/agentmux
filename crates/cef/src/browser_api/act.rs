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
  if (this.disabled) return { error: "the field is disabled" };
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
  const w = String(want).trim().toLowerCase();
  const opts = Array.from(this.options);
  const hit = opts.find(o => o.label.trim().toLowerCase() === w || o.text.trim().toLowerCase() === w)
    || opts.find(o => o.value.toLowerCase() === w);
  if (!hit) return { error: "no option " + JSON.stringify(want) + "; options: " + JSON.stringify(opts.slice(0, 40).map(o => o.label.trim() || o.value)) };
  if (hit.disabled) return { error: "that option is disabled" };
  this.value = hit.value;
  this.dispatchEvent(new Event("input", { bubbles: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
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
const FOCUSED_FACTS: &str = r#"(() => {
  let el = document.activeElement;
  while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
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
    let facts = v.pointer("/result/value").filter(|x| !x.is_null()).map(DomFacts::from_described);
    if facts.as_ref().is_some_and(snapshot::is_secret) {
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
        Err(e) => return fail(e),
    };
    let nodes: Vec<Value> = tree.get("nodes").and_then(|n| n.as_array()).cloned().unwrap_or_default();
    let mut facts = HashMap::new();
    let role_of = |n: &Value| n.pointer("/role/value").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let wanted = nodes
        .iter()
        .filter(|n| snapshot::VALUE_ROLES.contains(&role_of(n).as_str()))
        .chain(nodes.iter().filter(|n| role_of(n) == "button"));
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
    match req.action.as_str() {
        "click" => {
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
                return Ok(before);
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
    }
}
