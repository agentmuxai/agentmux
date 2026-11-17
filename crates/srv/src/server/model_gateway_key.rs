// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The gateway's key route: an agent's own Anthropic API key, an account of Connectors > Accounts.
//!
//! The agent is pointed at `/api/v1/model/anthropic/...` with a token that names it and the account
//! (`token::token_for_account`). Each request is checked against the agent's *current* link to that account, so a
//! switch or an unlink ends the token's use at once, and the key is read from the keychain here, for the one
//! request, and sent as `x-api-key`. The agent's environment, files and processes never hold it.

use super::*;
use crate::backend::model_gateway::token::Grant;
use axum::body::Body;
use axum::http::{header, HeaderMap, StatusCode};
use serde_json::Value;
use zeroize::Zeroizing;

/// The key to send upstream, and the name the person gave its account (for the messages that name it).
pub(super) struct KeyAccount {
    pub key: Zeroizing<String>,
    pub name: String,
}

fn label_of(account: &crate::backend::storage::identities::IdentityAccount) -> String {
    [&account.display_name, &account.name].into_iter().find(|n| !n.trim().is_empty()).cloned().unwrap_or_default()
}

/// The key of the account `grant` spends, when the agent is still linked to it. BLOCKING (store and keychain reads).
fn read(state: &AppState, grant: &Grant) -> Result<KeyAccount, (StatusCode, &'static str, String)> {
    let gone = || (StatusCode::FORBIDDEN, "permission_error", "this agent is no longer on that API key account".to_string());
    let links = state.identity_store.agent_identity_list_for_agent(&grant.def_id).map_err(|_| gone())?;
    let linked = links.iter().any(|l| l.account_id == grant.account && l.provider.eq_ignore_ascii_case(crate::backend::model_gateway::catalog::KEY_ROUTE_ID));
    if grant.account.is_empty() || !linked {
        return Err(gone());
    }
    let account = match crate::identity::resolver::inject::resolve_account_for_spawn(&state.id_store, &grant.account) {
        Ok(Some((a, _))) if a.kind == "api_key" => a,
        _ => return Err(gone()),
    };
    let name = label_of(&account);
    match crate::identity::resolver::secret::resolve_account_secret(&account, &state.id_store, &state.identity_store) {
        Ok(value) => Ok(KeyAccount { key: Zeroizing::new(value), name }),
        Err(_) => Err((StatusCode::SERVICE_UNAVAILABLE, "api_error", format!("the API key account \"{name}\" has no usable key; add it again in Connectors > Accounts"))),
    }
}

/// [`read`] off the async runtime.
pub(super) async fn key_of(state: &AppState, grant: &Grant) -> Result<KeyAccount, Response> {
    let (state, grant) = (state.clone(), grant.clone());
    match tokio::task::spawn_blocking(move || read(&state, &grant)).await {
        Ok(Ok(key)) => Ok(key),
        Ok(Err((status, kind, why))) => Err(super::model_gateway_handlers::refuse(status, kind, &why)),
        Err(_) => Err(super::model_gateway_handlers::refuse(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "the key lookup failed")),
    }
}

/// The model a request names, for counting its answer at that family's price.
pub(super) fn requested_model(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string)).unwrap_or_default()
}

/// An error answer from Anthropic (a rejected key, a used-up credit), with the account's name added to its message, so
/// the failure the agent shows names the account and nothing of the key. Other answers are returned as they are.
/// `Err` holds the finished answer, `Ok` the response to relay as it is.
pub(super) async fn named_error(response: reqwest::Response, name: &str) -> Result<reqwest::Response, Response> {
    let streaming = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream"));
    if !response.status().is_client_error() || streaming || name.is_empty() {
        return Ok(response);
    }
    let status = response.status();
    let mut headers = HeaderMap::new();
    for (n, v) in response.headers() {
        if ![header::CONTENT_LENGTH, header::TRANSFER_ENCODING, header::CONTENT_ENCODING, header::CONNECTION].contains(n) {
            headers.append(n.clone(), v.clone());
        }
    }
    let bytes = response.bytes().await.unwrap_or_default();
    let named = match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut json) => {
            if let Some(message) = json.pointer_mut("/error/message") {
                if let Some(text) = message.as_str() {
                    *message = Value::String(format!("{text} (AgentMux API key account \"{name}\")"));
                }
            }
            serde_json::to_vec(&json).unwrap_or_else(|_| bytes.to_vec())
        }
        Err(_) => bytes.to_vec(),
    };
    let mut out = Response::new(Body::from(named));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    Err(out)
}
