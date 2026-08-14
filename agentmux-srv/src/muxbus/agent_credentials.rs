// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent M2M Cognito credential fetch/cache — the client-side half of
//! agentmux-cloud's PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md.
//!
//! Historically every agent under one AgentMux login shared the same
//! account-level MUXBUS_TOKEN for every /reactive/* call, self-declaring its
//! identity via an unverified X-Agent-ID header — any credential could claim
//! any agent_id. This module gets each agent its own bound Cognito
//! client_credentials identity instead:
//!   1. provision_agent_client(): calls POST /agents/provision using the
//!      human's own PKCE token, receiving a Cognito client_id/client_secret
//!      scoped to exactly this (account, agent_id) pair. One-time per agent
//!      (idempotent server-side; cached locally in db_agent_credentials).
//!   2. ensure_agent_credential(): returns a live access_token for that
//!      agent, provisioning on first use and re-fetching via
//!      client_credentials whenever the cached token has expired.
//!
//! Callers (cloud_subscriber.rs) fall back to the shared MUXBUS_TOKEN
//! whenever this returns None — provisioning failure must never block
//! message delivery, only degrade the binding guarantee back to today's
//! self-declared behavior for that agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::backend::storage::store::Store;
use crate::muxbus::cloud_subscriber::{load_valid_token, MUXBUS_REST_URL};

/// Per-request timeout for the two HTTP calls in this module. The shared
/// `http` client (built via `reqwest::Client::new()` in
/// `cloud_subscriber::run_loop`) carries no default timeout, and both
/// calls here are awaited inline in the per-agent InjectAvailable loop —
/// without an explicit override, a stalled provisioning or Cognito token
/// endpoint would hang the whole loop indefinitely, blocking pings and
/// delivery for every OTHER registered agent too. reagentx P1 on PR #2342.
const CREDENTIAL_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to skip re-attempting this agent's credential pipeline
/// (provisioning OR token fetch) after either step fails, before trying
/// again. InjectAvailable broadcasts for ANY injection to ANY agent, so
/// without this an agent whose pipeline can't currently succeed (quota
/// exceeded, malformed agent_id, provisioning/token endpoint down) gets a
/// fresh network round-trip on every single broadcast — an unthrottled
/// retry storm, unlike the broker's single-flight-guarded scheduler used
/// for the shared token. reagentx P2 on PR #2342 (round 1: provisioning
/// only; round 2: also covers fetch_m2m_token, the gap round 1 left open
/// for an already-provisioned agent whose token endpoint is failing).
const CREDENTIAL_RETRY_COOLDOWN: Duration = Duration::from_secs(60);

static CREDENTIAL_COOLDOWN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn credential_cooldown() -> &'static Mutex<HashMap<String, Instant>> {
    CREDENTIAL_COOLDOWN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn credential_recently_failed(agent_id: &str) -> bool {
    let map = credential_cooldown().lock().unwrap();
    map.get(agent_id)
        .is_some_and(|t| t.elapsed() < CREDENTIAL_RETRY_COOLDOWN)
}

fn record_credential_failure(agent_id: &str) {
    credential_cooldown()
        .lock()
        .unwrap()
        .insert(agent_id.to_string(), Instant::now());
}

/// Get a live per-agent access token, provisioning the Cognito client on
/// first use. Returns None (never an error) on any failure — provisioning
/// being down or an agent not yet migrated must degrade to the caller's
/// shared-token fallback, not block delivery.
pub async fn ensure_agent_credential(
    agent_id: &str,
    wstore: &Arc<Store>,
    http: &reqwest::Client,
) -> Option<String> {
    let key = agent_id.to_lowercase();

    // One guard covers both network steps below (provisioning and token
    // fetch) — a recent failure in either means skip straight to the
    // shared-token fallback until the cooldown lapses.
    if credential_recently_failed(&key) {
        return None;
    }

    let creds = wstore.agent_credential_load(&key).ok().flatten();

    let creds = match creds {
        Some(c) if !c.client_id.is_empty() => c,
        _ => {
            // Not provisioned yet — do it now, once.
            if let Err(e) = provision_agent_client(&key, wstore, http).await {
                tracing::warn!(
                    agent_id = %key, error = %e,
                    "muxbus: agent credential provisioning failed, backing off {}s",
                    CREDENTIAL_RETRY_COOLDOWN.as_secs(),
                );
                record_credential_failure(&key);
                return None;
            }
            wstore.agent_credential_load(&key).ok().flatten()?
        }
    };

    if creds.is_valid() {
        return Some(creds.access_token);
    }

    match fetch_m2m_token(&key, &creds.client_id, &creds.client_secret, &creds.token_endpoint, wstore, http).await {
        Ok(access_token) => Some(access_token),
        Err(e) => {
            tracing::warn!(
                agent_id = %key, error = %e,
                "muxbus: agent m2m token fetch failed, backing off {}s",
                CREDENTIAL_RETRY_COOLDOWN.as_secs(),
            );
            record_credential_failure(&key);
            None
        }
    }
}

