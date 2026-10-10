// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Could the daemon pull this image without credentials?", asked of the
//! registry directly, for the create flow.
//!
//! The image name is user data: an imported or custom template can name any
//! registry, and the registry names the token endpoint (`realm`) in its own
//! challenge. So this module treats both as hostile input. It only ever talks
//! to public https hosts: every host it contacts is checked by name and by the
//! addresses it resolves to, the connection is pinned to those addresses, and
//! redirects are never followed. Anything it refuses to contact is `Unknown`,
//! which never blocks the sandbox.
//!
//! Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md section 3.4.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::{Client, Url};

use crate::backend::container_image::{parse_bearer_challenge, parse_image_ref};
use crate::backend::rpc_types::ContainerImageAccess;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// A token response is a few hundred bytes; anything near this is not one.
const MAX_TOKEN_BYTES: usize = 64 * 1024;

const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
     application/vnd.oci.image.manifest.v1+json, \
     application/vnd.docker.distribution.manifest.list.v2+json, \
     application/vnd.docker.distribution.manifest.v2+json";

/// How strictly each leg is checked. Production checks both; tests relax a leg
/// so a local fake server can stand in for it.
#[derive(Clone, Copy)]
struct Policy {
    /// The registry itself: https, and a public host.
    strict_registry: bool,
    /// The token endpoint the registry names: https, and a public host.
    strict_realm: bool,
}

const PRODUCTION: Policy = Policy { strict_registry: true, strict_realm: true };

/// Ask the image's registry, without credentials, whether its manifest can be
/// read: the same question the daemon's pull answers. `Unknown` for anything
/// that isn't a definite answer (offline, a proxy, a host this refuses to
/// contact).
pub async fn probe_anonymous(image: &str) -> ContainerImageAccess {
    probe_with(image, PRODUCTION, None).await
}

async fn probe_with(image: &str, policy: Policy, registry_base: Option<&str>) -> ContainerImageAccess {
    match tokio::time::timeout(PROBE_TIMEOUT, probe_inner(image, policy, registry_base)).await {
        Ok(access) => access,
        Err(_) => ContainerImageAccess::Unknown,
    }
}

async fn probe_inner(image: &str, policy: Policy, registry_base: Option<&str>) -> ContainerImageAccess {
    use ContainerImageAccess::{Denied, NotFound, Public, Unknown};

    let Some(image_ref) = parse_image_ref(image) else { return Unknown };
    let base = match registry_base {
        Some(b) => b.to_string(),
        None => format!("https://{}/", image_ref.registry),
    };
    let Ok(mut manifest_url) = Url::parse(&base) else { return Unknown };
    if manifest_url.host().is_none() || !manifest_url.username().is_empty() || manifest_url.password().is_some() {
        return Unknown;
    }
    {
        let Ok(mut segments) = manifest_url.path_segments_mut() else { return Unknown };
        segments.clear().push("v2");
        for part in image_ref.repo.split('/') {
            segments.push(part);
        }
        segments.push("manifests").push(&image_ref.reference);
    }

    let Some(client) = guarded_client(&manifest_url, policy.strict_registry).await else { return Unknown };
    let first = match client.get(manifest_url.clone()).header("Accept", MANIFEST_ACCEPT).send().await {
        Ok(r) => r,
        Err(_) => return Unknown,
    };
    match first.status().as_u16() {
        200 => return Public,
        404 => return NotFound,
        401 => {}
        403 => return Denied,
        // Includes a redirect: it is never followed.
        _ => return Unknown,
    }

    let challenge = first
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer_challenge);
    let Some(challenge) = challenge else { return Denied };

    let Ok(mut token_url) = validate_realm(&challenge.realm, policy.strict_realm) else { return Unknown };
    {
        let mut query = token_url.query_pairs_mut();
        if let Some(service) = &challenge.service {
            query.append_pair("service", service);
        }
        let scope = challenge.scope.clone().unwrap_or_else(|| format!("repository:{}:pull", image_ref.repo));
        query.append_pair("scope", &scope);
    }
    let Some(token_client) = guarded_client(&token_url, policy.strict_realm).await else { return Unknown };
    let token = match token_client.get(token_url).send().await {
        Ok(r) if r.status().is_success() => read_capped(r, MAX_TOKEN_BYTES).await.and_then(|body| {
            let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
            v.get("token")
                .or_else(|| v.get("access_token"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
        }),
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) => return Denied,
        _ => None,
    };
    let Some(token) = token else { return Unknown };

    match client
        .get(manifest_url)
        .header("Accept", MANIFEST_ACCEPT)
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(r) => match r.status().as_u16() {
            200 => Public,
            401 | 403 => Denied,
            404 => NotFound,
            _ => Unknown,
        },
        Err(_) => Unknown,
    }
}

