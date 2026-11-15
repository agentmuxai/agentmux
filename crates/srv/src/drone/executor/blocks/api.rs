// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! API block — HTTP request via reqwest. Honors `{{...}}` interpolation
//! in url, headers, and body.
//!
//! Config (`node.data`):
//!   * `method` — "GET" / "POST" / "PUT" / "DELETE" / "PATCH" (default GET)
//!   * `url` — required; supports `{{...}}`
//!   * `headers` — optional `{ name: value }` map; values support `{{...}}`
//!   * `body` — optional string body (POST/PUT/PATCH); supports `{{...}}`
//!   * `auth` — optional `{ vault: "<entry>", as: "bearer" | "header:<Name>" }`: a Vault
//!     entry's value sent as that header. See [`super::api_auth`] for what keeps it on the one
//!     site the drone names; the first call asks the person.
//!
//! Output:
//!   ```json
//!   { "status": 200, "body": <parsed-json-or-text>, "headers": { ... } }
//!   ```
//!
//! A response is read up to [`MAX_RESPONSE_BYTES`]; a longer one is an error.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::FlowNode;

use super::api_auth;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// The most of a response a call reads: a drone's ledger is not where a download goes.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The client for a call with a Vault credential: it follows no redirect, so the credential goes
/// to the one site the drone names and nowhere a response points.
static CREDENTIAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn credential_client() -> &'static reqwest::Client {
    CREDENTIAL_CLIENT.get_or_init(|| crate::http_client::client_with(|b| b.redirect(reqwest::redirect::Policy::none())))
}

/// Shared `reqwest::Client` so drones with multiple API blocks
/// reuse one connection pool instead of building a new pool per
/// request. Per-request timeouts move to the RequestBuilder.
/// (reagent P2 on PR #755.)
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        crate::http_client::client_with(|b| {
            // Re-run SSRF validation on every redirect target —
            // otherwise a public attacker-controlled URL can return
            // `302 Location: http://169.254.169.254/...` and bypass
            // the initial check. (codex P1 on PR #755.)
            b.redirect(reqwest::redirect::Policy::custom(|attempt| {
                if let Err(e) = validate_url_for_safety(attempt.url()) {
                    return attempt.error(format!("redirect blocked: {e}"));
                }
                if attempt.previous().len() >= 10 {
                    return attempt.error("too many redirects");
                }
                attempt.follow()
            }))
        })
    })
}

/// Validate a resolved URL before dispatching it. Phase 1 SSRF
/// protection: rejects non-http(s) schemes and literal-IP hosts that
/// fall into reserved / link-local / private / loopback ranges (the
/// AWS metadata endpoint 169.254.169.254, RFC1918 space, 127.0.0.1,
/// fc00::/7, ::1, etc.). DNS-resolved hostnames are not re-checked
/// post-resolution — that requires a custom `reqwest` resolver and
/// lands as a follow-up issue. See kimi P1 on PR #755.
fn validate_url_safety(url_str: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(url_str)
        .map_err(|e| format!("invalid URL: {e}"))?;
    validate_url_for_safety(&url)
}

/// Same checks as `validate_url_safety` but takes a pre-parsed
/// `reqwest::Url`. The redirect-policy closure receives Url values
/// directly, so this avoids a parse round-trip on every hop.
fn validate_url_for_safety(url: &reqwest::Url) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("URL scheme `{other}` is not allowed (http/https only)")),
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL missing host".to_string())?;
    // IPv6 literals in URLs are bracketed: `https://[::1]/`. Strip the
    // brackets before attempting an IP parse; that path also lets us
    // distinguish a true IPv6 literal from an ambiguous string.
    if let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let v6: Ipv6Addr = inner
            .parse()
            .map_err(|e| format!("invalid IPv6 host `{inner}`: {e}"))?;
        if is_reserved_v6(&v6) {
            return Err(format!("host `{v6}` is a reserved IPv6 address"));
        }
        return Ok(());
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err("host `localhost` is not allowed".to_string());
    }
    if let Ok(v4) = host.parse::<Ipv4Addr>() {
        if is_reserved_v4(&v4) {
            return Err(format!("host `{v4}` is a reserved/private IP"));
        }
    }
    // Otherwise it's a domain name — DNS-resolution-time SSRF
    // validation is out of scope for Phase 1 (see fn doc-comment).
    Ok(())
}