/// Clear a per-agent credential's cached access token — called by
/// cloud_subscriber when a request using it comes back 401 even though the
/// credential looked locally valid (revoked/rotated server-side out-of-band).
/// Without this, the next InjectAvailable round retries the exact same
/// rejected token forever. Best-effort: a store error here just means the
/// stale token survives until its local expiry, matching the pre-existing
/// failure mode rather than introducing a new one. reagentx P1 on PR #2342.
pub fn invalidate_cached_token(agent_id: &str, wstore: &Arc<Store>) {
    if let Err(e) = wstore.agent_credential_invalidate_token(&agent_id.to_lowercase()) {
        tracing::warn!(
            agent_id = %agent_id, error = %e,
            "muxbus: failed to invalidate stale per-agent credential",
        );
    }
}

/// Same token invalidation as `invalidate_cached_token`, PLUS a
/// `CREDENTIAL_RETRY_COOLDOWN` failure record — for a 403 (binding
/// mismatch) specifically, not a plain 401 (expired token).
///
/// A 401 means the token expired; clearing just the cached token is enough,
/// because the very same `client_id`/`client_secret` is still good and the
/// next `ensure_agent_credential` call correctly mints a fresh token from
/// it. A 403 means the CREDENTIAL ITSELF (not just its cached token) is
/// bound to the wrong agent_id — clearing only the token doesn't fix that:
/// `ensure_agent_credential` still finds the same `client_id` on file,
/// happily mints ANOTHER token from it (Cognito issues tokens for a
/// syntactically valid client/secret regardless of binding correctness —
/// the mismatch is caught downstream by the muxbus server's
/// `checkAgentBinding`, not at token-issuance time), and the very next
/// `/reactive/pending` or `/reactive/ack` call 403s again — repeating the
/// exact same failed round trip on every subsequent `InjectAvailable`
/// broadcast instead of falling back to the shared token (reagentx P2 on
/// PR #2573, flagged by chatgpt-codex-connector originally). Reusing the
/// existing cooldown here bounds that to once per
/// `CREDENTIAL_RETRY_COOLDOWN` window instead of once per broadcast, same
/// throttling this module already applies to a provisioning/fetch failure.
pub fn invalidate_binding_mismatched_credential(agent_id: &str, wstore: &Arc<Store>) {
    let key = agent_id.to_lowercase();
    invalidate_cached_token(&key, wstore);
    record_credential_failure(&key);
}

#[cfg(test)]
mod tests {
    use super::*;

    // credential_cooldown() is a process-wide static shared across every
    // test in this binary — each test below uses its own unique agent_id
    // (not shared with any other test file) so none of them can observe
    // another test's cooldown state.

    #[test]
    fn an_untouched_agent_has_no_recorded_failure() {
        assert!(!credential_recently_failed("test-agent-credentials-untouched"));
    }

    #[test]
    fn record_credential_failure_is_visible_to_credential_recently_failed() {
        let agent_id = "test-agent-credentials-record-failure";
        assert!(!credential_recently_failed(agent_id));
        record_credential_failure(agent_id);
        assert!(credential_recently_failed(agent_id));
    }

    // reagentx P2 on PR #2573: a 403 (binding mismatch) must do more than
    // clear the cached token, or ensure_agent_credential just re-mints
    // another token from the same permanently-mismatched client on the very
    // next call. invalidate_binding_mismatched_credential's whole point is
    // making that next call skip the per-agent pipeline entirely via the
    // cooldown.
    #[test]
    fn invalidate_binding_mismatched_credential_starts_a_cooldown() {
        let wstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let agent_id = "test-agent-credentials-binding-mismatch";
        assert!(!credential_recently_failed(agent_id));

        invalidate_binding_mismatched_credential(agent_id, &wstore);

        assert!(
            credential_recently_failed(agent_id),
            "a 403 binding mismatch must cool down the per-agent pipeline, not just clear the cached token"
        );
    }

