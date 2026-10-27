// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::*;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |name| map.get(name).cloned()
}

#[test]
fn parses_an_http_proxy_with_or_without_a_port_and_credentials() {
    let p = parse_proxy("http://127.0.0.1:8899").unwrap();
    assert_eq!((p.host.as_str(), p.port, p.authorization.as_deref()), ("127.0.0.1", 8899, None));
    assert_eq!(parse_proxy("HTTP://proxy.corp/").unwrap().port, 80);
    assert_eq!(parse_proxy("http://[::1]:3128").unwrap().host, "::1");
    // user:pass@ becomes a Basic header: base64("user:pass")
    let p = parse_proxy("http://user:pass@proxy.corp:8080/path").unwrap();
    assert_eq!((p.host.as_str(), p.port), ("proxy.corp", 8080));
    assert_eq!(p.authorization.as_deref(), Some("Basic dXNlcjpwYXNz"));
}

#[test]
fn refuses_other_schemes_and_never_repeats_the_address_in_the_error() {
    for bad in ["https://proxy:443", "socks5://u:secretpw@proxy:1080", "proxy:8080", "", "http://", "http://host:notaport", "http://u:secretpw@:80"] {
        let err = parse_proxy(bad).unwrap_err();
        assert!(!err.contains("secretpw") && !err.contains("proxy:8080"), "{bad}: {err}");
    }
    assert!(parse_proxy("https://proxy:443").unwrap_err().contains("only http://"));
}

#[test]
fn no_proxy_follows_the_usual_rules() {
    let m = |list: &str, host: &str, port: u16| no_proxy_matches(list, host, port);
    assert!(m("*", "anything.example", 443));
    assert!(m("localhost,127.0.0.1", "localhost", 443));
    assert!(!m("", "example.com", 443));
    // a name matches itself and its subdomains, at a dot boundary, with or without a leading dot or *.
    assert!(m("example.com", "example.com", 443));
    assert!(m("example.com", "ws.example.com", 443));
    assert!(m(".example.com", "ws.example.com", 443));
    assert!(m("*.example.com", "ws.example.com", 443));
    assert!(!m("example.com", "badexample.com", 443));
    assert!(!m("ws.example.com", "example.com", 443));
    assert!(m("EXAMPLE.com", "WS.Example.COM", 443));
    // with a port, only that port
    assert!(m("example.com:443", "example.com", 443));
    assert!(!m("example.com:8080", "example.com", 443));
    // spaces and empty entries
    assert!(m(" , other.org , example.com ,", "example.com", 443));
    // addresses and ranges
    assert!(m("203.0.113.5", "203.0.113.5", 80));
    assert!(m("44.0.0.0/8", "44.9.8.7", 80));
    assert!(!m("44.0.0.0/8", "45.0.0.1", 80));
    assert!(m("198.51.100.0/24", "198.51.100.200", 80));
    assert!(!m("198.51.100.0/24", "198.51.101.1", 80));
    assert!(m("::1", "::1", 80));
    assert!(m("fd00::/8", "fd12::1", 80));
    assert!(m("0.0.0.0/0", "8.8.8.8", 80));
    assert!(!m("44.0.0.0/8", "example.com", 80));
}

#[test]
fn picks_the_proxy_by_scheme_and_lets_no_proxy_exempt_a_host() {
    let get = env(&[("HTTPS_PROXY", "http://p:1"), ("HTTP_PROXY", "http://q:2"), ("NO_PROXY", "internal.corp")]);
    assert_eq!(proxy_for(&get, "muxbus-ws.agentmux.ai", 443, true).unwrap().unwrap().host, "p");
    assert_eq!(proxy_for(&get, "muxbus-ws.agentmux.ai", 80, false).unwrap().unwrap().host, "q");
    assert_eq!(proxy_for(&get, "a.internal.corp", 443, true).unwrap(), None);
    // lower case, ALL_PROXY as a fallback, an empty value as none
    assert_eq!(proxy_for(env(&[("https_proxy", "http://low:1")]), "h", 443, true).unwrap().unwrap().host, "low");
    assert_eq!(proxy_for(env(&[("ALL_PROXY", "http://all:1")]), "h", 443, true).unwrap().unwrap().host, "all");
    assert_eq!(proxy_for(env(&[("HTTPS_PROXY", "  ")]), "h", 443, true).unwrap(), None);
    assert_eq!(proxy_for(env(&[]), "h", 443, true).unwrap(), None);
}