fn is_reserved_v4(v4: &Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.is_multicast()
}

fn is_reserved_v6(v6: &Ipv6Addr) -> bool {
    // Check pure-v6 reserved ranges first. Order matters: `::1` is
    // v6 loopback AND happens to satisfy `to_ipv4()` (lower 32 bits
    // map to 0.0.0.1, a public v4). Catching loopback up here avoids
    // misclassifying it as "public v4 embedded in v6".
    if v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // unique-local fc00::/7 — stable API for this is gated, so
        // check the segment manually.
        || (v6.segments()[0] & 0xfe00) == 0xfc00
        // link-local fe80::/10
        || (v6.segments()[0] & 0xffc0) == 0xfe80
    {
        return true;
    }
    // IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`,
    // deprecated) literals route to the embedded IPv4 address on
    // most kernels, bypassing v6-only checks. Delegate to the v4
    // predicate so private/loopback IPv4 in v6 form is caught.
    // (codex P1 on PR #755.)
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_reserved_v4(&v4);
    }
    #[allow(deprecated)]
    if let Some(v4) = v6.to_ipv4() {
        // `to_ipv4` returns Some for mapped (handled above) AND
        // IPv4-compatible (`::a.b.c.d`). This branch catches the
        // compatible form.
        return is_reserved_v4(&v4);
    }
    false
}

/// What a call with a Vault credential sends it as, and what takes it out of the answer.
struct Credential {
    name: reqwest::header::HeaderName,
    value: reqwest::header::HeaderValue,
    scrubber: crate::backend::vault::scrub::Scrubber,
}

/// Checks and resolves the node's `auth`, asking the person the first time. `None` when the node
/// has none. Nothing is sent, and nothing but the person's answer is waited for.
async fn credential(node: &FlowNode, scope: &ExecutionScope, url_raw: &str, url: &str, headers: &HashMap<String, String>) -> Result<Option<Credential>, String> {
    let Some(auth) = api_auth::parse(&node.data)? else { return Ok(None) };
    if scope.drone_id.trim().is_empty() {
        return Err("a Vault credential is for a saved drone's run".into());
    }
    let site = api_auth::site(url_raw, url)?;
    let name = auth.how.name();
    if api_auth::sets_it_too(headers, &name) {
        return Err(format!("the node sets the {name} header and also takes it from the Vault: leave it out of `headers`"));
    }
    let state = crate::drone::host::state()?;
    let resolved = crate::server::drone_vault::resolve(state, &scope.drone_id, &auth.entry, &site).await?;
    let value = auth.how.value(&resolved.value)?;
    Ok(Some(Credential { name, value, scrubber: resolved.scrubber }))
}

/// `text` with the credential taken out of it, when the call had one.
fn clean(credential: &Option<Credential>, text: String) -> String {
    match credential {
        Some(c) => c.scrubber.scrub(&text).into_owned(),
        None => text,
    }
}

