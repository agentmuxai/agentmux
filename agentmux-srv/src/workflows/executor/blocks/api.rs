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
//!
//! Output:
//!   ```json
//!   { "status": 200, "body": <parsed-json-or-text>, "headers": { ... } }
//!   ```

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Shared `reqwest::Client` so workflows with multiple API blocks
/// reuse one connection pool instead of building a new pool per
/// request. Per-request timeouts move to the RequestBuilder.
/// (reagent P2 on PR #755.)
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .expect("reqwest client build failed")
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

    let client = http_client();
    let method = reqwest::Method::from_bytes(method_raw.as_bytes())
        .map_err(|e| format!("invalid method `{method_raw}`: {e}"))?;
    let mut req = client
        .request(method, &url)
        .timeout(Duration::from_millis(timeout_ms));
    for (k, v) in &headers_map {
        req = req.header(k, v);
    }
    if let Some(b) = body_resolved {
        if !b.is_empty() {
            req = req.body(b);
        }
    }
    let resp = req.send().await.map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
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
}
