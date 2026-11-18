// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-mcp gateway-fwd`: the gateway forwarder srv runs as a helper container on the agents network (Docker Desktop),
//! so a container agent's model-gateway request carries proof of which container it is from
//! (`agentmux_common::gateway_stamp`; srv's side is `container::gateway_fwd`).
//!
//! It accepts a request only on the gateway's own paths, removes every `x-amx-fwd-*` header the client sent, adds the address
//! the connection came from, the time and a proof made with the secret srv gave it, and streams the request and the answer
//! through unbuffered. It reads no token and holds no key: the agent's gateway token goes through untouched and srv decides
//! on it. Its secret comes from the environment at start and is removed from it; it is never printed.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use agentmux_common::gateway_stamp::{self, HEADER_PEER, HEADER_PREFIX, HEADER_PROOF, HEADER_TS};
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;

const ENV_SECRET: &str = "AGENTMUX_GWFWD_SECRET";
const ENV_UPSTREAM: &str = "AGENTMUX_GWFWD_UPSTREAM";
const ENV_PORT: &str = "AGENTMUX_GWFWD_PORT";
const DEFAULT_PORT: u16 = 4799;

/// Headers that describe one hop of a connection, not the request or answer, so they are not passed on.
const HOP_BY_HOP: &[&str] = &["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade", "host", "content-length"];

#[derive(Clone)]
struct Forwarder {
    client: reqwest::Client,
    /// `http://host:port`, no trailing slash.
    upstream: String,
    secret: Vec<u8>,
}

/// The headers to send srv: the client's own, less the hop's and every stamp header it set, plus this hop's stamp.
fn upstream_headers(from: &HeaderMap, secret: &[u8], peer: IpAddr, ts: u64, method: &str, target: &str) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in from {
        let n = name.as_str();
        if n.starts_with(HEADER_PREFIX) || HOP_BY_HOP.contains(&n) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    let proof = gateway_stamp::proof(secret, peer, ts, method, target);
    for (name, value) in [(HEADER_PEER, peer.to_string()), (HEADER_TS, ts.to_string()), (HEADER_PROOF, proof)] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            out.insert(HeaderName::from_static(name), value);
        }
    }
    out
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    (status, text).into_response()
}

async fn forward(State(fwd): State<Forwarder>, ConnectInfo(from): ConnectInfo<SocketAddr>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let target = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    // Only the gateway's own paths, written plainly: nothing else is stamped, so nothing else can be vouched for.
    if !gateway_stamp::covered_target(&target) {
        return plain(StatusCode::NOT_FOUND, "not a model gateway path");
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let headers = upstream_headers(&parts.headers, &fwd.secret, from.ip(), now, parts.method.as_str(), &target);
    let sent = fwd.client.request(parts.method, format!("{}{}", fwd.upstream, target)).headers(headers).body(reqwest::Body::wrap_stream(body.into_data_stream())).send().await;
    let Ok(answer) = sent else {
        return plain(StatusCode::BAD_GATEWAY, "AgentMux could not be reached");
    };
    let (status, answer_headers) = (answer.status(), answer.headers().clone());
    let mut out = Response::new(Body::from_stream(answer.bytes_stream()));
    *out.status_mut() = status;
    for (name, value) in &answer_headers {
        if !HOP_BY_HOP.contains(&name.as_str()) && name != header::CONTENT_ENCODING {
            out.headers_mut().append(name.clone(), value.clone());
        }
    }
    out
}

fn app(fwd: Forwarder) -> Router {
    Router::new().fallback(forward).with_state(fwd)
}

fn client() -> reqwest::Client {
    agentmux_common::local_http::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(300))
        .build()
        .unwrap_or_else(|e| panic!("the TLS provider is missing, so no HTTP client can be built: {e}"))
}

