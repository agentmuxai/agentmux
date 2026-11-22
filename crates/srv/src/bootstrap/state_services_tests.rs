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
    use crate::server::confirm_peer::Conn;
    use axum::extract::ConnectInfo;
    let router = axum::Router::new().route(
        "/peer",
        axum::routing::get(|plain: Option<ConnectInfo<std::net::SocketAddr>>, both: Option<ConnectInfo<Conn>>| async move {
            let both = both.map_or("no connection".to_string(), |b| format!("{} to {}", b.0.peer, b.0.local.map_or("?".to_string(), |l| l.to_string())));
            format!("{} / {both}", plain.map_or("no peer".to_string(), |p| p.0.to_string()))
        }),
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
        assert!(reply.ends_with(&format!("{local} / {local} to {addr}")), "the request says it came from {local} and arrived at {addr}: {reply}");
    }
    stop.cancel();
    serving.await.unwrap();
}

/// On either listener, the connection's own two ends find the process that holds the client's end: here, this one. (The
/// sign-in's confirm depends on it: the ws listener serves the same routes as the web one.)
#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn a_connection_to_either_listener_is_traced_to_its_process() {
    use crate::server::confirm_peer::Conn;
    use axum::extract::ConnectInfo;
    let router = axum::Router::new().route(
        "/owner",
        axum::routing::get(|conn: ConnectInfo<Conn>| async move {
            conn.0.local.and_then(|local| crate::server::peer_process::owner_of(conn.0.peer, local)).map_or("none".to_string(), |pid| pid.to_string())
        }),
    );
    let (web, ws) = (TcpListener::bind("127.0.0.1:0").await.unwrap(), TcpListener::bind("127.0.0.1:0").await.unwrap());
    let (web_addr, ws_addr) = (web.local_addr().unwrap(), ws.local_addr().unwrap());
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = tokio::spawn({
        let stop = stop.clone();
        async move { serve_until_shutdown(web, ws, router, &stop).await }
    });
    for addr in [web_addr, ws_addr] {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(b"GET /owner HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        assert!(reply.ends_with(&std::process::id().to_string()), "{addr}: {reply}");
    }
    stop.cancel();
    serving.await.unwrap();
}

/// An alert raised while the caller holds the lock the delivery needs returns at once; the line is delivered after
/// the lock is released.
#[test]
fn a_queued_alert_returns_while_the_caller_holds_the_lock_the_delivery_needs() {
    let lock = Arc::new(std::sync::Mutex::new(()));
    let (done, delivered) = std::sync::mpsc::channel::<String>();
    let alert = queued_alert({
        let lock = Arc::clone(&lock);
        move |line| {
            let _held = lock.lock().unwrap();
            let _ = done.send(line);
        }
    });
    let held = lock.lock().unwrap();
    alert("canary-alert-line");
    assert!(delivered.try_recv().is_err(), "nothing is delivered while the lock is held");
    drop(held);
    assert_eq!(delivered.recv_timeout(std::time::Duration::from_secs(10)).unwrap(), "canary-alert-line");
}
