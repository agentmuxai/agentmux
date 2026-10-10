// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `net.fetch` for sandboxed widgets
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3).
//!
//! srv makes the request, not the widget: no cookies, no proxy, no browser
//! profile, no AgentMux origin. The URL's origin must match one of the
//! package's `net:` grants, and so must every redirect. The host's addresses
//! are resolved once and the connection is pinned to them, so a name can't
//! resolve to one address for the check and another for the request. A
//! private or local address is reachable only through a grant that names it
//! as an IP address, or `localhost` for loopback, port included.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use url::Url;

use super::registry_probe::ip_is_blocked;
use super::widget_access::AccessError;

pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REDIRECTS: usize = 5;
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// Request headers a widget can't set: srv sets them, or they'd change how
/// the connection itself behaves.
const REFUSED_HEADERS: &[&str] = &[
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "keep-alive",
    "expect",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRequest {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_base64: Option<String>,
    pub url: String,
}

/// One `net:` grant, parsed.
#[derive(Debug, Clone, PartialEq)]
struct Grant {
    scheme: String,
    /// The host, or the domain under `*.`.
    host: String,
    wildcard: bool,
    port: u16,
}

fn parse_grant(permission: &str) -> Option<Grant> {
    let origin = permission.strip_prefix("net:")?;
    let (wildcard, parsable) = match origin.split_once("://*.") {
        Some((scheme, rest)) => (true, format!("{scheme}://{rest}")),
        None => (false, origin.to_string()),
    };
    let u = Url::parse(&parsable).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    Some(Grant {
        scheme: u.scheme().to_string(),
        host: u.host_str()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase(),
        wildcard,
        port: u.port_or_known_default()?,
    })
}

fn url_host(url: &Url) -> Option<String> {
    Some(url.host_str()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase())
}

impl Grant {
    fn covers(&self, url: &Url) -> bool {
        let Some(host) = url_host(url) else { return false };
        url.scheme() == self.scheme
            && url.port_or_known_default() == Some(self.port)
            && if self.wildcard { host.len() > self.host.len() + 1 && host.ends_with(&format!(".{}", self.host)) } else { host == self.host }
    }

    /// May a request under this grant reach `ip`? A public address, yes. A
    /// private or local one only when the grant names it: the same IP
    /// address, or `localhost` for loopback.
    fn may_reach(&self, ip: IpAddr) -> bool {
        if ip.is_unspecified() || ip.is_multicast() || matches!(ip, IpAddr::V4(v4) if v4.is_broadcast()) {
            return false;
        }
        if !ip_is_blocked(ip) {
            return true;
        }
        if self.wildcard {
            return false;
        }
        match self.host.parse::<IpAddr>() {
            Ok(named) => named == ip,
            Err(_) => self.host == "localhost" && ip.is_loopback(),
        }
    }
}

/// The grant among `granted` that covers `url`, if any.
fn grant_for(granted: &[String], url: &Url) -> Option<Grant> {
    granted.iter().filter_map(|g| parse_grant(g)).find(|g| g.covers(url))
}

/// The permission a request to `url` needs, named the way the manifest
/// writes it (for the error).
fn needed(url: &Url) -> String {
    format!("net:{}", url.origin().ascii_serialization())
}

async fn resolve(url: &Url, grant: &Grant, pkg_name: &str) -> Result<(String, Vec<SocketAddr>), AccessError> {
    let host = url_host(url).ok_or_else(|| AccessError::invalid("the url has no host"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host.as_str(), port)))
            .await
            .map_err(|_| AccessError::network(format!("looking up {host} timed out")))?
            .map_err(|e| AccessError::network(format!("can't look up {host}: {e}")))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(AccessError::network(format!("{host} has no address")));
    }
    if let Some(bad) = addrs.iter().find(|a| !grant.may_reach(a.ip())) {
        return Err(AccessError::denied(
            pkg_name,
            &format!("net:{}://{}:{}", url.scheme(), bad.ip(), port),
        )
        .with_message(format!(
            "{host} is a private or local address ({}); only a permission naming that address can reach it",
            bad.ip()
        )));
    }
    Ok((host, addrs))
}

impl AccessError {
    fn with_message(mut self, message: String) -> Self {
        self.message = message;
        self
    }
}

fn is_text(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("text/")
        || ct.contains("json")
        || ct.contains("xml")
        || ct.contains("javascript")
        || ct.contains("x-www-form-urlencoded")
}

