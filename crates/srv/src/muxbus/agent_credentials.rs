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
//!      client_credentials whenever the cached token has expired. A client
//!      the cloud no longer recognizes is dropped and re-provisioned
//!      (`CredentialFailure::Unrecognized`).
//!
//! Callers (cloud_subscriber.rs) fall back to the shared MUXBUS_TOKEN
//! whenever this returns None — provisioning failure must never block
//! message delivery, only degrade the binding guarantee back to today's
//! self-declared behavior for that agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::backend::storage::store::Store;
use crate::muxbus::cloud_subscriber::load_valid_token;

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

/// A failed provisioning or token fetch, split by what it says about the
/// stored credential (docs/specs/SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md
/// §3.4).
#[derive(Debug)]
enum CredentialFailure {
    /// The cloud doesn't recognize this agent's client — e.g. it was issued
    /// by a user pool the cloud has since left. Retrying with it can never
    /// work: drop the row so the next use re-provisions.
    Unrecognized(String),
    /// Anything else (network, 5xx, 429, a parse error): the credential may
    /// still be good, so keep it and retry after the cooldown.
    Other(String),
}

impl std::fmt::Display for CredentialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialFailure::Unrecognized(m) | CredentialFailure::Other(m) => f.write_str(m),
        }
    }
}

/// Does a client_credentials token endpoint's rejection mean "no such
/// client"? OAuth 2.0 (RFC 6749 §5.2) answers an unknown or unauthenticated
/// client with `invalid_client` on a 400 or 401.
fn token_rejection_is_unrecognized(status: u16, body: &str) -> bool {
    matches!(status, 400 | 401)
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|e| e == "invalid_client"))
            .unwrap_or(false)
}

/// Does `/agents/provision` refusing with `status` mean the stored row for
/// this agent's client can't be used? 401/403 do; 429, 5xx and the rest
/// are worth retrying as they are.
fn provision_rejection_is_unrecognized(status: u16) -> bool {
    matches!(status, 401 | 403)
}

/// Delete this agent's stored credential when the cloud no longer
/// recognizes its client, so the next use re-provisions instead of retrying
/// the dead one every cooldown forever.
fn drop_unrecognized_credential(agent_id: &str, failure: &CredentialFailure, mstore: &Arc<Store>) {
    if !matches!(failure, CredentialFailure::Unrecognized(_)) {
        return;
    }
    match mstore.agent_credential_delete(agent_id) {
        Ok(()) => tracing::info!(
            agent_id = %agent_id,
            "muxbus: cloud no longer recognizes this agent's credential — dropped it; the next use re-provisions",
        ),
        Err(e) => tracing::warn!(
            agent_id = %agent_id, error = %e,
            "muxbus: failed to drop an unrecognized per-agent credential",
        ),
    }
}

/// Get a live per-agent access token, provisioning the Cognito client on
/// first use. Returns None (never an error) on any failure — provisioning
/// being down or an agent not yet migrated must degrade to the caller's
/// shared-token fallback, not block delivery.
pub async fn ensure_agent_credential(
    agent_id: &str,
    mstore: &Arc<Store>,
    http: &reqwest::Client,
) -> Option<String> {
    ensure_agent_credential_at(
        agent_id,
        mstore,
        http,
        &crate::muxbus::relay::rest_base_url(),
        user_token(mstore),
    )
    .await
}

