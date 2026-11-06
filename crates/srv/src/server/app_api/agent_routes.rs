// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents routes` (`POST /api/v1/agent/routes`): the model routes a caller may choose, one row per route with its
//! models, prices and whether its key is in the Vault. Read-only, and open to every agent, container agents included
//! (`docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md` section 5.1).

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::backend::model_gateway::ask;
use crate::backend::vault::route_key;
use crate::server::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RoutesRequest {
    /// Only the routes that serve this harness; absent, all of them.
    #[serde(default)]
    harness: Option<String>,
}

pub(crate) async fn handle_agent_routes(State(state): State<AppState>, Json(req): Json<RoutesRequest>) -> Response {
    let harness = req.harness.as_deref().map(str::trim).filter(|h| !h.is_empty());
    let listing = ask::listing(harness, |route| route_key::present(&state.identity_store, route.key_label));
    Json(listing).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    async fn listed(state: &AppState, body: serde_json::Value) -> serde_json::Value {
        let resp = handle_agent_routes(State(state.clone()), Json(serde_json::from_value(body).unwrap())).await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn the_listing_says_whether_the_key_is_in_the_vault_and_filters_by_harness() {
        let state = test_state();
        crate::backend::vault::seal::use_test_key();
        assert_eq!(listed(&state, serde_json::json!({})).await["routes"][0]["key_present"], false);
        let entry: crate::backend::vault::NewEntry = serde_json::from_value(serde_json::json!({ "kind": "api-key", "label": "OpenRouter API key", "parts": { "value": "sk-or-test" } })).unwrap();
        crate::backend::vault::create(&state.identity_store, entry).unwrap();
        let all = listed(&state, serde_json::json!({})).await;
        assert_eq!(all["routes"][0]["key_present"], true);
        assert!(!serde_json::to_string(&all).unwrap().contains("sk-or-test"));
        assert!(listed(&state, serde_json::json!({ "harness": "codex" })).await["routes"].as_array().unwrap().is_empty());
    }
}
