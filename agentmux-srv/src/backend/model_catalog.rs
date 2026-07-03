// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Model catalog — authoritative Claude model list from the Anthropic Models
//! API (`GET /v1/models`), fetched with the agent's OAuth Bearer token.
//!
//! Why this exists: the agent-pane model dropdown was a hand-curated list whose
//! version labels ("Sonnet 4.6", …) drift every time Anthropic ships a model.
//! The CLI has no list-models surface, and prompting a model for the list is
//! hallucination-prone — so we read the authoritative catalog from
//! `GET /v1/models`. Verified 2026-07-02: a Claude **Max subscription** OAuth
//! token returns HTTP 200 (no API key, no `oauth-2025-04-20` beta header) with
//! the current models incl. `claude-sonnet-5` | "Claude Sonnet 5".
//!
//! This is a **metadata read only** — never call `/v1/messages` with this token
//! (that would cross the subscription-vs-API billing line). Access is treated as
//! best-effort: any failure (missing token on macOS Keychain, 401/expiry,
//! network) returns `None`, and the caller falls back to the bundled curated
//! catalog. See `docs/specs/SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md`.
//!
//! STATUS: fetch + parse layer (this file). Remaining wiring per the spec —
//! not yet implemented here:
//!   - cache the result per `(provider, cli_version)` under the data dir;
//!   - expose via a `providers.models` RPC (+ add `models` to Rust
//!     `ProviderConfig`, `backend/providers.rs`);
//!   - trigger a refresh on CLI install/upgrade (`install_handlers.rs` /
//!     `cli_handlers.rs`), plus an optional lazy ">30d & token valid" refetch
//!     modeled on `backend/cron/mod.rs` / `memory_heartbeat.rs`;
//!   - frontend: overlay `providers/index.ts` `models` with the cached catalog
//!     and converge the strip / `/model` dropdowns to read it.

use std::path::Path;

use crate::identity::key_validator::client;

/// One entry in the model dropdown. `id` is the concrete `--model` value the
/// CLI accepts (e.g. `claude-sonnet-5`); `display_name` is the label to show
/// (e.g. "Claude Sonnet 5"), taken verbatim from the API.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub display_name: String,
}

/// Read the Claude OAuth access token from an isolated auth dir's
/// `.credentials.json` (`claudeAiOauth.accessToken`). Returns `None` when the
/// file/token is absent — notably on **macOS**, where the CLI stores creds in
/// the Keychain rather than this file (`cli_handlers.rs:371-378`); the caller
/// falls back to the bundled catalog there. Windows/Linux keep the token here.
pub fn read_oauth_access_token(config_dir: &Path) -> Option<String> {
    let creds_path = config_dir.join(".credentials.json");
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let token = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|v| v.as_str())?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Map a `GET /v1/models` response body (`{ "data": [ { id, display_name }, … ] }`)
/// to our catalog. `display_name` falls back to `id` when absent. Extracted from
/// the network call so it is unit-testable without an outbound request.
fn parse_models(body: &serde_json::Value) -> Vec<CatalogModel> {
    body.get("data")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    let display_name = m
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(id.as_str())
                        .to_string();
                    Some(CatalogModel { id, display_name })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch the authoritative model catalog. Returns `None` (→ caller uses the
/// bundled fallback) on network error, non-2xx (incl. 401 token expiry), or an
/// empty/garbled body. Never logs the token.
pub async fn fetch_model_catalog(access_token: &str) -> Option<Vec<CatalogModel>> {
    let resp = match client()
        .get("https://api.anthropic.com/v1/models")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("model catalog: network error: {e}");
            return None;
        }
    };
    if !resp.status().is_success() {
        // 401 here means the OAuth token expired — keep last-good cache and
        // refetch on the next auth/install event (spec §3 caveats).
        tracing::warn!("model catalog: HTTP {} from /v1/models", resp.status());
        return None;
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("model catalog: bad JSON: {e}");
            return None;
        }
    };
    let models = parse_models(&body);
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_models_and_falls_back_display_name_to_id() {
        let body = serde_json::json!({
            "data": [
                { "id": "claude-sonnet-5", "display_name": "Claude Sonnet 5" },
                { "id": "claude-opus-4-8" } // no display_name → falls back to id
            ]
        });
        let got = parse_models(&body);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], CatalogModel { id: "claude-sonnet-5".into(), display_name: "Claude Sonnet 5".into() });
        assert_eq!(got[1].display_name, "claude-opus-4-8");
    }

    #[test]
    fn empty_when_data_missing_or_wrong_shape() {
        assert!(parse_models(&serde_json::json!({})).is_empty());
        assert!(parse_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }

    #[test]
    fn skips_entries_without_id() {
        let body = serde_json::json!({ "data": [ { "display_name": "no id here" }, { "id": "claude-haiku-4-5" } ] });
        let got = parse_models(&body);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "claude-haiku-4-5");
    }
}
