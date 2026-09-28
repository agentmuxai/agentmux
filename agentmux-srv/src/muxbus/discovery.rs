// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux Cloud settings discovery
//! (docs/specs/SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md).
//!
//! The cloud publishes where its sign-in and relay live at
//! `{relay REST base}/.well-known/agentmux-cloud.json`. srv reads it so the
//! desktop follows the cloud instead of compiled-in values: a new Cognito
//! user pool means a new client id, and installed builds would otherwise keep
//! the old one. The relay REST base (`relay::rest_base_url`) stays the root of
//! trust; everything else comes from the document it serves, with today's
//! compiled values as the fallback.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Path of the discovery document under the relay REST base.
pub const DISCOVERY_PATH: &str = "/.well-known/agentmux-cloud.json";

/// How long a fetched document (or a failed fetch) is reused before asking again.
const CACHE_TTL: Duration = Duration::from_secs(300);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// What the cloud publishes. Only the fields srv uses; others are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CloudSettings {
    /// Relay REST base URL.
    pub api: String,
    /// Relay WebSocket URL.
    pub ws: String,
    /// Cloud Console URL.
    #[serde(default)]
    pub console: String,
    pub cognito: CognitoSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CognitoSettings {
    /// Hosted-UI base URL.
    pub domain: String,
    /// The desktop's public PKCE app client.
    pub client_id: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub user_pool_id: String,
}

/// Parse and check a discovery document. `None` for anything srv shouldn't
/// act on: another format version, a missing field, or a URL that isn't
/// https/wss (plain http/ws only when the relay itself is plain http, i.e. a
/// local dev relay).
pub fn parse(body: &str, rest_base: &str) -> Option<CloudSettings> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return None;
    }
    let settings: CloudSettings = serde_json::from_value(value).ok()?;
    let local = rest_base.starts_with("http://");
    let secure = |url: &str, scheme: &str, dev: &str| url.starts_with(scheme) || (local && url.starts_with(dev));
    let ok = secure(&settings.api, "https://", "http://")
        && secure(&settings.ws, "wss://", "ws://")
        && secure(&settings.cognito.domain, "https://", "http://")
        && !settings.cognito.client_id.is_empty();
    ok.then_some(settings)
}

struct Cache {
    fetched_at: Instant,
    /// The document from the last successful fetch, kept across failed ones.
    last_good: Option<CloudSettings>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The cloud's published settings, or `None` when it publishes none (an older
/// relay, a network failure with no earlier copy). Cached for five minutes; a
/// failed fetch keeps the last good copy and also waits five minutes before
/// trying again, so an unreachable relay doesn't turn every caller into a
/// request.
pub async fn cloud_settings(http: &reqwest::Client) -> Option<CloudSettings> {
    if let Some(cache) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if cache.fetched_at.elapsed() < CACHE_TTL {
            return cache.last_good.clone();
        }
    }
    let base = super::relay::rest_base_url();
    let fetched = fetch(http, &base).await;
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let last_good = keep_last_good(guard.as_ref().and_then(|c| c.last_good.as_ref()), fetched);
    *guard = Some(Cache { fetched_at: Instant::now(), last_good: last_good.clone() });
    last_good
}

/// A fresh document replaces the cached one; a failed fetch keeps it.
fn keep_last_good(previous: Option<&CloudSettings>, fetched: Option<CloudSettings>) -> Option<CloudSettings> {
    fetched.or_else(|| previous.cloned())
}

async fn fetch(http: &reqwest::Client, base: &str) -> Option<CloudSettings> {
    let url = format!("{}{}", base.trim_end_matches('/'), DISCOVERY_PATH);
    let resp = match http.get(&url).timeout(FETCH_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(url = %url, error = %e, "cloud discovery: fetch failed; using defaults");
            return None;
        }
    };
    if !resp.status().is_success() {
        tracing::debug!(url = %url, status = %resp.status(), "cloud discovery: no document; using defaults");
        return None;
    }
    let body = resp.text().await.ok()?;
    let parsed = parse(&body, base);
    if parsed.is_none() {
        tracing::warn!(url = %url, "cloud discovery: document rejected (version, fields or URL schemes); using defaults");
    }
    parsed
}

/// Where the cloud subscriber connects: the published WebSocket URL, else the
/// compiled one.
pub async fn ws_url(http: &reqwest::Client) -> String {
    cloud_settings(http)
        .await
        .map(|s| s.ws)
        .unwrap_or_else(|| super::cloud_subscriber::MUXBUS_WS_URL.to_string())
}

/// The Cognito domain and client id to sign in with: the published ones when
/// the cloud has a document, else what the caller supplied (the build's
/// compiled values). Both of a pair come from the same source, never mixed.
pub fn resolve_login(
    discovered: Option<&CloudSettings>,
    supplied_domain: &str,
    supplied_client_id: &str,
) -> Result<(String, String), String> {
    if let Some(s) = discovered {
        return Ok((s.cognito.domain.clone(), s.cognito.client_id.clone()));
    }
    if !supplied_domain.is_empty() && !supplied_client_id.is_empty() {
        return Ok((supplied_domain.to_string(), supplied_client_id.to_string()));
    }
    Err("AgentMux Cloud sign-in isn't configured: the cloud publishes no settings and this build has none".to_string())
}

