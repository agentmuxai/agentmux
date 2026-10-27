// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Opens a WebSocket like `connect_async_tls_with_config`, but through the
//! HTTP proxy named by `HTTPS_PROXY` (`HTTP_PROXY` for `ws://`, then
//! `ALL_PROXY`) when there is one, honouring `NO_PROXY`. The proxy only
//! carries bytes: we send `CONNECT host:port`, and the TLS handshake then runs
//! over the tunnel against the real host, so the server name and the
//! certificate checks are the host's, never the proxy's. srv's HTTP clients
//! read the same variables (reqwest); this makes the live MuxBus connection
//! behave the same, and lets the fault proxy of `scripts/net-fault` take it
//! down (SPEC_OFFLINE_RESILIENCE_2026_10_20.md §7.1).
//!
//! Fail closed: when a proxy is configured and cannot be used (its address is
//! not `http://`, it refuses the CONNECT, it does not answer) the connect
//! fails, as a connect that cannot reach the network does; it never goes
//! direct. Credentials in the proxy address (`user:pass@`) are sent as
//! `Proxy-Authorization` and never logged or put in an error.

use std::io;
use std::net::IpAddr;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::UrlError;
use tokio_tungstenite::tungstenite::handshake::client::Response;
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::{client_async_tls_with_config, connect_async_tls_with_config, MaybeTlsStream, WebSocketStream};

/// How long the proxy may take to answer the CONNECT.
const PROXY_TIMEOUT: Duration = Duration::from_secs(15);
/// The longest reply head accepted from a proxy.
const MAX_REPLY_HEAD: usize = 16 * 1024;

pub(crate) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// An `http://` proxy: where it listens and the `Proxy-Authorization` value, if its address held credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proxy {
    host: String,
    port: u16,
    authorization: Option<String>,
}

/// Parses a proxy address. Only `http://` is supported; the error never repeats the address, which may hold a password.
pub(crate) fn parse_proxy(address: &str) -> Result<Proxy, String> {
    const BAD: &str = "the proxy address must look like http://host:port";
    let rest = address.trim();
    let rest = match rest.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("http://") => &rest[7..],
        _ => return Err("only http:// proxies are supported".to_string()),
    };
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (userinfo, hostport) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, rest),
    };
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').ok_or(BAD)?;
        (h, tail.strip_prefix(':'))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if host.is_empty() {
        return Err(BAD.to_string());
    }
    let port = match port {
        Some(p) if !p.is_empty() => p.parse::<u16>().map_err(|_| BAD.to_string())?,
        _ => 80,
    };
    let authorization = userinfo.map(|u| {
        // Basic is `user:password`, with the colon even when the password is empty.
        let mut pair = percent_decode(u);
        if !pair.contains(&b':') {
            pair.push(b':');
        }
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair))
    });
    Ok(Proxy { host: host.to_string(), port, authorization })
}

/// `s` with its `%XX` escapes decoded, as curl and reqwest read the credentials of a proxy address; a `%` not
/// followed by two hex digits stays as it is.
fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Whether `NO_PROXY` (a comma list) exempts `host:port`: `*`; a name, matching itself and its subdomains (a leading
/// `.` or `*.` is the same); an address; a CIDR range; each optionally with `:port`.
pub(crate) fn no_proxy_matches(list: &str, host: &str, port: u16) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    let ip = host.parse::<IpAddr>().ok();
    list.split(',').map(str::trim).filter(|e| !e.is_empty()).any(|entry| {
        if entry == "*" {
            return true;
        }
        let entry = entry.to_ascii_lowercase();
        let (name, wanted_port) = split_entry_port(&entry);
        if wanted_port.is_some_and(|p| p != port) {
            return false;
        }
        if let (Some(ip), Some((net, bits))) = (ip, name.split_once('/')) {
            return net.trim_matches(['[', ']']).parse::<IpAddr>().ok().zip(bits.parse::<u8>().ok()).is_some_and(|(net, bits)| in_range(ip, net, bits));
        }
        let name = name.trim_matches(['[', ']']);
        let name = name.strip_prefix("*.").or_else(|| name.strip_prefix('.')).unwrap_or(name);
        !name.is_empty() && (host == name || host.ends_with(&format!(".{name}")))
    })
}

