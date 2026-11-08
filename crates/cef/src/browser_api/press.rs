// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Enter and Space as the click they are
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §5).
//!
//! `press` never sends the key. It reads what has focus, names what the key
//! would do there (`scripts/press_effect.js`: a form's default button, a
//! button or link, a checkbox toggle) and makes that a click through `act`'s
//! own check: a click that submits or whose name says Pay or Delete is handed
//! back to srv to ask about, and an approved one is made only if the form still
//! shows what the person saw. An effect it cannot name is refused. Whether the
//! agent may press at all is srv's decision (the grant's Enter and Space
//! power); this module re-checks the sites and does what it is asked.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::act::{backend_of, call, call_on, check_click, click_element, describe, fail, insert_text, isolated_context, page_now, page_url, redact_secret, refuse_typing_into_secret_field, unauthorized, ClickCheck, READ_BACK};
use super::snapshot::DomFacts;
use super::types::ApiResponse;
use crate::state::AppState;

const EFFECT_JS: &str = include_str!("scripts/press_effect.js");

#[derive(Debug, Deserialize)]
pub struct PressReq {
    pub block_id: String,
    /// `Enter` or `Space`.
    pub key: String,
    /// The sites the person's grant is bound to; the page must still be on one.
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub approved_as: Option<Value>,
}

/// What `press_effect.js` answered: its `effect` and `why`, and the object ids of
/// the element to click and of the one that has focus.
struct Resolved {
    effect: String,
    why: String,
    target: Option<String>,
    focused: Option<String>,
}

async fn resolve(cdp: &mut super::cdp::CdpSession, key: &str) -> Result<Resolved, String> {
    let ctx = isolated_context(cdp, None).await?;
    let reply = call(
        cdp,
        "Runtime.evaluate",
        json!({ "expression": format!("({EFFECT_JS})(document, {})", json!(key)), "contextId": ctx, "returnByValue": false }),
    )
    .await?;
    if reply.get("exceptionDetails").is_some() {
        return Err("couldn't read what has focus on the page".to_string());
    }
    let id = reply.pointer("/result/objectId").and_then(|o| o.as_str()).ok_or("couldn't read what has focus on the page")?;
    let props = call(cdp, "Runtime.getProperties", json!({ "objectId": id, "ownProperties": true })).await?;
    let prop = |name: &str| props.get("result").and_then(|r| r.as_array()).and_then(|r| r.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))).and_then(|p| p.get("value"));
    let text = |name: &str| prop(name).and_then(|v| v.get("value")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let object = |name: &str| prop(name).and_then(|v| v.get("objectId")).and_then(|v| v.as_str()).map(str::to_string);
    Ok(Resolved { effect: text("effect"), why: text("why"), target: object("el"), focused: object("focused") })
}

/// Does the element listen for keys itself? Then the key's effect is the page's
/// own, and not one to name. Asked of the node in the page's own world: the
/// isolated world's view of it holds only the listeners AgentMux added.
async fn handles_keys(cdp: &mut super::cdp::CdpSession, object_id: &str) -> bool {
    let Some(backend) = backend_of(cdp, object_id).await else { return true };
    let Ok(node) = cdp.call("DOM.resolveNode", json!({ "backendNodeId": backend })).await else { return true };
    let Some(page_object) = node.pointer("/object/objectId").and_then(|o| o.as_str()) else { return true };
    let Ok(v) = cdp.call("DOMDebugger.getEventListeners", json!({ "objectId": page_object })).await else { return true };
    v.get("listeners")
        .and_then(|l| l.as_array())
        .is_none_or(|l| l.iter().any(|x| matches!(x.get("type").and_then(|t| t.as_str()), Some("keydown" | "keypress" | "keyup"))))
}

/// The element's facts, for hiding what a field holds.
async fn facts_of(cdp: &mut super::cdp::CdpSession, object_id: &str) -> Option<DomFacts> {
    let backend = backend_of(cdp, object_id).await?;
    describe(cdp, backend).await
}