    // A plain 401 (expired token) is a normal, expected, recurring event —
    // it must NOT cool down the per-agent pipeline, or a busy agent would
    // get throttled onto the shared token for a full CREDENTIAL_RETRY_COOLDOWN
    // window every time its token simply expires.
    #[test]
    fn invalidate_cached_token_alone_does_not_start_a_cooldown() {
        let wstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let agent_id = "test-agent-credentials-plain-401";

        invalidate_cached_token(agent_id, &wstore);

        assert!(!credential_recently_failed(agent_id));
    }
}

/// Calls POST /agents/provision (authenticated with the human's own PKCE
/// token — an M2M agent credential can never provision another one) and
/// caches the returned client_id/client_secret.
async fn provision_agent_client(agent_id: &str, wstore: &Arc<Store>, http: &reqwest::Client) -> Result<(), String> {
    // The scheduler is a process-wide singleton, initialized by
    // cloud_subscriber::run_loop before any WS session (and therefore any
    // handle_server_msg call reaching this function) can start — see
    // crate::broker's own doc comment. get_global() (not init_global) here:
    // this code path has no sweep_interval opinion of its own, it just needs
    // the already-running scheduler ensure_fresh uses for the shared token.
    let scheduler = crate::broker::get_global()
        .ok_or_else(|| "muxbus refresh scheduler not initialized yet".to_string())?;
    let user_token = load_valid_token(wstore, &scheduler)
        .await
        .ok_or_else(|| "no valid user-level muxbus login to provision from".to_string())?;

    #[derive(serde::Deserialize)]
    struct ProvisionResp {
        client_id: String,
        client_secret: String,
        token_endpoint: String,
    }

    let url = format!("{}/agents/provision", MUXBUS_REST_URL);
    // `http` (built via reqwest::Client::new() in cloud_subscriber::run_loop)
    // carries no default timeout, and this call is awaited inline in the
    // per-agent InjectAvailable loop — a stalled provisioning endpoint would
    // otherwise block pings/delivery for every OTHER agent too, compounding
    // across N agents per broadcast. reagentx P1 on PR #2342.
    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {}", user_token))
        .json(&serde_json::json!({ "agent_id": agent_id }))
        .timeout(CREDENTIAL_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("provision request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("provision failed: {body}"));
    }

    let parsed: ProvisionResp = resp
        .json()
        .await
        .map_err(|e| format!("provision response parse failed: {e}"))?;

    wstore
        .agent_credential_save(agent_id, &parsed.client_id, &parsed.client_secret, &parsed.token_endpoint)
        .map_err(|e| format!("failed to save agent credential: {e}"))?;

    tracing::info!(agent_id = %agent_id, "muxbus: provisioned per-agent credential");
    Ok(())
}

/// Fetches a fresh client_credentials access token and caches it.
/// client_credentials tokens carry no refresh token — expiry just means
/// re-fetching from scratch with the (already-provisioned) client secret.
async fn fetch_m2m_token(
    agent_id: &str,
    client_id: &str,
    client_secret: &str,
    token_endpoint: &str,
    wstore: &Arc<Store>,
    http: &reqwest::Client,
) -> Result<String, String> {
    if client_id.is_empty() || token_endpoint.is_empty() {
        return Err("agent credential missing client_id/token_endpoint".to_string());
    }

    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];
    let resp = http
        .post(token_endpoint)
        .form(&params)
        .timeout(CREDENTIAL_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("m2m token request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("m2m token request failed: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("m2m token response parse failed: {e}"))?;

    let access_token = json["access_token"].as_str().unwrap_or("").to_string();
    if access_token.is_empty() {
        return Err("m2m token response missing access_token".to_string());
    }
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
        + expires_in;

    if let Err(e) = wstore.agent_credential_save_token(agent_id, &access_token, expires_at) {
        tracing::warn!(agent_id = %agent_id, error = %e, "muxbus: failed to cache m2m token (will re-fetch next time)");
    }

    Ok(access_token)
}
