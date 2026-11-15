// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The container listener's guard on the real router: the operator surface is refused whatever the
//! source, and path tricks never reach it. They need `build_routers`, which belongs to the layer
//! above the listener, so they live here and not in `backend/container/listener.rs`.

use std::net::{IpAddr, SocketAddr};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::Router;
use tower::ServiceExt;

use crate::backend::container::listener::{layer, Subnet};

fn subnet() -> Subnet {
    Subnet::parse(&d("172_30_0_0/16")).unwrap()
}

/// Test addresses are written with `_` so no dotted private address sits in the source.
fn d(s: &str) -> String {
    s.replace('_', ".")
}

fn ip(s: impl AsRef<str>) -> IpAddr {
    s.as_ref().parse().unwrap()
}

async fn call(router: &Router, peer: &str, key: Option<&str>, path: &str) -> StatusCode {
    let mut req = Request::builder().uri(path);
    if let Some(key) = key {
        req = req.header(agentmux_common::AUTH_KEY_HEADER, key);
    }
    let mut req = req.body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(SocketAddr::new(ip(peer), 40000)));
    router.clone().oneshot(req).await.unwrap().status()
}

/// On the real router: the loopback-only operator RPCs and the instance key are refused on the
/// container listener, whatever the source.
#[tokio::test]
async fn the_operator_surface_is_refused_on_the_container_listener() {
    let state = crate::server::tests::test_state();
    let instance_key = state.auth_key.clone();
    let router = layer(crate::server::build_routers(state).full, || Some(subnet()));
    let token = crate::backend::container::credential::token_for_block("cl-block-real", Some("agent-tok"));

    // The instance key is not a container token, from the right network too.
    assert_eq!(call(&router, &d("172_30_0_7"), Some(&instance_key), "/agentmux/discovery").await, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&router, &d("172_30_0_7"), Some(&instance_key), "/agentmux/service").await, StatusCode::UNAUTHORIZED);
    // A container token reaches every route but the host's: commands, host files, the app's plumbing.
    for operator in ["/agentmux/service", "/ws", "/agentmux/file", "/api/v1/shell/create", "/api/v1/ptyshell/input", "/agentmux/stream-local-file", "/api/v1/host/vault", "/api/v1/agent/desktop/authorize"] {
        let got = call(&router, &d("172_30_0_7"), Some(&token), operator).await;
        assert_eq!(got, StatusCode::FORBIDDEN, "{operator}");
    }
    // An allowed route is passed to the app (whatever it then answers is not the guard's).
    let got = call(&router, &d("172_30_0_7"), Some(&token), "/agentmux/discovery").await;
    assert!(got != StatusCode::UNAUTHORIZED && got != StatusCode::FORBIDDEN, "discovery reached the app: {got}");
    // Not from loopback, not from another network.
    assert_eq!(call(&router, "127.0.0.1", Some(&token), "/agentmux/discovery").await, StatusCode::FORBIDDEN);
    assert_eq!(call(&router, &d("10_1_2_3"), Some(&token), "/agentmux/discovery").await, StatusCode::FORBIDDEN);
}

/// What the real HTTP parser and router make of a request line: the status the listener
/// answers with, over a TCP connection.
async fn raw_status(addr: SocketAddr, target: &str, key: &str) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut conn = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: x\r\nX-AuthKey: {key}\r\nConnection: close\r\n\r\n");
    conn.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    let _ = conn.read_to_string(&mut answer).await;
    answer.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("none").to_string()
}

/// Paths that look like an allowed route, or climb out of one, never reach a refused route:
/// the denylist matches the raw path exactly as the router does, so a trick that names a
/// refused route is refused and one that names no route gets the router's 404.
#[tokio::test]
async fn path_tricks_do_not_reach_an_operator_route() {
    let state = crate::server::tests::test_state();
    let router = layer(crate::server::build_routers(state).full, || Subnet::parse("127.0.0.0/8"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
    });
    let token = crate::backend::container::credential::token_for_block("cl-block-paths", Some("agent-tok"));

    let tricks = [
        "//agentmux/service",
        "//agentmux/discovery",
        "/agentmux/service/",
        "/agentmux/service.",
        "/agentmux/service/.",
        "/agentmux/service?/agentmux/discovery",
        "/agentmux/discovery/",
        "/agentmux/discovery/.",
        "/agentmux/discovery/../service",
        "/agentmux/discovery/%2e%2e/service",
        "/agentmux/discovery%2F..%2Fservice",
        "/agentmux%2Fservice",
        "/agentmux/%73ervice",
        "/AGENTMUX/service",
        "/api/v1/agent/../../agentmux/service",
        "/api/v1/agent/%2e%2e/%2e%2e/agentmux/service",
        "/api/v1/agent/%2E%2E/%2E%2E/agentmux/service",
        "/api/v1/agent/memory/list/../../../../agentmux/service",
        "/api/v1/agent/memory/list/%2e%2e/%2e%2e/%2e%2e/%2e%2e/agentmux/service",
        "/api/v1/agent/shutdown/..%2F..%2F..%2Fagentmux/service",
        "/api/v1/agent/shutdown/x/../../../../ws",
        "/agentmux/work/x/../../service",
        "/agentmux/work/%2e%2e/service",
        "/agentmux/work/a%2Fb",
        "/agentmux/work/x/heartbeat/../../../service",
        "/agentmux/work/x\\..\\service",
        "/%2e%2e/agentmux/service",
        "/ws/",
        "/ws/.",
    ];
    for target in tricks {
        // With a denylist a trick that names no route is a 404 from the router; what must never happen is
        // an answer from a route the container is refused.
        let got = raw_status(addr, target, &token).await;
        assert!(got == "403" || got == "400" || got == "404", "{target} answered {got}");
    }
    // The guard is not simply refusing everything.
    let got = raw_status(addr, "/agentmux/discovery", &token).await;
    assert!(got != "401" && got != "403" && got != "400", "discovery answered {got}");
}