/// `name` and `:port` of a NO_PROXY entry; a bare IPv6 address has no port.
fn split_entry_port(entry: &str) -> (&str, Option<u16>) {
    if entry.matches(':').count() > 1 && !entry.starts_with('[') {
        return (entry, None);
    }
    match entry.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(']') || entry.starts_with('[') => match port.parse::<u16>() {
            Ok(p) => (name, Some(p)),
            Err(_) => (entry, None),
        },
        _ => (entry, None),
    }
}

fn in_range(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) if bits <= 32 => mask_eq(u128::from(u32::from(a)), u128::from(u32::from(b)), 32, bits),
        (IpAddr::V6(a), IpAddr::V6(b)) if bits <= 128 => mask_eq(u128::from(a), u128::from(b), 128, bits),
        _ => false,
    }
}

fn mask_eq(a: u128, b: u128, width: u32, bits: u8) -> bool {
    let shift = width - u32::from(bits);
    shift >= 128 || (a >> shift) == (b >> shift)
}

/// The proxy for `host:port`, from the environment `get` reads: `Ok(None)` when none is configured or `NO_PROXY` exempts
/// the host, an error (fail closed) when one is configured but cannot be used.
pub(crate) fn proxy_for(get: impl Fn(&str) -> Option<String>, host: &str, port: u16, tls: bool) -> Result<Option<Proxy>, String> {
    let first = |names: &[&str]| names.iter().find_map(|n| get(n).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()));
    let configured = if tls {
        first(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"])
    } else {
        first(&["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"])
    };
    let Some(address) = configured else { return Ok(None) };
    if first(&["NO_PROXY", "no_proxy"]).is_some_and(|list| no_proxy_matches(&list, host, port)) {
        return Ok(None);
    }
    parse_proxy(&address).map(Some)
}

/// Opens a tunnel to `host:port` through the proxy: the CONNECT and its `200`, nothing more. The stream is then the
/// server's own.
async fn tunnel(proxy: &Proxy, host: &str, port: u16) -> io::Result<TcpStream> {
    let authority = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let attempt = async {
        let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port)).await?;
        let mut head = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
        if let Some(auth) = &proxy.authorization {
            head.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).await?;
        // One byte at a time: nothing past the reply head may be consumed, it is the server's.
        let mut reply = Vec::new();
        while !reply.ends_with(b"\r\n\r\n") {
            if reply.len() >= MAX_REPLY_HEAD {
                return Err(io::Error::other("the proxy's reply was too long"));
            }
            reply.push(stream.read_u8().await?);
        }
        let status = String::from_utf8_lossy(&reply);
        let status_line = status.lines().next().unwrap_or("").trim();
        if status_line.split_whitespace().nth(1) == Some("200") {
            Ok(stream)
        } else {
            Err(io::Error::other(format!("the proxy refused CONNECT {authority}: {status_line}")))
        }
    };
    match tokio::time::timeout(PROXY_TIMEOUT, attempt).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, format!("the proxy did not answer CONNECT {authority}"))),
    }
}

/// Connects the WebSocket for `request`, through the environment's proxy when it names one for the request's host.
pub(crate) async fn connect(request: impl IntoClientRequest + Unpin) -> Result<(Socket, Response), Error> {
    connect_with_env(request, |name| std::env::var(name).ok()).await
}

async fn connect_with_env(request: impl IntoClientRequest + Unpin, get: impl Fn(&str) -> Option<String>) -> Result<(Socket, Response), Error> {
    let request = request.into_client_request()?;
    let uri = request.uri();
    // `Uri::host` keeps the brackets of an IPv6 literal; `tunnel` adds them back.
    let host = uri.host().ok_or(Error::Url(UrlError::NoHostName))?.trim_matches(['[', ']']).to_string();
    let tls = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    match proxy_for(get, &host, port, tls) {
        Ok(None) => connect_async_tls_with_config(request, None, false, None).await,
        Ok(Some(proxy)) => {
            tracing::debug!(host = %host, port, proxy_host = %proxy.host, proxy_port = proxy.port, "ws: connecting through the proxy");
            let stream = tunnel(&proxy, &host, port).await.map_err(Error::Io)?;
            client_async_tls_with_config(request, stream, None, None).await
        }
        Err(why) => {
            tracing::warn!("ws: a proxy is configured but cannot be used ({why}); not connecting direct");
            Err(Error::Io(io::Error::other(format!("proxy not usable: {why}"))))
        }
    }
}

#[cfg(test)]
#[path = "ws_connect_tests.rs"]
mod tests;