/// The element's state after the key, as `act` gives it: a secret field's value and any card, ID or
/// token number hidden, on the facts from before and after (the page's handlers may have changed the field).
async fn read_back(cdp: &mut super::cdp::CdpSession, object_id: &str, before: &DomFacts) -> Value {
    let after = redact_secret(call_on(cdp, object_id, READ_BACK, vec![]).await.unwrap_or(Value::Null), before);
    match facts_of(cdp, object_id).await {
        Some(now) => redact_secret(after, &now),
        None => after,
    }
}

fn refused(key: &str, why: &str) -> String {
    format!("{key} was not pressed: {why}. The host only presses a key whose click it can name; use Browser's click action on the control (a snapshot has its reference)")
}

/// `POST /agentmux/browser/press` — what `Enter` or `Space` would do where the
/// focus is, made as a click. `after` is the clicked element, or a
/// `needs_approval` when the click commits something; `resolved` says what the
/// key was taken to mean. A space in a text field is typed here, after the
/// secret-field guard.
pub async fn press_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<PressReq>) -> (axum::http::StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    if req.key != "Enter" && req.key != "Space" {
        return fail("press takes Enter or Space");
    }
    let mut root = match super::routes::open_dedicated_pane(&state, &req.block_id, "press").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let done = press(&mut root, &req).await;
    let _ = root.close().await;
    match done {
        Ok(v) => (axum::http::StatusCode::OK, Json(ApiResponse::ok(v))),
        Err(e) => fail(e),
    }
}

async fn press(root: &mut super::cdp::CdpSession, req: &PressReq) -> Result<Value, String> {
    let now = page_url(root).await;
    if !agentmux_common::allowed_origins::allows(&req.allowed_origins, &now) {
        return Err(format!("{} was not pressed: the page is off the sites the person granted", req.key));
    }
    let found = resolve(root, &req.key).await?;
    if found.effect == "type" {
        // An approved retry presents a click: if the focus has moved to a text field since, that click is not
        // the one the person saw, and a space typed there is not what they approved.
        if req.approved {
            return Err(format!("{} was not pressed: the focus moved while the user was reading the approval; press it again", req.key));
        }
        // A space in a text field, typed here and not by a second call: text goes where the focus is and
        // cannot press a button the page moved focus to, which a key event would.
        refuse_typing_into_secret_field(root).await?;
        let before = match found.focused.as_deref() {
            Some(focused) => facts_of(root, focused).await,
            None => None,
        };
        insert_text(root, " ").await?;
        // Without the field's facts nothing is shown of it: it could hold a secret.
        let after = match (found.focused.as_deref(), before) {
            (Some(focused), Some(before)) => read_back(root, focused, &before).await,
            _ => Value::Null,
        };
        return Ok(json!({ "after": after, "resolved": found.why }));
    }
    let (Some(target), "click") = (found.target.as_deref(), found.effect.as_str()) else { return Err(refused(&req.key, &found.why)) };
    if let Some(focused) = found.focused.as_deref() {
        if handles_keys(root, focused).await {
            return Err(refused(&req.key, "the page listens for keys on this element, so the key's effect is the page's own"));
        }
    }
    let backend = backend_of(root, target).await.ok_or("the control is gone from the page")?;
    let facts = describe(root, backend).await.ok_or("the control is gone from the page")?;
    match check_click(root, target, &facts, "", &now, req.approved_as.as_ref(), req.approved).await? {
        ClickCheck::Ask(ask) => return Ok(json!({ "after": { "needs_approval": ask }, "resolved": found.why })),
        ClickCheck::Click => {}
    }
    let mut inner = root.in_session(None);
    click_element(&mut inner, root, None, target).await?;
    let after = read_back(&mut inner, target, &facts).await;
    let page = page_now(root, &now).await;
    Ok(json!({ "after": after, "page": page, "resolved": found.why }))
}