/// Serves until the process ends. Returns the exit code.
pub async fn run() -> i32 {
    let (Ok(secret), Ok(upstream)) = (std::env::var(ENV_SECRET), std::env::var(ENV_UPSTREAM)) else {
        eprintln!("gateway-fwd: {ENV_SECRET} and {ENV_UPSTREAM} are needed; AgentMux starts this itself");
        return 2;
    };
    // Out of this process's environment from here on, so nothing it starts (it starts nothing) could read it.
    std::env::remove_var(ENV_SECRET);
    let port = std::env::var(ENV_PORT).ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let fwd = Forwarder { client: client(), upstream: upstream.trim_end_matches('/').to_string(), secret: secret.into_bytes() };
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gateway-fwd: could not listen on port {port}: {e}");
            return 1;
        }
    };
    match axum::serve(listener, app(fwd).into_make_service_with_connect_info::<SocketAddr>()).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gateway-fwd: stopped: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::any;
    use std::sync::{Arc, Mutex};

    const SECRET: &[u8] = b"a-test-secret";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (n, v) in pairs {
            h.append(HeaderName::from_bytes(n.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn peer() -> IpAddr {
        "172_30_0_9".replace('_', ".").parse().unwrap()
    }

    #[test]
    fn a_stamp_a_client_sent_is_removed_whatever_its_case_and_the_forwarders_own_is_added() {
        let target = "/api/v1/model/anthropic/v1/messages";
        let sent = headers(&[
            ("Authorization", "Bearer amxg_token"),
            ("X-Amx-Fwd-Peer", "not-the-real-peer"),
            ("x-amx-fwd-ts", "1"),
            ("X-AMX-FWD-PROOF", "00"),
            ("x-amx-fwd-extra", "whatever"),
            ("Host", "agentmux-gateway-fwd:4799"),
            ("Connection", "keep-alive"),
            ("anthropic-version", "2023-06-01"),
        ]);
        let out = upstream_headers(&sent, SECRET, peer(), 1000, "POST", target);
        assert_eq!(out.get("authorization").unwrap(), "Bearer amxg_token", "the token goes through untouched");
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert!(out.get("host").is_none() && out.get("connection").is_none(), "this hop's headers are not passed on");
        assert!(out.get("x-amx-fwd-extra").is_none());
        for name in [HEADER_PEER, HEADER_TS, HEADER_PROOF] {
            assert_eq!(out.get_all(name).iter().count(), 1, "exactly one {name}, the forwarder's");
        }
        let get = |n: &str| out.get(n).and_then(|v| v.to_str().ok());
        assert_eq!(gateway_stamp::verify(SECRET, get(HEADER_PEER), get(HEADER_TS), get(HEADER_PROOF), "POST", target, 1000), Ok(peer()), "the stamp names the connection's own address");
    }

    /// A srv stand-in that records what it was sent and answers in two chunks.
    async fn upstream(seen: Arc<Mutex<Vec<(String, HeaderMap, Vec<u8>)>>>) -> SocketAddr {
        let router = Router::new().fallback(any(move |request: Request<Body>| {
            let seen = seen.clone();
            async move {
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap().to_vec();
                seen.lock().unwrap().push((parts.uri.to_string(), parts.headers, bytes));
                let chunks = futures_chunks();
                let mut answer = Response::new(Body::from_stream(chunks));
                answer.headers_mut().insert("x-answer", HeaderValue::from_static("yes"));
                answer
            }
        }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        addr
    }

    fn futures_chunks() -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> {
        futures_util::stream::iter(vec![Ok(axum::body::Bytes::from_static(b"data: one\n\n")), Ok(axum::body::Bytes::from_static(b"data: two\n\n"))])
    }

    #[tokio::test]
    async fn it_forwards_gateway_paths_with_the_stamp_streams_the_answer_and_refuses_every_other_path() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let srv = upstream(seen.clone()).await;
        let fwd = Forwarder { client: client(), upstream: format!("http://{srv}"), secret: SECRET.to_vec() };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app(fwd).into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
        let http = client();

        let ok = http
            .post(format!("http://{at}/api/v1/model/anthropic/v1/messages?beta=true"))
            .header("authorization", "Bearer amxg_t")
            .header("x-amx-fwd-peer", "10_9_9_9".replace('_', "."))
            .body("{\"model\":\"m\"}")
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.headers().get("x-answer").unwrap(), "yes");
        assert_eq!(ok.text().await.unwrap(), "data: one\n\ndata: two\n\n");
        let (uri, headers, body) = seen.lock().unwrap().pop().expect("srv was asked");
        assert!(uri.ends_with("/api/v1/model/anthropic/v1/messages?beta=true"), "{uri}");
        assert_eq!(body, b"{\"model\":\"m\"}");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer amxg_t");
        let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let named = gateway_stamp::verify(SECRET, get(HEADER_PEER), get(HEADER_TS), get(HEADER_PROOF), "POST", "/api/v1/model/anthropic/v1/messages?beta=true", now);
        assert_eq!(named, Ok(IpAddr::from([127, 0, 0, 1])), "the address of the connection, never the one the client wrote");

        for path in ["/api/v1/containers", "/agentmux/hook/x", "/", "/api/v1/model/../containers", "/api/v1/%6dodel/anthropic/v1/messages", "/API/v1/model/anthropic"] {
            let refused = http.get(format!("http://{at}{path}")).send().await.unwrap();
            assert_eq!(refused.status(), 404, "{path}");
        }
        assert!(seen.lock().unwrap().is_empty(), "none of them reached srv");
    }
}