/// The body of `resp`, or `None` if it is larger than `max` bytes.
async fn read_capped(mut resp: reqwest::Response, max: usize) -> Option<Vec<u8>> {
    if resp.content_length().is_some_and(|len| len > max as u64) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > max {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

/// A client for requests to `url`'s host. Strict: the host is checked by name
/// and by every address it resolves to, and the connection is pinned to those
/// addresses, so the name cannot resolve differently between the check and the
/// connection. Never follows redirects. `None` when the host is refused.
async fn guarded_client(url: &Url, strict: bool) -> Option<Client> {
    let mut builder = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if strict {
        if url.scheme() != "https" {
            return None;
        }
        let port = url.port_or_known_default().unwrap_or(443);
        match url.host()? {
            url::Host::Ipv4(ip) => {
                if ip_is_blocked(IpAddr::V4(ip)) {
                    return None;
                }
            }
            url::Host::Ipv6(ip) => {
                if ip_is_blocked(IpAddr::V6(ip)) {
                    return None;
                }
            }
            url::Host::Domain(name) => {
                if hostname_is_blocked(name) {
                    return None;
                }
                let resolved: Vec<SocketAddr> = tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((name, port)))
                    .await
                    .ok()?
                    .ok()?
                    .collect();
                if !all_public(&resolved) {
                    return None;
                }
                builder = builder.resolve_to_addrs(name, &resolved);
            }
        }
    }
    builder.build().ok()
}

/// A token endpoint is only used if it is an https URL, with no credentials in
/// it and no unusual port, to a host that is not a local or private name or
/// address. `strict = false` is for tests only.
fn validate_realm(realm: &str, strict: bool) -> Result<Url, &'static str> {
    let url = Url::parse(realm).map_err(|_| "not a URL")?;
    if !strict {
        return Ok(url);
    }
    if url.scheme() != "https" {
        return Err("not https");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("has credentials");
    }
    if url.port().is_some_and(|p| p != 443) {
        return Err("unusual port");
    }
    match url.host().ok_or("no host")? {
        url::Host::Ipv4(ip) if ip_is_blocked(IpAddr::V4(ip)) => Err("private address"),
        url::Host::Ipv6(ip) if ip_is_blocked(IpAddr::V6(ip)) => Err("private address"),
        url::Host::Domain(name) if hostname_is_blocked(name) => Err("local name"),
        _ => Ok(url),
    }
}

/// True when there is at least one address and none is private or local.
fn all_public(addrs: &[SocketAddr]) -> bool {
    !addrs.is_empty() && addrs.iter().all(|a| !ip_is_blocked(a.ip()))
}

/// Names that mean "this machine or this network", not a public registry.
fn hostname_is_blocked(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() || !name.contains('.') {
        return true;
    }
    [".localhost", ".local", ".internal", ".localdomain", ".home.arpa", ".lan"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    Ipv4Addr::new((high >> 8) as u8, high as u8, (low >> 8) as u8, low as u8)
}

/// Addresses that are not a public host: loopback, private, link-local (cloud
/// metadata), carrier-grade NAT, unspecified, multicast, reserved and
/// documentation ranges, and IPv6 forms that carry an IPv4 address. Also
/// the widget fetch's test (`widget_net.rs`).
pub(crate) fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_blocked(v4),
        IpAddr::V6(v6) => ipv6_is_blocked(v6),
    }
}

fn ipv4_is_blocked(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xC0) == 64)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xFE) == 18)
        || o[0] >= 240
}

