// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC: `providers.models` — authoritative model catalog for a provider,
//! fetched from the Anthropic Models API with the account-global OAuth token.
//!
//! Best-effort by design: any missing token (e.g. macOS Keychain), expiry, or
//! network failure returns an empty `models` list, and the frontend keeps its
//! bundled static catalog. Only Claude has an authoritative Models API today;
//! other providers return empty. See `backend/model_catalog.rs` and
//! `docs/specs/SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md`.

use std::sync::Arc;

use crate::backend::model_catalog::{fetch_model_catalog, resolve_access_token, CatalogModel};
use crate::backend::providers::get_provider;
use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

#[derive(serde::Deserialize)]
struct ProvidersModelsParams {
    provider_id: String,
}

#[derive(serde::Serialize)]
struct ProvidersModelsResult {
    /// `CatalogModel` serializes to `{ id, display_name }`.
    models: Vec<CatalogModel>,
}

fn empty_result() -> Result<Option<serde_json::Value>, String> {
    serde_json::to_value(ProvidersModelsResult { models: vec![] })
        .map(Some)
        .map_err(|e| format!("serialize: {e}"))
}

pub fn register_providers_handlers(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    // providers.models → authoritative model list for a provider (Claude only
    // today). Reads the account-global OAuth token and hits GET /v1/models.
    engine.register_handler(
        "providers.models",
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let params: ProvidersModelsParams = serde_json::from_value(data)
                    .map_err(|e| format!("providers.models: {e}"))?;

                // Only Claude exposes an authoritative Models API; others fall
                // back to the frontend's static catalog.
                let provider = match get_provider(&params.provider_id) {
                    Some(p) => p,
                    None => return Err(format!("unknown provider: {}", params.provider_id)),
                };
                if provider.id != "claude" {
                    return empty_result();
                }

                // Account-GLOBAL shared creds dir (version/channel-independent).
                // If data paths can't be resolved (CI / unusual env), stay
                // best-effort per the module contract: empty list → FE keeps its
                // static catalog, rather than erroring the RPC.
                let paths = match agentmux_common::DataPaths::from_env() {
                    Some(p) => p,
                    None => return empty_result(),
                };
                let dir = paths.provider_auth_dir(provider.auth_dir_name);
                // `allow_keychain_fallback: false` — this RPC is fired
                // automatically, unprompted, on every app launch
                // (`frontend/app-init.ts`'s init wave). Persisting or reading
                // back a stored fallback token (model_catalog.rs's steps 2/3)
                // can trigger an interactive macOS Keychain password prompt;
                // a background model-label refresh must never surface one.
                // The `CLAUDE_CODE_OAUTH_TOKEN` env var itself is still
                // honored either way (a plain process-env read, no keychain
                // involved) — only the keychain-touching persist/read-back
                // is skipped, so macOS falls back to its static bundled
                // catalog only when that env var isn't set for this process.
                let token = match resolve_access_token(&dir, false).await {
                    Some(t) => t,
                    // No token from the `.credentials.json` file (logged
                    // out, or macOS Keychain — the only source this
                    // background call is allowed to use) → empty → frontend
                    // keeps its static fallback.
                    None => return empty_result(),
                };

                let models = fetch_model_catalog(&token).await.unwrap_or_default();
                serde_json::to_value(ProvidersModelsResult { models })
                    .map(Some)
                    .map_err(|e| format!("serialize: {e}"))
            })
        }),
    );
}
