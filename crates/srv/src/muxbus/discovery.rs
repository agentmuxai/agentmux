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
//!
//! The desktop follows a document only to hosts that are AgentMux's own: every host it names (`api`, `ws`, `console`,
//! `cognito.domain`) must be on the list for the base it was fetched from ([`HostRule`]).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Path of the discovery document under the relay REST base.
pub const DISCOVERY_PATH: &str = "/.well-known/agentmux-cloud.json";

/// How long a fetched document (or a failed fetch) is reused before asking again.
const CACHE_TTL: Duration = Duration::from_secs(300);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// The hosts a document fetched from the production relay may name: exact names, not a suffix, so a sub-name nobody
/// listed is never trusted.
///
/// These lists are a contract with the cloud. An installed build refuses every host that is not on its own list, so
/// a host the cloud wants to publish later has to be in a released build first. A change here is a change to what
/// every installed build follows: test it (`the_host_lists_are_pinned`) and release it before the cloud uses it.
pub const PRODUCTION_HOSTS: &[&str] = &[
    "auth.agentmux.ai",
    "cloud.agentmux.ai",
    "muxbus.agentmux.ai",
    "muxbus-ws.agentmux.ai",
    "auth.muxbus.agentmux.ai",
];

/// The names that exist under `staging.agentmux.ai`, for a document fetched from the staging relay. The lists hold
/// only names that exist.
pub const STAGING_HOSTS: &[&str] = &[
    "auth.staging.agentmux.ai",
    "cloud.staging.agentmux.ai",
    "muxbus.staging.agentmux.ai",
    "muxbus-ws.staging.agentmux.ai",
];

/// Which hosts a document may name, decided by the relay base it was fetched from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum HostRule {
    /// The production list. Also what a value that never went through [`parse`] gets: the strictest.
    #[default]
    Production,
    Staging,
    /// A plain-http base: a local relay for tests and development. Any host.
    Local,
    /// Any other https base (set by hand): that base's own host, and the two lists.
    Own(String),
}

/// The host of an `https`/`wss` URL, lower-cased, if it is a bare host name: no credentials in it and no port other
/// than the scheme's own.
fn bare_host(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    Some(url.host_str()?.to_ascii_lowercase())
}

impl HostRule {
    fn for_base(rest_base: &str) -> HostRule {
        if rest_base.starts_with("http://") {
            return HostRule::Local;
        }
        match bare_host(rest_base) {
            Some(host) if PRODUCTION_HOSTS.contains(&host.as_str()) => HostRule::Production,
            Some(host) if STAGING_HOSTS.contains(&host.as_str()) => HostRule::Staging,
            Some(host) => HostRule::Own(host),
            None => HostRule::Production,
        }
    }

    /// Whether the host `url` names may be followed.
    fn allows(&self, url: &str) -> bool {
        if *self == HostRule::Local {
            return true;
        }
        let Some(host) = bare_host(url) else {
            return false;
        };
        let host = host.as_str();
        match self {
            HostRule::Production => PRODUCTION_HOSTS.contains(&host),
            HostRule::Staging => STAGING_HOSTS.contains(&host),
            HostRule::Own(own) => host == own || PRODUCTION_HOSTS.contains(&host) || STAGING_HOSTS.contains(&host),
            HostRule::Local => true,
        }
    }
}

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
    /// Which hosts this document may name, from the base it was fetched from. Set by [`parse`].
    #[serde(skip)]
    rule: HostRule,
}

impl CloudSettings {
    /// Whether the host `url` names is one this document may send a sign-in or a token to. The check at the point of
    /// use, whatever [`parse`] did.
    pub fn allows_host_of(&self, url: &str) -> bool {
        self.rule.allows(url)
    }
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
    /// The pool's social sign-ins (Cognito names, e.g. `Google`,
    /// `SignInWithApple`). Empty from a cloud that doesn't publish them.
    #[serde(default)]
    pub identity_providers: Vec<String>,
}

/// The providers a sign-in may go straight to (Cognito's `identity_provider`).
/// Nothing else reaches the authorize URL.
pub const DIRECT_IDENTITY_PROVIDERS: &[&str] = &["Google", "SignInWithApple"];