/// Was a stored sign-in issued to a Cognito client the cloud no longer
/// publishes? Then it belongs to a user pool the cloud has left, and neither
/// its tokens nor a refresh will work. `false` when that can't be told: no
/// document, or a credential saved without its client id.
pub fn client_id_superseded(discovered: Option<&CloudSettings>, stored_client_id: &str) -> bool {
    match discovered {
        Some(s) => !stored_client_id.is_empty() && s.cognito.client_id != stored_client_id,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{
        "version": 1,
        "api": "https://muxbus.example.test",
        "ws": "wss://muxbus-ws.example.test",
        "console": "https://cloud.example.test",
        "cognito": { "domain": "https://auth.example.test", "clientId": "abc123", "region": "us-east-1", "userPoolId": "us-east-1_X" },
        "someFutureField": true
    }"#;

    #[test]
    fn a_version_1_document_parses_and_ignores_unknown_fields() {
        let s = parse(DOC, "https://muxbus.example.test").unwrap();
        assert_eq!(s.ws, "wss://muxbus-ws.example.test");
        assert_eq!(s.cognito.client_id, "abc123");
        assert_eq!(s.cognito.user_pool_id, "us-east-1_X");
    }

    #[test]
    fn other_versions_and_incomplete_documents_are_ignored() {
        let base = "https://muxbus.example.test";
        assert!(parse(&DOC.replace("\"version\": 1", "\"version\": 2"), base).is_none());
        assert!(parse(&DOC.replace("\"clientId\": \"abc123\"", "\"clientId\": \"\""), base).is_none());
        assert!(parse(r#"{"version":1,"api":"https://a","ws":"wss://b"}"#, base).is_none());
        assert!(parse("not json", base).is_none());
    }

    #[test]
    fn insecure_urls_are_refused_except_from_a_local_dev_relay() {
        let plain = DOC.replace("wss://muxbus-ws", "ws://muxbus-ws");
        assert!(parse(&plain, "https://muxbus.example.test").is_none());
        assert!(parse(&plain, "http://localhost:3100").is_some());
        let plain_auth = DOC.replace("https://auth", "http://auth");
        assert!(parse(&plain_auth, "https://muxbus.example.test").is_none());
    }

    #[test]
    fn login_uses_the_published_pair_else_the_builds_else_errors() {
        let s = parse(DOC, "https://muxbus.example.test").unwrap();
        assert_eq!(
            resolve_login(Some(&s), "https://compiled", "compiled-id").unwrap(),
            ("https://auth.example.test".to_string(), "abc123".to_string())
        );
        assert_eq!(
            resolve_login(None, "https://compiled", "compiled-id").unwrap(),
            ("https://compiled".to_string(), "compiled-id".to_string())
        );
        assert!(resolve_login(None, "https://compiled", "").is_err());
        assert!(resolve_login(None, "", "").is_err());
    }

    #[test]
    fn a_sign_in_from_another_client_is_superseded_only_when_the_cloud_says_so() {
        let s = parse(DOC, "https://muxbus.example.test").unwrap();
        assert!(client_id_superseded(Some(&s), "old-client"));
        assert!(!client_id_superseded(Some(&s), "abc123"));
        assert!(!client_id_superseded(None, "old-client"), "no document: can't tell");
        assert!(!client_id_superseded(Some(&s), ""), "no recorded client id: can't tell");
    }

    /// A relay that answers every request with `status` and `body`.
    async fn fake_relay(status: &'static str, body: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_relay_serving_the_document_is_discovered() {
        let base = fake_relay("200 OK", DOC.to_string()).await;
        let s = fetch(&reqwest::Client::new(), &base).await.expect("document served");
        assert_eq!(s.cognito.client_id, "abc123");
    }

    #[tokio::test]
    async fn an_unreachable_relay_or_one_without_the_document_is_not_discovered() {
        let http = reqwest::Client::new();
        // Nothing listens on port 1.
        assert!(fetch(&http, "http://127.0.0.1:1").await.is_none());
        let base = fake_relay("404 Not Found", "{}".to_string()).await;
        assert!(fetch(&http, &base).await.is_none());
        let base = fake_relay("200 OK", DOC.replace("\"version\": 1", "\"version\": 2")).await;
        assert!(fetch(&http, &base).await.is_none());
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_good_document() {
        let old = parse(DOC, "https://muxbus.example.test").unwrap();
        let new = parse(&DOC.replace("abc123", "def456"), "https://muxbus.example.test").unwrap();
        assert_eq!(keep_last_good(Some(&old), None), Some(old.clone()));
        assert_eq!(keep_last_good(Some(&old), Some(new.clone())), Some(new));
        assert_eq!(keep_last_good(None, None), None);
    }
}
