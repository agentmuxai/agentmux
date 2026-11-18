// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the host typed from the Vault into a pane's page, and keeping it out of every answer an agent gets
//! (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md, G4).
//!
//! The matching is `agentmux_common::typed_secrets` (digests, no value kept). This file is the part that needs the
//! pane: which document it is showing (the main frame's `loaderId`), noting a value as it is typed, answering
//! every browser route's reply with the typed values left out, and the boxes of a screenshot.

use std::sync::Arc;

use agentmux_common::typed_secrets::TypedSecrets;
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{json, Value};

use super::cdp::CdpSession;
use crate::state::AppState;

/// The most a request or an answer may be, for the check below to read it.
const MAX_BODY: usize = 32 * 1024 * 1024;

/// The key of this process's digests: random, made now, kept nowhere else.
pub fn new_registry() -> TypedSecrets {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    TypedSecrets::new(key)
}

/// The document the pane's main frame shows now: its `loaderId`, which a navigation or a reload changes. `None` when
/// it can't be read, which a check counts as the document the values were typed in.
pub async fn current_doc(cdp: &mut CdpSession) -> Option<String> {
    let tree = cdp.call("Page.getFrameTree", json!({})).await.ok()?;
    tree.pointer("/frameTree/frame/loaderId").and_then(Value::as_str).map(str::to_string)
}

/// `value` was typed into the page of pane `block`: from now on no answer to an agent holds it.
pub async fn note_typed(state: &Arc<AppState>, cdp: &mut CdpSession, block: &str, value: &str) {
    let doc = current_doc(cdp).await.unwrap_or_else(|| "unknown".to_string());
    state.browser_api.typed.note(block, &doc, value);
}

/// Whether a Vault value was typed into the document pane `block` shows now.
pub async fn typed_into_now(state: &Arc<AppState>, cdp: &mut CdpSession, block: &str) -> bool {
    if !state.browser_api.typed.typed_into(block, None) {
        return false;
    }
    let doc = current_doc(cdp).await;
    state.browser_api.typed.typed_into(block, doc.as_deref())
}

/// What a scan of a pane's fields and text checks them against.
pub struct TypedFilter<'a> {
    secrets: &'a TypedSecrets,
    block: &'a str,
    doc: Option<String>,
}

impl<'a> TypedFilter<'a> {
    /// The filter for pane `block`; `None` when nothing was typed into the document it shows.
    pub async fn of(state: &'a Arc<AppState>, cdp: &mut CdpSession, block: &'a str) -> Option<Self> {
        if !typed_into_now(state, cdp, block).await {
            return None;
        }
        Some(Self { secrets: &state.browser_api.typed, block, doc: current_doc(cdp).await })
    }

    /// Whether `text` holds a value typed from the Vault.
    pub fn hides(&self, text: &str) -> bool {
        self.secrets.withhold(self.block, self.doc.as_deref(), text).is_some()
    }
}

/// Middleware over the browser routes: an answer for a pane that has a Vault value typed into its document leaves
/// that value out, wherever the page shows it (a field whose type was switched and whose mark was taken off, a text
/// it copied it to). A pane with nothing typed into it costs one lookup.
pub async fn withhold_typed(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Response::builder().status(413).body(Body::empty()).unwrap_or_default();
    };
    let block = serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| v.get("block_id").and_then(Value::as_str).map(str::to_string));
    let response = next.run(Request::from_parts(parts, Body::from(bytes))).await;
    let Some(block) = block.filter(|b| state.browser_api.typed.typed_into(b, None)) else { return response };
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Response::builder().status(500).body(Body::empty()).unwrap_or_default();
    };
    let Ok(mut answer) = serde_json::from_slice::<Value>(&bytes) else { return Response::from_parts(parts, Body::from(bytes)) };
    let doc = match state.browser_api.target_cache.resolve(&state, &block).await {
        Ok(resolved) => current_doc(&mut CdpSession::attach(&state, &resolved.label)).await,
        Err(_) => None,
    };
    if state.browser_api.typed.withhold_json(&block, doc.as_deref(), &mut answer) == 0 {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let bytes = serde_json::to_vec(&answer).unwrap_or_default();
    parts.headers.remove(CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(bytes))
}