/// Check a requested sign-in provider: one of `DIRECT_IDENTITY_PROVIDERS`, and
/// one the cloud publishes when it publishes a list. `None` (email, or
/// Cognito's own page) is always fine.
pub fn check_identity_provider(
    discovered: Option<&CloudSettings>,
    requested: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(provider) = requested else {
        return Ok(None);
    };
    if !DIRECT_IDENTITY_PROVIDERS.contains(&provider) {
        return Err(format!("AgentMux Cloud has no sign-in with \"{provider}\""));
    }
    if let Some(published) = discovered.map(|s| &s.cognito.identity_providers) {
        if !published.is_empty() && !published.iter().any(|p| p == provider) {
            return Err(format!("AgentMux Cloud doesn't offer sign-in with {provider} right now"));
        }
    }
    Ok(Some(provider.to_string()))
}

/// Parse and check a discovery document. `None` for anything srv shouldn't
/// act on: another format version, a missing field, a URL that isn't
/// https/wss (plain http/ws only when the relay itself is plain http, i.e. a
/// local dev relay), or a host that is not AgentMux's own (see [`HostRule`]).
pub fn parse(body: &str, rest_base: &str) -> Option<CloudSettings> {
    check(body, rest_base).ok()
}

/// [`parse`], saying why a document is refused. The reason names a field, never a value.
fn check(body: &str, rest_base: &str) -> Result<CloudSettings, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "not JSON".to_string())?;
    if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("not a version 1 document".to_string());
    }
    let mut settings: CloudSettings = serde_json::from_value(value).map_err(|_| "a field is missing".to_string())?;
    let local = rest_base.starts_with("http://");
    let secure = |url: &str, scheme: &str, dev: &str| url.starts_with(scheme) || (local && url.starts_with(dev));
    let ok = secure(&settings.api, "https://", "http://")
        && secure(&settings.ws, "wss://", "ws://")
        && secure(&settings.cognito.domain, "https://", "http://")
        && !settings.cognito.client_id.is_empty();
    if !ok {
        return Err("a URL scheme or the client id is not acceptable".to_string());
    }
    settings.rule = HostRule::for_base(rest_base);
    let named = [
        ("api", &settings.api),
        ("ws", &settings.ws),
        ("console", &settings.console),
        ("cognito.domain", &settings.cognito.domain),
    ];
    for (field, url) in named {
        if !url.is_empty() && !settings.rule.allows(url) {
            return Err(format!("{field} names a host that is not AgentMux's own"));
        }
    }
    Ok(settings)
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
    let sent = http.get(&url).timeout(FETCH_TIMEOUT).send().await;
    crate::connectivity::report_http(crate::connectivity::Service::Muxbus, &sent);
    let resp = match sent {
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
    match check(&body, base) {
        Ok(settings) => Some(settings),
        Err(why) => {
            tracing::warn!(url = %url, reason = %why, "cloud discovery: document refused; using defaults");
            None
        }
    }
}