/// The response's body up to [`MAX_RESPONSE_BYTES`].
async fn read_capped(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    if resp.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
        return Err(format!("the response is longer than {} MiB", MAX_RESPONSE_BYTES / (1024 * 1024)));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("read body: {e}"))? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(format!("the response is longer than {} MiB", MAX_RESPONSE_BYTES / (1024 * 1024)));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let method_raw = node
        .data
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase();
    let url_raw = node
        .data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "API block missing `url`".to_string())?;
    let url = scope.resolve(url_raw);
    if url.trim().is_empty() {
        return Err("API block resolved URL is empty".to_string());
    }
    validate_url_safety(&url)?;

    let headers_map: HashMap<String, String> = match node.data.get("headers") {
        Some(Value::Object(obj)) => obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), scope.resolve(s))))
            .collect(),
        _ => HashMap::new(),
    };

    let body_resolved: Option<String> = node
        .data
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| scope.resolve(s));

    let timeout_ms = node
        .data
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    // Before anything is sent: a bad `auth` is an error, never a call with no credential.
    let credential = credential(node, scope, url_raw, &url, &headers_map).await?;

    let client = if credential.is_some() { credential_client() } else { http_client() };
    let method = reqwest::Method::from_bytes(method_raw.as_bytes())
        .map_err(|e| format!("invalid method `{method_raw}`: {e}"))?;
    let mut req = client
        .request(method, &url)
        .timeout(Duration::from_millis(timeout_ms));
    for (k, v) in &headers_map {
        req = req.header(k, v);
    }
    if let Some(c) = &credential {
        req = req.header(c.name.clone(), c.value.clone());
    }
    if let Some(b) = body_resolved {
        if !b.is_empty() {
            req = req.body(b);
        }
    }
    let resp = req.send().await.map_err(|e| clean(&credential, format!("request failed: {e}")))?;
    let status = resp.status().as_u16();
    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), clean(&credential, v.to_str().unwrap_or("").to_string())))
        .collect();
    let bytes = read_capped(resp).await?;
    let text = clean(&credential, String::from_utf8_lossy(&bytes).to_string());
    // Try to parse as JSON for downstream `{{api.body.field}}` access.
    let body_val: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));

    Ok(json!({
        "status": status,
        "body": body_val,
        "headers": resp_headers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssrf_rejects_loopback() {
        assert!(validate_url_safety("http://127.0.0.1/").is_err());
        assert!(validate_url_safety("http://127.0.0.1:8080/x").is_err());
        assert!(validate_url_safety("https://[::1]/").is_err());
    }

    #[test]
    fn ssrf_rejects_localhost_hostname() {
        assert!(validate_url_safety("http://localhost/").is_err());
        assert!(validate_url_safety("http://LocalHost:80/").is_err());
    }

    #[test]
    fn ssrf_rejects_aws_metadata_endpoint() {
        // 169.254.169.254 is link-local (RFC 3927) — covers the cloud
        // metadata endpoints for AWS, GCP, Azure (all use this address
        // or other link-local IPs).
        assert!(validate_url_safety("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn ssrf_rejects_rfc1918_private() {
        assert!(validate_url_safety("http://10.0.0.1/").is_err());
        assert!(validate_url_safety("http://172.16.0.1/").is_err());
        assert!(validate_url_safety("http://192.168.1.1/").is_err());
    }

    #[test]
    fn ssrf_rejects_non_http_schemes() {
        assert!(validate_url_safety("file:///etc/passwd").is_err());
        assert!(validate_url_safety("ftp://example.com/").is_err());
        assert!(validate_url_safety("gopher://example.com/").is_err());
    }

    #[test]
    fn ssrf_rejects_ipv6_unique_local() {
        // fc00::/7 — RFC 4193 unique local addresses.
        assert!(validate_url_safety("http://[fc00::1]/").is_err());
        assert!(validate_url_safety("http://[fd00::1]/").is_err());
    }

    #[test]
    fn ssrf_rejects_ipv4_mapped_ipv6_to_reserved() {
        // ::ffff:a.b.c.d routes to a.b.c.d on most kernels. The v6
        // path must delegate to the v4 reserved check or these slip
        // past the SSRF guard. (codex P1.)
        assert!(validate_url_safety("http://[::ffff:127.0.0.1]/").is_err());
        assert!(validate_url_safety("http://[::ffff:169.254.169.254]/").is_err());
        assert!(validate_url_safety("http://[::ffff:10.0.0.1]/").is_err());
        assert!(validate_url_safety("http://[::ffff:192.168.1.1]/").is_err());
    }

    #[test]
    fn ssrf_rejects_ipv4_compatible_ipv6_to_reserved() {
        // ::a.b.c.d (IPv4-compatible, deprecated form). Defense in
        // depth — some kernels still honor it.
        assert!(validate_url_safety("http://[::127.0.0.1]/").is_err());
    }

    #[test]
    fn ssrf_allows_public_ipv4_in_mapped_form() {
        // Mapped form of a public IP is consistent with allowing the
        // plain form — block only the reserved range, not the wrapper.
        assert!(validate_url_safety("https://[::ffff:8.8.8.8]/").is_ok());
    }

    #[test]
    fn ssrf_allows_public_hostnames() {
        assert!(validate_url_safety("https://api.example.com/v1/users").is_ok());
        assert!(validate_url_safety("http://example.com:8080/").is_ok());
    }

    #[test]
    fn ssrf_allows_public_ip_literal() {
        // 8.8.8.8 is a public DNS address — not reserved.
        assert!(validate_url_safety("https://8.8.8.8/").is_ok());
    }

    fn node(data: Value) -> FlowNode {
        FlowNode { id: "api1".to_string(), position: Default::default(), data, node_type: None }
    }

    fn scope_of(drone: &str) -> ExecutionScope {
        ExecutionScope { drone_id: drone.to_string(), ..ExecutionScope::new() }
    }

    #[tokio::test]
    async fn a_credential_that_cannot_be_used_is_an_error_never_a_call_without_it() {
        let auth = json!({ "vault": "api-key-acme", "as": "bearer" });
        // Not https: the credential would cross in the clear.
        let http = node(json!({ "url": "http://api.example.com/x", "auth": auth }));
        let err = run(&http, &scope_of("nightly")).await.unwrap_err();
        assert!(err.contains("https"), "{err}");
        // The site comes from a run's value.
        let moving = node(json!({ "url": "https://{{trigger.host}}/x", "auth": auth }));
        let scope = ExecutionScope { trigger: json!({ "host": "evil.example.net" }), ..scope_of("nightly") };
        let err = run(&moving, &scope).await.unwrap_err();
        assert!(err.contains("written out"), "{err}");
        // An `auth` the block doesn't understand is not ignored.
        let basic = node(json!({ "url": "https://api.example.com/x", "auth": { "vault": "k", "as": "basic" } }));
        assert!(run(&basic, &scope_of("nightly")).await.is_err());
        // The node can't set the credential's header itself.
        let both = node(json!({ "url": "https://api.example.com/x", "headers": { "Authorization": "Bearer mine" }, "auth": auth }));
        let err = run(&both, &scope_of("nightly")).await.unwrap_err();
        assert!(err.contains("authorization"), "{err}");
        // Not a saved drone's run: nobody to grant it to.
        let ok = node(json!({ "url": "https://api.example.com/x", "auth": auth }));
        assert!(run(&ok, &scope_of("")).await.unwrap_err().contains("saved drone"));
    }

    /// A one-shot HTTP server on a free local port that answers with `head` and `body`.
    async fn serve(head: &'static str, body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.shutdown().await;
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn a_response_is_read_up_to_the_cap_and_no_further() {
        let small = serve("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n", b"hello".to_vec()).await;
        let resp = reqwest::get(&small).await.unwrap();
        assert_eq!(read_capped(resp).await.unwrap(), b"hello");
        // One that says how long it is, too long.
        let long = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_RESPONSE_BYTES + 1);
        let head: &'static str = Box::leak(long.into_boxed_str());
        let url = serve(head, Vec::new()).await;
        let err = read_capped(reqwest::get(&url).await.unwrap()).await.unwrap_err();
        assert!(err.contains("longer than"), "{err}");
        // One that doesn't say, and goes on past it.
        let url = serve("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n", vec![b'x'; MAX_RESPONSE_BYTES + 10]).await;
        let err = read_capped(reqwest::get(&url).await.unwrap()).await.unwrap_err();
        assert!(err.contains("longer than"), "{err}");
    }

    #[test]
    fn what_comes_back_is_cleaned_of_the_credential_when_the_call_had_one() {
        let secret = "sk_live_abcdefgh1234";
        let c = Some(Credential {
            name: reqwest::header::AUTHORIZATION,
            value: reqwest::header::HeaderValue::from_static("x"),
            scrubber: crate::backend::vault::scrub::Scrubber::new(&[("api-key-acme".to_string(), secret.to_string())]),
        });
        assert_eq!(clean(&c, format!("{{\"echo\":\"Bearer {secret}\"}}")), "{\"echo\":\"Bearer [vault:api-key-acme]\"}");
        assert_eq!(clean(&None, format!("Bearer {secret}")), format!("Bearer {secret}"));
    }
}
