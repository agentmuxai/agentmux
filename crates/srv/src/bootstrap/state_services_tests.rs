// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What the server answers to a request for `/peer`: the address it says the request came from.
async fn peer_seen_on(listener_addr: std::net::SocketAddr) -> (String, std::net::SocketAddr) {
    let mut stream = tokio::net::TcpStream::connect(listener_addr).await.unwrap();
    let local = stream.local_addr().unwrap();
    stream.write_all(b"GET /peer HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    (reply, local)
}

/// A request to either listener carries the address of the connection it came on.
#[tokio::test]
async fn both_listeners_tell_a_route_where_a_request_came_from() {
    use axum::extract::ConnectInfo;
    let router = axum::Router::new().route(
        "/peer",
        axum::routing::get(|peer: Option<ConnectInfo<std::net::SocketAddr>>| async move { peer.map_or("no peer".to_string(), |p| p.0.to_string()) }),
    );
    let (web, ws) = (TcpListener::bind("127.0.0.1:0").await.unwrap(), TcpListener::bind("127.0.0.1:0").await.unwrap());
    let (web_addr, ws_addr) = (web.local_addr().unwrap(), ws.local_addr().unwrap());
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = tokio::spawn({
        let stop = stop.clone();
        async move { serve_until_shutdown(web, ws, router, &stop).await }
    });
    for addr in [web_addr, ws_addr] {
        let (reply, local) = peer_seen_on(addr).await;
        assert!(reply.ends_with(&local.to_string()), "the request says it came from {local}: {reply}");
    }
    stop.cancel();
    serving.await.unwrap();
}