/// Makes the request for a package granted `granted`.
pub async fn fetch(pkg_name: &str, granted: &[String], req: FetchRequest) -> Result<FetchResponse, AccessError> {
    let mut url = Url::parse(&req.url).map_err(|_| AccessError::invalid("net.fetch needs an absolute http(s) url"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(AccessError::invalid("net.fetch takes http and https urls only"));
    }
    let mut method = reqwest::Method::from_bytes(req.method.as_deref().unwrap_or("GET").to_ascii_uppercase().as_bytes())
        .map_err(|_| AccessError::invalid("not an HTTP method"))?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &req.headers {
        let lower = name.to_ascii_lowercase();
        if REFUSED_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
            return Err(AccessError::invalid(format!("a widget can't set the {name} header")));
        }
        let n = reqwest::header::HeaderName::from_bytes(lower.as_bytes()).map_err(|_| AccessError::invalid(format!("bad header name {name}")))?;
        let v = reqwest::header::HeaderValue::from_str(value).map_err(|_| AccessError::invalid(format!("bad value for header {name}")))?;
        headers.append(n, v);
    }
    let mut body: Option<Vec<u8>> = match (&req.body, &req.body_base64) {
        (Some(_), Some(_)) => return Err(AccessError::invalid("give body or bodyBase64, not both")),
        (Some(text), None) => Some(text.clone().into_bytes()),
        (None, Some(b64)) => Some(
            base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| AccessError::invalid("bodyBase64 isn't base64"))?,
        ),
        (None, None) => None,
    };
    if body.as_ref().is_some_and(|b| b.len() > MAX_REQUEST_BYTES) {
        return Err(AccessError::limit("requestBody", "a request body is limited to 10 MB"));
    }
    let timeout = req.timeout_ms.map(Duration::from_millis).unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
    let deadline = Instant::now() + timeout;

    for _hop in 0..=MAX_REDIRECTS {
        let grant = grant_for(granted, &url).ok_or_else(|| AccessError::denied(pkg_name, &needed(&url)))?;
        let (host, addrs) = resolve(&url, &grant, pkg_name).await?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(AccessError::network("the request timed out"));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(&host, &addrs)
            .timeout(left)
            .user_agent(concat!("AgentMux-Widget/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AccessError::internal(e.to_string()))?;
        let mut builder = client.request(method.clone(), url.clone()).headers(headers.clone());
        if let Some(b) = &body {
            builder = builder.body(b.clone());
        }
        let resp = builder.send().await.map_err(|e| {
            AccessError::network(if e.is_timeout() { "the request timed out".to_string() } else { format!("the request failed: {e}") })
        })?;
        let status = resp.status();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) {
                let next = url.join(loc).map_err(|_| AccessError::network("a redirect to a bad url"))?;
                if grant_for(granted, &next).is_none() {
                    return Err(AccessError::network(format!("a redirect to {} isn't allowed", next.origin().ascii_serialization())));
                }
                // As browsers do: 303, and 301/302 after a POST, become a GET.
                if status == reqwest::StatusCode::SEE_OTHER
                    || ((status == reqwest::StatusCode::MOVED_PERMANENTLY || status == reqwest::StatusCode::FOUND) && method == reqwest::Method::POST)
                {
                    method = reqwest::Method::GET;
                    body = None;
                }
                url = next;
                continue;
            }
        }
        return read(resp, &url).await;
    }
    Err(AccessError::network(format!("more than {MAX_REDIRECTS} redirects")))
}

