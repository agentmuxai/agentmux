// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/agents/self/keys` — identity M4d-3
//! (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10).
//!
//! An agent's MCP fetches its own signing keys here instead of reading them
//! from `.mcp.json`, which goes stale: the host jekt key rotates after 24 h
//! (`JEKT_KEY_TTL_SECS`) while a running agent keeps the copy it was launched
//! with. Served only to `Caller::Agent(uid)`, the agent its `X-Agent-Token`
//! names: these are private keys, so the 401 is this endpoint's own.
//!
//! The name-keyed jekt, LAN and WAN keys are those of the row's slug only,
//! ensured as injection ensures them (so the jekt key rotates here), and the
//! response says which slug they belong to: the MCP uses them only when that
//! slug is exactly its own `AGENTMUX_AGENT_ID`. The UID-keyed LAN and WAN keys
//! (M4d-2) ride along for M4d-6's v2 signatures. Nothing here is logged.

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Serialize;

use super::caller::Caller;
use super::AppState;

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct SelfKeys {
    /// The row's slug: the only name these name-keyed keys are for.
    pub slug: String,
    /// Host-tier HMAC key, base64.
    pub jekt_key: Option<String>,
    /// LAN (and cross-channel) Ed25519 private key, base64.
    pub lan_key: Option<String>,
    /// WAN Ed25519 private key, base64.
    pub wan_key: Option<String>,
    pub uid: String,
    /// The UID-keyed LAN and WAN private keys (M4d-2), base64.
    pub uid_lan_key: Option<String>,
    pub uid_wan_key: Option<String>,
}

/// The keys for `uid`, or `None` when it has no row.
pub(crate) fn self_keys_for(mstore: &crate::backend::storage::store::Store, uid: &str) -> Option<SelfKeys> {
    let def = mstore.agent_def_get(uid).ok().flatten()?;
    let slug = def.slug.trim().to_string();
    let named = !slug.is_empty();
    let jekt_key = named.then(|| mstore.agent_jekt_key_ensure(&slug).ok()).flatten().map(|k| BASE64.encode(k));
    let lan_key = named.then(|| mstore.agent_lan_key_ensure(&slug).ok()).flatten().map(|k| k.private_key);
    let wan_key = named.then(|| mstore.agent_wan_key_ensure(&slug).ok()).flatten().map(|k| k.private_key);
    let uid_keys = mstore.agent_uid_keys_ensure(uid).ok().flatten().map(|(keys, _)| keys);
    Some(SelfKeys {
        slug,
        jekt_key,
        lan_key,
        wan_key,
        uid: uid.to_string(),
        uid_lan_key: uid_keys.as_ref().map(|k| k.lan.private_key.clone()),
        uid_wan_key: uid_keys.map(|k| k.wan.private_key),
    })
}

pub(crate) async fn handle_agent_self_keys(State(state): State<AppState>, caller: Option<axum::Extension<Caller>>) -> Response {
    let Some(uid) = caller.as_deref().and_then(Caller::uid).map(str::to_string) else {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "agents/self/keys: this request carries no agent identity (X-Agent-Token)" }))).into_response();
    };
    let mstore = state.mstore.clone();
    let keys = tokio::task::spawn_blocking(move || self_keys_for(&mstore, &uid)).await.ok().flatten();
    match keys {
        Some(keys) => ([(header::CACHE_CONTROL, "no-store")], Json(keys)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "agents/self/keys: no agent row for this token" }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;
    use crate::backend::storage::store::Store;

    fn store_with(uid: &str, slug: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut def = test_agent_def(uid, slug, "claude", "agent", agentmux_common::time::now_ms(), "");
        def.slug = slug.to_string();
        store.agent_def_insert(&mut def).unwrap();
        store
    }

    #[test]
    fn serves_the_slug_s_keys_the_same_ones_injection_writes() {
        let store = store_with("uid-aria", "aria");
        let keys = self_keys_for(&store, "uid-aria").unwrap();
        assert_eq!(keys.slug, "aria");
        assert_eq!(keys.jekt_key, Some(BASE64.encode(store.agent_jekt_key_ensure("aria").unwrap())));
        assert_eq!(keys.lan_key, Some(store.agent_lan_key_ensure("aria").unwrap().private_key));
        assert_eq!(keys.wan_key, Some(store.agent_wan_key_ensure("aria").unwrap().private_key));
        assert!(keys.uid_lan_key.is_some() && keys.uid_wan_key.is_some());
        assert_eq!(self_keys_for(&store, "uid-aria").unwrap(), keys, "stable across fetches");
    }

    #[test]
    fn an_unknown_uid_gets_nothing() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(self_keys_for(&store, "uid-nobody"), None);
    }
}
