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
use crate::identity::secret_store;

/// Fixed, sentinel account id for the user-supplied long-lived
/// `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`), persisted via
/// `identity::secret_store::put`/`get`. Those wrap it in `account_key()`
/// (`"acct:{account_id}"`), so the actual keychain entry is
/// `"acct:system:claude-code-oauth-token"` — this constant only needs to be
/// distinct as an *account id* from the per-identity Armory account ids used
/// elsewhere (see `identity::resolver`, `identity::oauth_client`), which are
/// account UUIDs and would never collide with this sentinel string. This is a
/// single, system-wide fallback token, not tied to any AgentMux identity.
const CLAUDE_OAUTH_TOKEN_ACCOUNT: &str = "system:claude-code-oauth-token";

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

/// Resolve the Claude OAuth access token to use for the model-catalog fetch,
/// trying multiple sources in order before giving up (→ caller falls back to
/// the bundled curated catalog, unchanged from before this existed). Order:
///
///   1. [`read_oauth_access_token`] — the `.credentials.json` file written by
///      the Claude Code CLI. Covers Linux/Windows with zero behavior change.
///   2. The `CLAUDE_CODE_OAUTH_TOKEN` env var — the long-lived token a user
///      gets by running `claude setup-token` themselves in a real terminal
///      (Anthropic's documented recipe for headless/CI use of Claude Code)
///      and exporting before launching AgentMux. This is how macOS — where
///      Claude Code keeps its token in the Keychain, not a plain file — gets
///      a token at all. When found, it is persisted into AgentMux's own
///      OS-keychain-backed secret store (`identity::secret_store`) under
///      [`CLAUDE_OAUTH_TOKEN_ACCOUNT`] so a later run still has it even
///      without the env var set (e.g. the app relaunched from the Dock
///      rather than from the terminal that had it exported).
///   3. AgentMux's own secret store — a token persisted by a previous run of
///      step 2.
///
/// Callers should use this instead of calling [`read_oauth_access_token`]
/// directly. Never logs the token.
///
/// `allow_keychain_fallback` gates steps 2/3 — both hit the OS keychain (via
/// `identity::secret_store`), which wraps blocking `keyring` syscalls that
/// can trigger an interactive macOS "App wants to use your confidential
/// information" password prompt whenever a previously-stored entry exists
/// under a code signature the OS doesn't already trust (e.g. a rebuilt local
/// dev binary). That's fine for a flow the user explicitly triggered, but
/// this function's only caller today (`providers.models`) is a silent,
/// fire-and-forget background refresh fired on every app launch
/// (`frontend/app-init.ts`) — a keychain prompt appearing unprompted at
/// startup, for a purely cosmetic model-label refresh, is not acceptable.
/// Pass `false` from any automatic/background caller; steps 2/3 (and the
/// `spawn_blocking` they'd run on) are skipped entirely in that case, so
/// there is no keychain interaction at all — matching the pre-existing,
/// already-documented "macOS Keychain → empty → static fallback" contract.
/// Only pass `true` from a path the user explicitly initiated.
pub async fn resolve_access_token(config_dir: &Path, allow_keychain_fallback: bool) -> Option<String> {
    if let Some(token) = read_oauth_access_token(config_dir) {
        return Some(token);
    }
    if !allow_keychain_fallback {
        return None;
    }
    tokio::task::spawn_blocking(|| {
        resolve_fallback_access_token(
            || std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok(),
            |token| {
                if let Err(e) = secret_store::put(CLAUDE_OAUTH_TOKEN_ACCOUNT, token) {
                    tracing::warn!("model catalog: failed to persist CLAUDE_CODE_OAUTH_TOKEN: {e}");
                }
            },
            || {
                secret_store::get(CLAUDE_OAUTH_TOKEN_ACCOUNT)
                    .ok()
                    .map(|z| z.to_string())
            },
        )
    })
    .await
    .unwrap_or(None)
}

/// Pure resolution-order logic backing steps 2/3 of [`resolve_access_token`],
/// factored out so it is unit-testable without touching a real OS keychain
/// (`identity::secret_store` wraps the `keyring` crate directly with no
/// mockable seam, so tests inject plain closures here instead).
fn resolve_fallback_access_token(
    env_var: impl FnOnce() -> Option<String>,
    persist: impl FnOnce(&str),
    stored: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if let Some(token) = env_var().filter(|t| !t.is_empty()) {
        persist(&token);
        return Some(token);
    }
    stored().filter(|t| !t.is_empty())
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

    #[tokio::test]
    async fn resolve_access_token_never_touches_keychain_when_fallback_disallowed() {
        // The fix for the app-launch keychain-prompt regression: when
        // `allow_keychain_fallback` is false and no `.credentials.json`
        // exists (the macOS case), this must return None without ever
        // entering the env-var/keychain fallback branch at all — not just
        // "return the same result," but structurally skip it, since even a
        // *read* of a stale keychain entry can trigger an OS prompt. This
        // test can't observe "no syscall happened" directly, but a real
        // keychain entry under CLAUDE_OAUTH_TOKEN_ACCOUNT existing on the
        // machine this test runs on would make the old (pre-fix) code path
        // return Some(..) — so None here is a meaningful assertion, not a
        // vacuous one, whenever such an entry happens to be present.
        let tmp = tempfile::tempdir().unwrap();
        let got = resolve_access_token(tmp.path(), false).await;
        assert_eq!(got, None);
    }

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

    #[test]
    fn fallback_prefers_env_var_and_persists_it() {
        let mut persisted = None;
        let got = resolve_fallback_access_token(
            || Some("sk-ant-oat-from-env".to_string()),
            |token| persisted = Some(token.to_string()),
            || Some("sk-ant-oat-from-store".to_string()),
        );
        assert_eq!(got.as_deref(), Some("sk-ant-oat-from-env"));
        assert_eq!(persisted.as_deref(), Some("sk-ant-oat-from-env"));
    }

    #[test]
    fn fallback_uses_secret_store_when_env_var_absent() {
        let mut persist_calls = 0;
        let got = resolve_fallback_access_token(
            || None,
            |_| persist_calls += 1,
            || Some("sk-ant-oat-from-store".to_string()),
        );
        assert_eq!(got.as_deref(), Some("sk-ant-oat-from-store"));
        assert_eq!(persist_calls, 0);
    }

    #[test]
    fn fallback_treats_empty_env_var_as_absent() {
        let got = resolve_fallback_access_token(
            || Some(String::new()),
            |_| panic!("must not persist an empty token"),
            || Some("sk-ant-oat-from-store".to_string()),
        );
        assert_eq!(got.as_deref(), Some("sk-ant-oat-from-store"));
    }

    #[test]
    fn fallback_none_when_no_source_has_a_token() {
        let got = resolve_fallback_access_token(|| None, |_| panic!("must not persist"), || None);
        assert!(got.is_none());
    }
}