async fn read(mut resp: reqwest::Response, url: &Url) -> Result<FetchResponse, AccessError> {
    let status = resp.status();
    let mut headers = BTreeMap::new();
    for (name, value) in resp.headers() {
        let v = String::from_utf8_lossy(value.as_bytes()).into_owned();
        headers.entry(name.as_str().to_string()).and_modify(|e: &mut String| {
            e.push_str(", ");
            e.push_str(&v);
        }).or_insert(v);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| AccessError::network(format!("reading the response failed: {e}")))? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AccessError::limit("responseBody", "a response is limited to 10 MB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = headers.get("content-type").is_some_and(|ct| is_text(ct));
    let (body, body_base64) = match (text, String::from_utf8(bytes)) {
        (true, Ok(s)) => (Some(s), None),
        (_, Ok(s)) => (None, Some(base64::engine::general_purpose::STANDARD.encode(s.as_bytes()))),
        (_, Err(e)) => (None, Some(base64::engine::general_purpose::STANDARD.encode(e.as_bytes()))),
    };
    Ok(FetchResponse {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        body_base64,
        url: url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn grants_cover_exact_origins_and_subdomains_only() {
        let g = |p: &str| parse_grant(p).unwrap();
        assert!(g("net:https://api.github.com").covers(&u("https://api.github.com/user")));
        assert!(!g("net:https://api.github.com").covers(&u("http://api.github.com/user")));
        assert!(!g("net:https://api.github.com").covers(&u("https://api.github.com:8443/user")));
        assert!(!g("net:https://api.github.com").covers(&u("https://evil.api.github.com/")));
        assert!(g("net:https://*.example.com").covers(&u("https://a.example.com/x")));
        assert!(!g("net:https://*.example.com").covers(&u("https://example.com/x")));
        assert!(!g("net:https://*.example.com").covers(&u("https://example.com.evil.net/x")));
        assert!(!g("net:https://*.example.com").covers(&u("https://badexample.com/x")));
        assert!(g("net:http://127.0.0.1:8188").covers(&u("http://127.0.0.1:8188/prompt")));
        assert!(!g("net:http://127.0.0.1:8188").covers(&u("http://127.0.0.1:9999/")));
        assert!(parse_grant("storage").is_none());
        assert!(parse_grant("net:ftp://x.com").is_none());
    }

    #[test]
    fn private_addresses_only_through_a_grant_that_names_them() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        // Private addresses built from their octets: made-up test values.
        let private = |a: u8, b: u8, c: u8, d: u8| IpAddr::from([a, b, c, d]);
        let named = parse_grant("net:http://127.0.0.1:8188").unwrap();
        assert!(named.may_reach(ip("127.0.0.1")));
        assert!(!named.may_reach(private(10, 0, 0, 5)));
        let local = parse_grant("net:http://localhost:3000").unwrap();
        assert!(local.may_reach(ip("127.0.0.1")) && local.may_reach(ip("::1")));
        assert!(!local.may_reach(private(192, 168, 1, 2)));
        let lan_ip = private(192, 168, 1, 10);
        let lan = parse_grant(&format!("net:http://{lan_ip}:8080")).unwrap();
        assert!(lan.may_reach(lan_ip));
        // A public name that resolves somewhere private: refused.
        let public = parse_grant("net:https://api.example.com").unwrap();
        assert!(public.may_reach(ip("93.184.216.34")));
        assert!(!public.may_reach(ip("169.254.169.254")));
        assert!(!public.may_reach(ip("127.0.0.1")));
        assert!(!parse_grant("net:https://*.example.com").unwrap().may_reach(private(10, 1, 2, 3)));
        assert!(!parse_grant("net:http://0.0.0.0:80").unwrap().may_reach(ip("0.0.0.0")));
    }

    /// A one-shot HTTP server on 127.0.0.1 answering every request with `reply`.
    async fn serve(reply: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        port
    }

    #[tokio::test]
    async fn fetches_through_a_grant_and_refuses_without_one() {
        let port = serve("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").await;
        let url = format!("http://127.0.0.1:{port}/x");
        let granted = vec![format!("net:http://127.0.0.1:{port}")];
        let resp = fetch("Test", &granted, FetchRequest { url: url.clone(), ..Default::default() }).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body.as_deref(), Some("{\"ok\":true}"));
        assert_eq!(resp.headers.get("content-type").map(String::as_str), Some("application/json"));

        let err = fetch("Test", &[], FetchRequest { url: url.clone(), ..Default::default() }).await.unwrap_err();
        assert_eq!(err.code, 1001);
        assert_eq!(err.data, Some(serde_json::json!({ "permission": format!("net:http://127.0.0.1:{port}") })));
        // A grant for a public name doesn't reach loopback through it.
        let err = fetch("Test", &["net:https://*.example.com".to_string()], FetchRequest { url, ..Default::default() }).await.unwrap_err();
        assert_eq!(err.code, 1001);
    }

    #[tokio::test]
    async fn a_redirect_must_stay_within_the_grants() {
        let target = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi").await;
        let reply: &'static str = Box::leak(
            format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target}/there\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_boxed_str(),
        );
        let first = serve(reply).await;
        let url = format!("http://127.0.0.1:{first}/");
        let only_first = vec![format!("net:http://127.0.0.1:{first}")];
        let err = fetch("Test", &only_first, FetchRequest { url: url.clone(), ..Default::default() }).await.unwrap_err();
        assert_eq!(err.code, 1004);
        let both = vec![format!("net:http://127.0.0.1:{first}"), format!("net:http://127.0.0.1:{target}")];
        let resp = fetch("Test", &both, FetchRequest { url, ..Default::default() }).await.unwrap();
        assert_eq!(resp.url, format!("http://127.0.0.1:{target}/there"));
        // No content type: the bytes come back as base64.
        assert_eq!(resp.body_base64.as_deref(), Some("aGk="));
    }

    #[test]
    fn refuses_headers_that_belong_to_the_connection() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt
            .block_on(fetch(
                "Test",
                &["net:https://api.example.com".to_string()],
                FetchRequest { url: "https://api.example.com/".into(), headers: BTreeMap::from([("Host".into(), "evil".into())]), ..Default::default() },
            ))
            .unwrap_err();
        assert_eq!(err.code, -32602);
    }
}