/// [`ensure_agent_credential`]'s body, with the relay base URL and the
/// human's token (only awaited when provisioning) injectable for tests.
async fn ensure_agent_credential_at(
    agent_id: &str,
    mstore: &Arc<Store>,
    http: &reqwest::Client,
    base: &str,
    user_token: impl std::future::Future<Output = Result<String, String>>,
) -> Option<String> {
    let key = agent_id.to_lowercase();

    // One guard covers both network steps below (provisioning and token
    // fetch) — a recent failure in either means skip straight to the
    // shared-token fallback until the cooldown lapses.
    if credential_recently_failed(&key) {
        return None;
    }

    let creds = mstore.agent_credential_load(&key).ok().flatten();

    let creds = match creds {
        Some(c) if !c.client_id.is_empty() => c,
        _ => {
            // Not provisioned yet — do it now, once.
            if let Err(e) = provision_agent_client(&key, mstore, http, base, user_token).await {
                tracing::warn!(
                    agent_id = %key, error = %e,
                    "muxbus: agent credential provisioning failed, backing off {}s",
                    CREDENTIAL_RETRY_COOLDOWN.as_secs(),
                );
                drop_unrecognized_credential(&key, &e, mstore);
                record_credential_failure(&key);
                return None;
            }
            mstore.agent_credential_load(&key).ok().flatten()?
        }
    };

    if creds.is_valid() {
        return Some(creds.access_token);
    }

    match fetch_m2m_token(&key, &creds.client_id, &creds.client_secret, &creds.token_endpoint, mstore, http).await {
        Ok(access_token) => Some(access_token),
        Err(e) => {
            tracing::warn!(
                agent_id = %key, error = %e,
                "muxbus: agent m2m token fetch failed, backing off {}s",
                CREDENTIAL_RETRY_COOLDOWN.as_secs(),
            );
            drop_unrecognized_credential(&key, &e, mstore);
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
pub fn invalidate_cached_token(agent_id: &str, mstore: &Arc<Store>) {
    if let Err(e) = mstore.agent_credential_invalidate_token(&agent_id.to_lowercase()) {
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
pub fn invalidate_binding_mismatched_credential(agent_id: &str, mstore: &Arc<Store>) {
    let key = agent_id.to_lowercase();
    invalidate_cached_token(&key, mstore);
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
        let mstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let agent_id = "test-agent-credentials-binding-mismatch";
        assert!(!credential_recently_failed(agent_id));

        invalidate_binding_mismatched_credential(agent_id, &mstore);

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
        let mstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let agent_id = "test-agent-credentials-plain-401";

        invalidate_cached_token(agent_id, &mstore);

        assert!(!credential_recently_failed(agent_id));
    }

    // ── A client the cloud no longer recognizes is dropped and re-provisioned
    // (SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.4) ───────────────────

    #[test]
    fn only_invalid_client_on_400_or_401_means_the_client_is_unrecognized() {
        let invalid_client = r#"{"error":"invalid_client"}"#;
        assert!(token_rejection_is_unrecognized(400, invalid_client));
        assert!(token_rejection_is_unrecognized(401, invalid_client));
        assert!(!token_rejection_is_unrecognized(500, invalid_client));
        assert!(!token_rejection_is_unrecognized(429, invalid_client));
        assert!(!token_rejection_is_unrecognized(400, r#"{"error":"invalid_request"}"#));
        assert!(!token_rejection_is_unrecognized(400, "<html>bad gateway</html>"));

        assert!(provision_rejection_is_unrecognized(401));
        assert!(provision_rejection_is_unrecognized(403));
        for status in [400, 404, 429, 500, 503] {
            assert!(!provision_rejection_is_unrecognized(status), "{status}");
        }
    }

    type Route = (&'static str, u16, serde_json::Value);

    /// A fake cloud answering `"METHOD /path"` with a scripted status and
    /// JSON body (anything else: 404), recording every request line. `routes`
    /// gets the server's base URL, for bodies that point back at it.
    async fn fake_cloud(routes: impl FnOnce(&str) -> Vec<Route>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&base);
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let app = axum::Router::new().fallback({
            let seen = Arc::clone(&seen);
            move |req: axum::extract::Request| {
                let (seen, routes) = (Arc::clone(&seen), routes.clone());
                async move {
                    let line = format!("{} {}", req.method(), req.uri().path());
                    seen.lock().unwrap().push(line.clone());
                    let (status, body) = routes
                        .iter()
                        .find(|(route, _, _)| *route == line)
                        .map(|(_, status, body)| (*status, body.clone()))
                        .unwrap_or((404, serde_json::Value::Null));
                    (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body))
                }
            }
        });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    /// Provisioning hands out `new-client`, whose token endpoint works.
    fn reprovisioning(base: &str) -> Vec<Route> {
        vec![
            (
                "POST /agents/provision",
                200,
                serde_json::json!({
                    "client_id": "new-client",
                    "client_secret": "new-secret",
                    "token_endpoint": format!("{base}/new/oauth2/token"),
                }),
            ),
            ("POST /new/oauth2/token", 200, serde_json::json!({ "access_token": "fresh", "expires_in": 3600 })),
        ]
    }

    fn unique_agent() -> String {
        format!("test-agent-credentials-{}", uuid::Uuid::new_v4())
    }

    /// A store holding `agent`'s provisioned `old-client`, with no cached token.
    fn provisioned_store(agent: &str, token_endpoint: &str) -> Arc<Store> {
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        mstore.agent_credential_save(agent, "old-client", "old-secret", token_endpoint).unwrap();
        mstore
    }

    async fn ensure(agent: &str, mstore: &Arc<Store>, base: &str) -> Option<String> {
        let user_token = async { Ok("user-token".to_string()) };
        ensure_agent_credential_at(agent, mstore, &reqwest::Client::new(), base, user_token).await
    }

    fn clear_cooldown(agent: &str) {
        credential_cooldown().lock().unwrap().remove(agent);
    }

    #[tokio::test]
    async fn invalid_client_drops_the_row_and_the_next_use_re_provisions() {
        for status in [400, 401] {
            let agent = unique_agent();
            let (base, seen) = fake_cloud(|base| {
                let mut routes = reprovisioning(base);
                routes.push(("POST /old/oauth2/token", status, serde_json::json!({ "error": "invalid_client" })));
                routes
            })
            .await;
            let mstore = provisioned_store(&agent, &format!("{base}/old/oauth2/token"));

            assert_eq!(ensure(&agent, &mstore, &base).await, None);
            assert!(mstore.agent_credential_load(&agent).unwrap().is_none(), "{status}: row dropped");
            assert!(credential_recently_failed(&agent), "{status}: the cooldown still applies");

            // Once the cooldown lapses, the next use provisions a new client.
            clear_cooldown(&agent);
            assert_eq!(ensure(&agent, &mstore, &base).await.as_deref(), Some("fresh"));
            assert_eq!(mstore.agent_credential_load(&agent).unwrap().unwrap().client_id, "new-client");
            assert_eq!(
                *seen.lock().unwrap(),
                ["POST /old/oauth2/token", "POST /agents/provision", "POST /new/oauth2/token"]
            );
        }
    }

    #[tokio::test]
    async fn a_401_or_403_from_provisioning_drops_the_agents_row() {
        for status in [401, 403] {
            let agent = unique_agent();
            let (base, _) = fake_cloud(|_| vec![("POST /agents/provision", status, serde_json::json!({ "error": "forbidden" }))]).await;
            // A row without a client id is what sends this agent to provisioning.
            let mstore = Arc::new(Store::open_in_memory().unwrap());
            mstore.agent_credential_save(&agent, "", "", "").unwrap();

            assert_eq!(ensure(&agent, &mstore, &base).await, None);
            assert!(mstore.agent_credential_load(&agent).unwrap().is_none(), "{status}: row dropped");
            assert!(credential_recently_failed(&agent), "{status}: the cooldown still applies");
        }
    }

    #[tokio::test]
    async fn transient_failures_keep_the_row() {
        // 5xx, 429 and another OAuth error from the token endpoint.
        for (status, body) in [
            (500, serde_json::json!({ "error": "invalid_client" })),
            (503, serde_json::Value::Null),
            (429, serde_json::json!({ "error": "slow_down" })),
            (400, serde_json::json!({ "error": "invalid_request" })),
        ] {
            let agent = unique_agent();
            let (base, _) = fake_cloud(|_| vec![("POST /old/oauth2/token", status, body)]).await;
            let mstore = provisioned_store(&agent, &format!("{base}/old/oauth2/token"));

            assert_eq!(ensure(&agent, &mstore, &base).await, None);
            assert_eq!(
                mstore.agent_credential_load(&agent).unwrap().unwrap().client_id,
                "old-client",
                "{status}: row kept"
            );
            assert!(credential_recently_failed(&agent));
        }

        // A network failure: nothing listens on the token endpoint.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}/oauth2/token", closed.local_addr().unwrap());
        drop(closed);
        let agent = unique_agent();
        let mstore = provisioned_store(&agent, &dead);
        assert_eq!(ensure(&agent, &mstore, "http://127.0.0.1:9").await, None);
        assert_eq!(mstore.agent_credential_load(&agent).unwrap().unwrap().client_id, "old-client");
    }
}

/// The human's own PKCE token, which provisioning is authenticated with.
async fn user_token(mstore: &Arc<Store>) -> Result<String, String> {
    // The scheduler is a process-wide singleton, initialized by
    // cloud_subscriber::run_loop before any WS session (and therefore any
    // handle_server_msg call reaching this function) can start — see
    // crate::broker's own doc comment. get_global() (not init_global) here:
    // this code path has no sweep_interval opinion of its own, it just needs
    // the already-running scheduler ensure_fresh uses for the shared token.
    let scheduler = crate::broker::get_global()
        .ok_or_else(|| "muxbus refresh scheduler not initialized yet".to_string())?;
    load_valid_token(mstore, &scheduler)
        .await
        .ok_or_else(|| "no valid user-level muxbus login to provision from".to_string())
}

/// Calls POST /agents/provision (authenticated with the human's own PKCE
/// token — an M2M agent credential can never provision another one) and
/// caches the returned client_id/client_secret.
async fn provision_agent_client(
    agent_id: &str,
    mstore: &Arc<Store>,
    http: &reqwest::Client,
    base: &str,
    user_token: impl std::future::Future<Output = Result<String, String>>,
) -> Result<(), CredentialFailure> {
    let user_token = user_token.await.map_err(CredentialFailure::Other)?;

    #[derive(serde::Deserialize)]
    struct ProvisionResp {
        client_id: String,
        client_secret: String,
        token_endpoint: String,
    }

    let url = format!("{}/agents/provision", base);
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
        .map_err(|e| CredentialFailure::Other(format!("provision request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let msg = format!("provision failed ({status}): {body}");
        return Err(if provision_rejection_is_unrecognized(status) {
            CredentialFailure::Unrecognized(msg)
        } else {
            CredentialFailure::Other(msg)
        });
    }

    let parsed: ProvisionResp = resp
        .json()
        .await
        .map_err(|e| CredentialFailure::Other(format!("provision response parse failed: {e}")))?;

    mstore
        .agent_credential_save(agent_id, &parsed.client_id, &parsed.client_secret, &parsed.token_endpoint)
        .map_err(|e| CredentialFailure::Other(format!("failed to save agent credential: {e}")))?;

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
    mstore: &Arc<Store>,
    http: &reqwest::Client,
) -> Result<String, CredentialFailure> {
    if client_id.is_empty() || token_endpoint.is_empty() {
        return Err(CredentialFailure::Other(
            "agent credential missing client_id/token_endpoint".to_string(),
        ));
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
        .map_err(|e| CredentialFailure::Other(format!("m2m token request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let msg = format!("m2m token request failed ({status}): {body}");
        return Err(if token_rejection_is_unrecognized(status, &body) {
            CredentialFailure::Unrecognized(msg)
        } else {
            CredentialFailure::Other(msg)
        });
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CredentialFailure::Other(format!("m2m token response parse failed: {e}")))?;

    let access_token = json["access_token"].as_str().unwrap_or("").to_string();
    if access_token.is_empty() {
        return Err(CredentialFailure::Other("m2m token response missing access_token".to_string()));
    }
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
        + expires_in;

    if let Err(e) = mstore.agent_credential_save_token(agent_id, &access_token, expires_at) {
        tracing::warn!(agent_id = %agent_id, error = %e, "muxbus: failed to cache m2m token (will re-fetch next time)");
    }

    Ok(access_token)
}