fn ipv6_is_blocked(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_is_blocked(v4);
    }
    let s = ip.segments();
    let first_six_zero = s[..6].iter().all(|&x| x == 0);
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    // ::a.b.c.d (IPv4-compatible), 64:ff9b::/96 (NAT64), 2002::/16 (6to4).
    if first_six_zero {
        return true;
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
        return ipv4_is_blocked(embedded_v4(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return ipv4_is_blocked(embedded_v4(s[1], s[2]));
    }
    (s[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
        || (s[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (s[0] & 0xffc0) == 0xfec0 // fec0::/10 site-local (deprecated)
        || (s[0] == 0x2001 && s[1] == 0) // Teredo
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0) // discard-only
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_local_and_reserved_v4_addresses_are_blocked() {
        for a in [
            "127.0.0.1", "127.255.255.254",
            "169.254.169.254", "169.254.0.1", "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3",
            "255.255.255.255", "224.0.0.1", "192.0.0.8", "192.0.2.1", "198.18.0.1", "198.19.255.255",
            "240.0.0.1",
        ] {
            assert!(ip_is_blocked(ip(a)), "{a} must be blocked");
        }
    }

    #[test]
    fn the_rfc1918_private_ranges_are_blocked() {
        // Built from octets so the test names no literal private address.
        for (a, b, c, d) in [(10, 0, 0, 1), (10, 255, 255, 255), (172, 16, 0, 1), (172, 31, 255, 255), (192, 168, 0, 1), (192, 168, 255, 255)] {
            assert!(ip_is_blocked(IpAddr::V4(Ipv4Addr::new(a, b, c, d))), "{a}.{b}.{c}.{d}");
        }
    }

    #[test]
    fn public_v4_addresses_pass() {
        for a in ["140.82.112.34", "8.8.8.8", "52.1.2.3", "100.63.255.255", "100.128.0.1", "172.15.0.1", "172.32.0.1", "198.17.0.1", "198.20.0.1"] {
            assert!(!ip_is_blocked(ip(a)), "{a} must pass");
        }
    }

    #[test]
    fn private_local_and_embedding_v6_addresses_are_blocked() {
        for a in [
            "::1", "::", "fe80::1", "febf::1", "fc00::1", "fd12:3456::1", "ff02::1", "fec0::1", "2001:db8::1", "2001:0:4136:e378::1",
            "::ffff:127.0.0.1", "::ffff:a01:203", "::ffff:169.254.169.254", "::127.0.0.1", "::a00:1",
            "64:ff9b::7f00:1", "64:ff9b::a9fe:a9fe", "2002:7f00:1::1", "2002:a9fe:a9fe::1", "100::1",
        ] {
            assert!(ip_is_blocked(ip(a)), "{a} must be blocked");
        }
    }

    #[test]
    fn public_v6_addresses_pass() {
        for a in ["2606:4700:4700::1111", "2a00:1450:4001:81b::200e", "::ffff:8.8.8.8", "64:ff9b::808:808", "2002:808:808::1"] {
            assert!(!ip_is_blocked(ip(a)), "{a} must pass");
        }
    }

    #[test]
    fn local_names_are_blocked_and_registry_names_pass() {
        for n in [
            "localhost", "LOCALHOST", "localhost.", "foo.localhost", "printer.local", "metadata.google.internal",
            "host.internal", "box.localdomain", "router.home.arpa", "nas.lan", "intranet", "", ".",
        ] {
            assert!(hostname_is_blocked(n), "{n:?} must be blocked");
        }
        for n in ["ghcr.io", "auth.docker.io", "quay.io", "gcr.io", "registry-1.docker.io", "us-docker.pkg.dev", "public.ecr.aws"] {
            assert!(!hostname_is_blocked(n), "{n} must pass");
        }
    }

    #[test]
    fn every_resolved_address_must_be_public() {
        let sa = |s: &str| SocketAddr::new(ip(s), 443);
        assert!(all_public(&[sa("140.82.112.34"), sa("2606:4700::1")]));
        assert!(!all_public(&[sa("140.82.112.34"), SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)), 443)]), "one private answer is enough to refuse");
        assert!(!all_public(&[sa("::1")]));
        assert!(!all_public(&[]), "no answer is no answer");
    }

    #[test]
    fn the_real_registries_token_endpoints_are_accepted() {
        for realm in [
            "https://ghcr.io/token",
            "https://auth.docker.io/token",
            "https://quay.io/v2/auth",
            "https://gcr.io/v2/token",
            "https://us-docker.pkg.dev/v2/token",
            "https://ghcr.io:443/token",
        ] {
            assert!(validate_realm(realm, true).is_ok(), "{realm} must be accepted");
        }
    }

    #[test]
    fn a_token_endpoint_must_be_https_public_and_plain() {
        let cases = [
            ("http://ghcr.io/token", "not https"),
            ("ftp://ghcr.io/token", "not https"),
            ("https://user:pw@ghcr.io/token", "has credentials"),
            ("https://ghcr.io:8443/token", "unusual port"),
            ("https://127.0.0.1/token", "private address"),
            ("https://[::1]/token", "private address"),
            ("https://[::ffff:127.0.0.1]/token", "private address"),
            ("https://[fd00::1]/token", "private address"),
            ("https://169.254.169.254/latest/meta-data", "private address"),
            ("https://0xa000007/token", "private address"),
            ("https://100.64.1.1/token", "private address"),
            ("https://2130706433/token", "private address"),
            ("https://0x7f.1/token", "private address"),
            ("https://localhost/token", "local name"),
            ("https://metadata.google.internal/token", "local name"),
            ("https://registry.local/token", "local name"),
            ("https://intranet/token", "local name"),
            ("not a url", "not a URL"),
            ("", "not a URL"),
        ];
        for (realm, why) in cases {
            assert_eq!(validate_realm(realm, true).err(), Some(why), "{realm}");
        }
    }

    #[tokio::test]
    async fn a_strict_client_refuses_private_and_local_hosts_without_connecting() {
        for u in [
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://169.254.169.254/",
            "https://localhost/",
            "https://metadata.google.internal/",
            "http://ghcr.io/",
        ] {
            assert!(guarded_client(&Url::parse(u).unwrap(), true).await.is_none(), "{u}");
        }
        assert!(guarded_client(&Url::parse("http://127.0.0.1/").unwrap(), false).await.is_some());
    }

    #[tokio::test]
    async fn an_image_on_a_private_registry_host_is_not_probed() {
        use ContainerImageAccess::Unknown;
        for image in ["127.0.0.1/o/r:1", "169.254.169.254/latest/meta-data:x", "0xa.0.0.5/o/r:1", "localhost.localdomain/o/r:1", "registry.internal/o/r:1"] {
            assert_eq!(probe_anonymous(image).await, Unknown, "{image}");
        }
    }

    /// A tiny HTTP server: `respond` maps the request head to a full response.
    /// Returns its address and a counter of requests served.
    async fn serve<F>(respond: F) -> (SocketAddr, Arc<AtomicUsize>)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let respond = Arc::new(respond);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let counter = counter.clone();
                let respond = respond.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut head = String::new();
                    while !head.contains("\r\n\r\n") {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.push_str(&String::from_utf8_lossy(&buf[..n])),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let _ = sock.write_all(respond(&head).as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (addr, hits)
    }

    fn http(status: &str, headers: &[&str], body: &str) -> String {
        let mut out = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
        for h in headers {
            out.push_str(h);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out.push_str(body);
        out
    }

    const FAKE_REGISTRY: Policy = Policy { strict_registry: false, strict_realm: true };
    const FAKE_BOTH: Policy = Policy { strict_registry: false, strict_realm: false };

    #[tokio::test]
    async fn a_token_endpoint_on_loopback_is_never_requested() {
        let (victim, victim_hits) = serve(|_| http("200 OK", &[], r#"{"token":"x"}"#)).await;
        let realm = format!("http://{victim}/token");
        let challenge = format!("www-authenticate: Bearer realm=\"{realm}\",service=\"s\"");
        let (registry, registry_hits) = serve(move |_| http("401 Unauthorized", &[&challenge], "")).await;

        let got = probe_with("example.com/o/r:1", FAKE_REGISTRY, Some(&format!("http://{registry}/"))).await;
        assert_eq!(got, ContainerImageAccess::Unknown, "a refused token endpoint never blocks the sandbox");
        assert_eq!(registry_hits.load(Ordering::SeqCst), 1);
        assert_eq!(victim_hits.load(Ordering::SeqCst), 0, "no request reached the token endpoint");
    }

    #[tokio::test]
    async fn a_redirect_to_a_private_address_is_not_followed() {
        let (victim, victim_hits) = serve(|_| http("200 OK", &[], "secret")).await;
        let location = format!("location: http://{victim}/internal");
        let (registry, registry_hits) = serve(move |_| http("302 Found", &[&location], "")).await;

        let got = probe_with("example.com/o/r:1", FAKE_REGISTRY, Some(&format!("http://{registry}/"))).await;
        assert_eq!(got, ContainerImageAccess::Unknown);
        assert_eq!(registry_hits.load(Ordering::SeqCst), 1);
        assert_eq!(victim_hits.load(Ordering::SeqCst), 0, "the redirect target was never requested");
    }

    #[tokio::test]
    async fn a_redirecting_token_endpoint_is_not_followed_either() {
        let (victim, victim_hits) = serve(|_| http("200 OK", &[], r#"{"token":"x"}"#)).await;
        let location = format!("location: http://{victim}/token");
        let (idp, _) = serve(move |_| http("307 Temporary Redirect", &[&location], "")).await;
        let challenge = format!("www-authenticate: Bearer realm=\"http://{idp}/token\"");
        let (registry, _) = serve(move |_| http("401 Unauthorized", &[&challenge], "")).await;

        let got = probe_with("example.com/o/r:1", FAKE_BOTH, Some(&format!("http://{registry}/"))).await;
        assert_eq!(got, ContainerImageAccess::Unknown);
        assert_eq!(victim_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_token_flow_still_reaches_a_verdict() {
        let (addr, _) = serve_self_realm().await;
        let base = format!("http://{addr}/");
        assert_eq!(probe_with("example.com/o/ok:1", FAKE_BOTH, Some(&base)).await, ContainerImageAccess::Public);
        assert_eq!(probe_with("example.com/denied/x:1", FAKE_BOTH, Some(&base)).await, ContainerImageAccess::Denied);
        assert_eq!(probe_with("example.com/missing/x:1", FAKE_BOTH, Some(&base)).await, ContainerImageAccess::NotFound);
    }

    /// A registry that sends its own address as the token endpoint.
    async fn serve_self_realm() -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut head = String::new();
                    while !head.contains("\r\n\r\n") {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.push_str(&String::from_utf8_lossy(&buf[..n])),
                        }
                    }
                    let first = head.lines().next().unwrap_or("").to_string();
                    let reply = if first.contains("/token") {
                        http("200 OK", &[], r#"{"token":"t0k"}"#)
                    } else if head.to_ascii_lowercase().contains("authorization: bearer t0k") {
                        if first.contains("/denied/") {
                            http("403 Forbidden", &[], "")
                        } else if first.contains("/missing/") {
                            http("404 Not Found", &[], "")
                        } else {
                            http("200 OK", &[], "{}")
                        }
                    } else {
                        let challenge = format!("www-authenticate: Bearer realm=\"http://{addr}/token\",service=\"svc\"");
                        http("401 Unauthorized", &[&challenge], "")
                    };
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (addr, hits)
    }

    #[tokio::test]
    async fn an_oversized_token_response_is_not_read() {
        let big = "x".repeat(MAX_TOKEN_BYTES + 1024);
        let body = format!(r#"{{"token":"{big}"}}"#);
        let (idp, _) = serve(move |_| http("200 OK", &[], &body)).await;
        let challenge = format!("www-authenticate: Bearer realm=\"http://{idp}/token\"");
        let (registry, _) = serve(move |_| http("401 Unauthorized", &[&challenge], "")).await;

        let got = probe_with("example.com/o/r:1", FAKE_BOTH, Some(&format!("http://{registry}/"))).await;
        assert_eq!(got, ContainerImageAccess::Unknown);
    }

    /// Hits the real registries. Run by hand:
    /// `cargo test -p agentmux-srv --bin agentmux-srv -- --ignored probe_real`
    #[tokio::test]
    #[ignore]
    async fn probe_real_registries() {
        let public = probe_anonymous("docker.io/library/alpine:latest").await;
        let ghcr_public = probe_anonymous("ghcr.io/astral-sh/uv:latest").await;
        let quay_public = probe_anonymous("quay.io/prometheus/prometheus:latest").await;
        let gcr_public = probe_anonymous("gcr.io/distroless/static:latest").await;
        let private = probe_anonymous("ghcr.io/agentmuxai/agent-claude:latest").await;
        let missing = probe_anonymous("docker.io/library/agentmux-no-such-image-xyz:latest").await;
        println!("alpine={public:?} uv={ghcr_public:?} prometheus={quay_public:?} distroless={gcr_public:?} agent-claude={private:?} missing={missing:?}");
        assert_eq!(public, ContainerImageAccess::Public);
        assert_eq!(ghcr_public, ContainerImageAccess::Public);
        assert_eq!(quay_public, ContainerImageAccess::Public);
        assert_eq!(gcr_public, ContainerImageAccess::Public);
        assert_eq!(private, ContainerImageAccess::Denied);
        assert!(matches!(missing, ContainerImageAccess::Denied | ContainerImageAccess::NotFound));
    }
}