/// Where the cloud subscriber connects: the published WebSocket URL, else the
/// compiled one.
pub async fn ws_url(http: &reqwest::Client) -> String {
    // The same kind of override as `relay::rest_base_url`'s
    // `AGENTMUX_MUXBUS_REST_URL`: for tests and a local muxbus server.
    if let Some(url) = std::env::var("AGENTMUX_MUXBUS_WS_URL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return url;
    }
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
    if let Some(s) = discovered.filter(|s| s.allows_host_of(&s.cognito.domain)) {
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
        "api": "https://muxbus.agentmux.ai",
        "ws": "wss://muxbus-ws.agentmux.ai",
        "console": "https://cloud.agentmux.ai",
        "cognito": { "domain": "https://auth.agentmux.ai", "clientId": "abc123", "region": "us-east-1", "userPoolId": "us-east-1_X" },
        "someFutureField": true
    }"#;

    /// The compiled production relay base.
    const BASE: &str = "https://muxbus.agentmux.ai";

    #[test]
    fn a_version_1_document_parses_and_ignores_unknown_fields() {
        let s = parse(DOC, BASE).unwrap();
        assert_eq!(s.ws, "wss://muxbus-ws.agentmux.ai");
        assert_eq!(s.cognito.client_id, "abc123");
        assert_eq!(s.cognito.user_pool_id, "us-east-1_X");
        assert!(s.cognito.identity_providers.is_empty(), "an older document has no provider list");
    }

    #[test]
    fn the_published_identity_providers_parse() {
        let doc = DOC.replace("\"region\"", "\"identityProviders\": [\"Google\", \"SignInWithApple\"], \"region\"");
        let s = parse(&doc, BASE).unwrap();
        assert_eq!(s.cognito.identity_providers, vec!["Google", "SignInWithApple"]);
    }

    #[test]
    fn a_sign_in_provider_must_be_known_and_published() {
        let old = parse(DOC, BASE).unwrap();
        let google_only = parse(
            &DOC.replace("\"region\"", "\"identityProviders\": [\"Google\"], \"region\""),
            BASE,
        )
        .unwrap();
        assert_eq!(check_identity_provider(Some(&google_only), None), Ok(None));
        assert_eq!(check_identity_provider(Some(&google_only), Some("Google")), Ok(Some("Google".to_string())));
        assert!(check_identity_provider(Some(&google_only), Some("SignInWithApple")).is_err(), "not published");
        // No list (an older cloud, or no document): any known provider.
        assert!(check_identity_provider(Some(&old), Some("SignInWithApple")).is_ok());
        assert!(check_identity_provider(None, Some("Google")).is_ok());
        assert!(check_identity_provider(None, Some("Facebook")).is_err());
        assert!(check_identity_provider(None, Some("Google&prompt=none")).is_err());
        assert!(check_identity_provider(None, Some("")).is_err());
    }

    #[test]
    fn other_versions_and_incomplete_documents_are_ignored() {
        let base = BASE;
        assert!(parse(&DOC.replace("\"version\": 1", "\"version\": 2"), base).is_none());
        assert!(parse(&DOC.replace("\"clientId\": \"abc123\"", "\"clientId\": \"\""), base).is_none());
        assert!(parse(r#"{"version":1,"api":"https://a","ws":"wss://b"}"#, base).is_none());
        assert!(parse("not json", base).is_none());
    }

    #[test]
    fn insecure_urls_are_refused_except_from_a_local_dev_relay() {
        let plain = DOC.replace("wss://muxbus-ws", "ws://muxbus-ws");
        assert!(parse(&plain, BASE).is_none());
        assert!(parse(&plain, "http://localhost:3100").is_some());
        let plain_auth = DOC.replace("https://auth.agentmux.ai", "http://auth.agentmux.ai");
        assert!(parse(&plain_auth, BASE).is_none());
    }

    #[test]
    fn login_uses_the_published_pair_else_the_builds_else_errors() {
        let s = parse(DOC, BASE).unwrap();
        assert_eq!(
            resolve_login(Some(&s), "https://compiled", "compiled-id").unwrap(),
            ("https://auth.agentmux.ai".to_string(), "abc123".to_string())
        );
        assert_eq!(
            resolve_login(None, "https://compiled", "compiled-id").unwrap(),
            ("https://compiled".to_string(), "compiled-id".to_string())
        );
        assert!(resolve_login(None, "https://compiled", "").is_err());
        assert!(resolve_login(None, "", "compiled-id").is_err(), "half a pair is no pair");
        assert!(resolve_login(None, "", "").is_err());
    }

    #[test]
    fn a_sign_in_from_another_client_is_superseded_only_when_the_cloud_says_so() {
        let s = parse(DOC, BASE).unwrap();
        assert!(client_id_superseded(Some(&s), "old-client"));
        assert!(!client_id_superseded(Some(&s), "abc123"));
        assert!(!client_id_superseded(None, "old-client"), "no document: can't tell");
        assert!(!client_id_superseded(Some(&s), ""), "no recorded client id: can't tell");
    }

    #[test]
    fn the_host_lists_are_pinned() {
        // A host added here is followed by every build that has it and refused by every build that does not.
        assert_eq!(
            PRODUCTION_HOSTS,
            ["auth.agentmux.ai", "cloud.agentmux.ai", "muxbus.agentmux.ai", "muxbus-ws.agentmux.ai", "auth.muxbus.agentmux.ai"]
        );
        assert_eq!(
            STAGING_HOSTS,
            [
                "auth.staging.agentmux.ai",
                "cloud.staging.agentmux.ai",
                "muxbus.staging.agentmux.ai",
                "muxbus-ws.staging.agentmux.ai"
            ]
        );
    }

    #[test]
    fn every_listed_host_is_accepted_in_every_field_it_may_be_published_in() {
        for host in PRODUCTION_HOSTS {
            let doc = DOC
                .replace("https://muxbus.agentmux.ai", &format!("https://{host}"))
                .replace("wss://muxbus-ws.agentmux.ai", &format!("wss://{host}"))
                .replace("https://cloud.agentmux.ai", &format!("https://{host}"))
                .replace("https://auth.agentmux.ai", &format!("https://{host}"));
            assert!(parse(&doc, BASE).is_some(), "{host}");
        }
    }

    #[test]
    fn a_document_naming_another_host_in_any_field_is_refused() {
        let other = "other.example.test";
        for (field, doc) in [
            ("api", DOC.replace("https://muxbus.agentmux.ai", &format!("https://{other}"))),
            ("ws", DOC.replace("wss://muxbus-ws.agentmux.ai", &format!("wss://{other}"))),
            ("console", DOC.replace("https://cloud.agentmux.ai", &format!("https://{other}"))),
            ("cognito.domain", DOC.replace("https://auth.agentmux.ai", &format!("https://{other}"))),
        ] {
            assert!(parse(&doc, BASE).is_none(), "{field}");
            let why = check(&doc, BASE).unwrap_err();
            assert!(why.starts_with(field) && !why.contains(other), "the reason names the field, not the host: {why}");
        }
    }

    #[test]
    fn a_host_is_matched_exactly_not_by_its_suffix_and_without_credentials_or_a_port() {
        for bad in [
            "https://evil.auth.agentmux.ai",
            "https://auth.agentmux.ai.evil.test",
            "https://agentmux.ai",
            "https://xauth.agentmux.ai",
            "https://auth.agentmux.ai@evil.test",
            "https://evil.test@auth.agentmux.ai",
            "https://auth.agentmux.ai:8443",
            "https://auth.staging.agentmux.ai",
            "https://auth.agentmux.ai.",
        ] {
            let doc = DOC.replace("https://auth.agentmux.ai", bad);
            assert!(parse(&doc, BASE).is_none(), "{bad}");
        }
        // The host name's letter case and a path do not matter; the name does.
        assert!(parse(&DOC.replace("https://auth.agentmux.ai", "https://AUTH.AgentMux.ai/login"), BASE).is_some());
        assert!(parse(&DOC.replace("https://auth.agentmux.ai", "https://auth.agentmux.ai:443"), BASE).is_some());
    }

    #[test]
    fn the_list_is_chosen_by_the_base_the_document_came_from() {
        let staging = DOC.replace(".agentmux.ai", ".staging.agentmux.ai");
        // Staging names from a staging base; production names from a production base; never across.
        assert!(parse(&staging, "https://muxbus.staging.agentmux.ai").is_some());
        assert!(parse(&staging, BASE).is_none());
        assert!(parse(DOC, "https://muxbus.staging.agentmux.ai").is_none());
        // Either name of an environment's relay picks that environment's list: muxbus.X today, cloud.X later.
        assert!(parse(DOC, "https://cloud.agentmux.ai").is_some());
        assert!(parse(&staging, "https://cloud.staging.agentmux.ai").is_some());
        assert!(parse(DOC, "https://cloud.staging.agentmux.ai").is_none());
        assert!(parse(&staging, "https://cloud.agentmux.ai").is_none());
        // A plain-http base is a local relay: any host, as before.
        assert!(parse(&DOC.replace("https://auth.agentmux.ai", "http://127.0.0.1:9000"), "http://localhost:3100").is_some());
        // Another https base set by hand: its own host and the two lists, nothing else.
        let own = "https://relay.example.test";
        assert!(parse(&DOC.replace("https://muxbus.agentmux.ai", own), own).is_some());
        assert!(parse(&DOC.replace("https://muxbus.agentmux.ai", "https://other.example.test"), own).is_none());
        assert!(parse(&DOC.replace("https://auth.agentmux.ai", "https://auth.example.test"), own).is_none());
    }

    #[test]
    fn a_sign_in_never_starts_at_a_host_that_is_not_the_documents_to_name() {
        // Whatever `parse` let through, the host is checked again where it is used.
        let mut s = parse(DOC, BASE).unwrap();
        s.cognito.domain = "https://other.example.test".to_string();
        assert_eq!(
            resolve_login(Some(&s), "https://compiled", "compiled-id").unwrap(),
            ("https://compiled".to_string(), "compiled-id".to_string()),
            "the compiled pair keeps working"
        );
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
        let old = parse(DOC, BASE).unwrap();
        let new = parse(&DOC.replace("abc123", "def456"), BASE).unwrap();
        assert_eq!(keep_last_good(Some(&old), None), Some(old.clone()));
        assert_eq!(keep_last_good(Some(&old), Some(new.clone())), Some(new));
        assert_eq!(keep_last_good(None, None), None);
    }
}