#[test]
fn a_configured_proxy_that_cannot_be_used_is_an_error_not_a_direct_connection() {
    let err = proxy_for(env(&[("HTTPS_PROXY", "socks5://u:secretpw@p:1080")]), "h", 443, true).unwrap_err();
    assert!(!err.contains("secretpw"));
}

/// A proxy that answers one CONNECT with `reply`, handing back the head it received and (when it said 200) tunnelling
/// to `upstream` afterwards.
async fn fake_proxy(reply: &'static str, upstream: Option<std::net::SocketAddr>) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(client.read_u8().await.unwrap());
        }
        client.write_all(reply.as_bytes()).await.unwrap();
        if let Some(addr) = upstream {
            let mut up = TcpStream::connect(addr).await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut up).await;
        }
        String::from_utf8(head).unwrap()
    });
    (port, task)
}

#[tokio::test]
async fn sends_connect_with_the_credentials_and_opens_the_websocket_through_the_tunnel() {
    // The server: a plain ws:// echo of one message.
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = server.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
        let msg = ws.next().await.unwrap().unwrap();
        ws.send(msg).await.unwrap();
    });
    let (proxy_port, seen) = fake_proxy("HTTP/1.1 200 Connection Established\r\n\r\n", Some(server_addr)).await;
    let proxy = format!("http://user:pass@127.0.0.1:{proxy_port}");
    // ws:// to a made-up name: only the proxy can reach it, so a direct connect would fail.
    let get = env(&[("HTTP_PROXY", proxy.as_str())]);
    let (mut ws, _) = connect_with_env("ws://relay.invalid:9/socket", &get).await.unwrap();
    ws.send(Message::Text("hello".into())).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap(), Message::Text("hello".into()));
    drop(ws);
    let head = seen.await.unwrap();
    assert!(head.starts_with("CONNECT relay.invalid:9 HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("Host: relay.invalid:9\r\n"), "{head}");
    assert!(head.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"), "{head}");
}

#[tokio::test]
async fn a_proxy_that_refuses_the_connect_fails_the_connect_without_going_direct_or_leaking_the_password() {
    let (proxy_port, _) = fake_proxy("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n", None).await;
    let proxy = format!("http://user:secretpw@127.0.0.1:{proxy_port}");
    let get = env(&[("HTTP_PROXY", proxy.as_str())]);
    let err = connect_with_env("ws://relay.invalid:9/socket", &get).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("407"), "{text}");
    assert!(!text.contains("secretpw"), "{text}");
    assert!(matches!(err, Error::Io(_)), "{text}");
}

#[tokio::test]
async fn an_unusable_proxy_setting_fails_closed() {
    let get = env(&[("HTTP_PROXY", "socks5://127.0.0.1:1080")]);
    let err = connect_with_env("ws://127.0.0.1:9/socket", &get).await.unwrap_err();
    assert!(matches!(err, Error::Io(_)));
    assert!(err.to_string().contains("only http://"), "{err}");
}

#[tokio::test]
async fn a_host_in_no_proxy_connects_direct() {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (sock, _) = server.accept().await.unwrap();
        let _ = tokio_tungstenite::accept_async(sock).await;
    });
    // The proxy named is not listening: using it would fail.
    let get = env(&[("HTTP_PROXY", "http://127.0.0.1:1"), ("NO_PROXY", "127.0.0.1")]);
    let (ws, _) = connect_with_env(format!("ws://127.0.0.1:{port}/socket"), &get).await.unwrap();
    drop(ws);
}
