// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One request through the dev proxy to a connection already open to the dev server, WebSocket
//! upgrades included (Vite's hot reload is one).

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};

/// Headers that describe one connection, not the message. A WebSocket handshake needs `Connection`
/// and `Upgrade` kept, and only that.
const HOP_BY_HOP: &[&str] = &["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade"];

fn wants_upgrade(headers: &HeaderMap) -> bool {
    let has = |name: HeaderName, token: &str| {
        headers.get_all(name).iter().filter_map(|v| v.to_str().ok()).any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    };
    has(header::CONNECTION, "upgrade") && headers.contains_key(header::UPGRADE)
}

fn copy_headers(from: &HeaderMap, to: &mut HeaderMap, keep_upgrade: bool) {
    for (name, value) in from {
        if name == header::HOST || (HOP_BY_HOP.contains(&name.as_str()) && !(keep_upgrade && (name == header::CONNECTION || name == header::UPGRADE))) {
            continue;
        }
        to.append(name.clone(), value.clone());
    }
}

/// Send `req` over `backend` and give back the dev server's answer. `host` is what the dev server is
/// told it is being asked for (the proxy's own `Host` names the proxy, not the dev server).
pub(super) async fn forward<S>(mut req: Request, backend: S, host: &str) -> Result<Response, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let upgrade = wants_upgrade(req.headers());
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
    let (parts, body) = req.into_parts();

    let mut out = hyper::Request::builder()
        .method(parts.method)
        .uri(parts.uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/"))
        .version(axum::http::Version::HTTP_11);
    let headers = out.headers_mut().expect("builder");
    copy_headers(&parts.headers, headers, upgrade);
    headers.insert(header::HOST, HeaderValue::from_str(host).map_err(|e| e.to_string())?);
    let out = out.body(body).map_err(|e| e.to_string())?;

    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(backend)).await.map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    let mut answer = sender.send_request(out).await.map_err(|e| e.to_string())?;

    if upgrade && answer.status() == StatusCode::SWITCHING_PROTOCOLS {
        let backend_upgrade = hyper::upgrade::on(&mut answer);
        let mut response = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
        copy_headers(answer.headers(), response.headers_mut().expect("builder"), true);
        if let Some(client_upgrade) = client_upgrade {
            tokio::spawn(async move {
                if let (Ok(client), Ok(backend)) = (client_upgrade.await, backend_upgrade.await) {
                    let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(client), &mut TokioIo::new(backend)).await;
                }
            });
        }
        return response.body(Body::empty()).map_err(|e| e.to_string());
    }

    let (answer_parts, answer_body) = answer.into_parts();
    let mut response = Response::builder().status(answer_parts.status);
    copy_headers(&answer_parts.headers, response.headers_mut().expect("builder"), false);
    response.body(Body::new(answer_body)).map_err(|e| e.to_string())
}
