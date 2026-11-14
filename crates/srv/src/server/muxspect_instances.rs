// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `muxspect instances` and `muxspect --channel <name> <command>`: look at another AgentMux
//! instance on this machine from inside one
//! (`docs/specs/SPEC_MUXSPECT_CROSS_TIER_INSTANCE_INSPECTION_2026_09_02.md` §2.3, §2.4).
//!
//! `GET /api/v1/muxspect/instances` lists the live instances from `backend::instance_registry`.
//! [`forward_to_channel`] is a layer on muxspect's read routes: a request carrying `?channel=<name>`
//! is sent, as it is minus that parameter, to that instance's own route and its answer returned.
//! The layer sits on the routes named in `routes.rs` only. `dock/clear` and every other write is
//! never behind it, so no instance can change another's state through this.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::backend::instance_registry::{self, InstanceRecord};

/// How long a forwarded request may take. These are diagnostic reads of a local process: a
/// instance that does not answer in this time is reported, not waited for.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
/// The most of a forwarded answer that is passed on.
const MAX_FORWARDED_BYTES: usize = 8 * 1024 * 1024;

/// `GET /api/v1/muxspect/instances`: every live AgentMux instance on this machine (the caller's own
/// is marked `self`), without their keys.
pub async fn handle_muxspect_instances() -> impl IntoResponse {
    let Some(dir) = crate::registry::resolve_shared_instances_dir() else {
        return Json(json!({ "instances": [], "error": "this instance has no shared folder to list instances in" }));
    };
    let records = tokio::task::spawn_blocking(move || instance_registry::live_instances(&dir))
        .await
        .unwrap_or_default();
    Json(listing_body(&records, std::process::id(), agentmux_common::time::now_ms_u64()))
}

fn listing_body(records: &[InstanceRecord], own_pid: u32, now_ms: u64) -> serde_json::Value {
    json!({
        "own_channel": crate::backend::reactive::registry::local_channel_id(),
        "instances": records.iter().map(|r| r.listing(own_pid, now_ms)).collect::<Vec<_>>(),
    })
}

/// The `channel` parameter of a query, and the query without it. `None` when the request names no
/// channel (an empty `channel=` names none).
fn split_channel(query: Option<&str>) -> Option<(String, String)> {
    let mut channel = None;
    let mut rest = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in url::form_urlencoded::parse(query?.as_bytes()) {
        if k == "channel" {
            channel = Some(v.into_owned()).filter(|c| !c.is_empty());
        } else {
            rest.append_pair(&k, &v);
        }
    }
    channel.map(|c| (c, rest.finish()))
}

fn error_response(status: StatusCode, message: String, known: &[InstanceRecord]) -> Response {
    let channels: Vec<&str> = known.iter().map(|r| r.channel.as_str()).collect();
    (status, Json(json!({ "error": message, "live_channels": channels }))).into_response()
}

fn forward_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    // Loopback only: a proxy in the environment must never see these requests.
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(FORWARD_TIMEOUT)
            .build()
            .expect("a plain HTTP client builds")
    })
}

/// A layer for muxspect's read routes. Without `?channel=`, or naming this instance's own channel,
/// the request is served here as always.
pub async fn forward_to_channel(req: Request, next: Next) -> Response {
    let Some((channel, rest_query)) = split_channel(req.uri().query()) else {
        return next.run(req).await;
    };
    if channel == crate::backend::reactive::registry::local_channel_id() {
        return next.run(req).await;
    }
    let Some(dir) = crate::registry::resolve_shared_instances_dir() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "this instance has no shared folder to find other instances in".into(), &[]);
    };
    let path = req.uri().path().to_string();
    forward(&dir, &channel, &path, &rest_query, instance_registry::live_instances).await
}

/// Send `GET <path>?<query>` to the live instance on `channel`. `live` lists the live instances of
/// `dir` (the real process table in production; a test passes its own).
async fn forward(
    dir: &Path,
    channel: &str,
    path: &str,
    query: &str,
    live: impl FnOnce(&Path) -> Vec<InstanceRecord> + Send + 'static,
) -> Response {
    let owned_dir = dir.to_path_buf();
    let records = tokio::task::spawn_blocking(move || live(&owned_dir)).await.unwrap_or_default();
    let Some(target) = records.iter().find(|r| r.channel == channel) else {
        return error_response(StatusCode::NOT_FOUND, format!("no live AgentMux instance on channel '{channel}'"), &records);
    };
    let url = if query.is_empty() { format!("{}{path}", target.local_url) } else { format!("{}{path}?{query}", target.local_url) };
    let sent = forward_client().get(url).header(agentmux_common::AUTH_KEY_HEADER, &target.auth_key).send().await;
    match sent {
        Ok(resp) => relay(resp, channel).await,
        Err(e) => error_response(
            StatusCode::BAD_GATEWAY,
            format!("instance '{channel}' is listed but did not answer ({})", if e.is_timeout() { "timed out" } else { "connection failed" }),
            &records,
        ),
    }
}

/// The body of `resp`, read chunk by chunk so that no more than `cap` bytes are ever held: `None`
/// when it is larger (known from its Content-Length, or found out as it streams in).
async fn read_capped(mut resp: reqwest::Response, cap: usize) -> Result<Option<Vec<u8>>, reqwest::Error> {
    if resp.content_length().is_some_and(|len| len > cap as u64) {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > cap {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// The other instance's answer, as it gave it: its status, its content type and its body.
async fn relay(resp: reqwest::Response, channel: &str) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let version = resp.headers().get("x-agentmux-srv-version").cloned();
    let body = match read_capped(resp, MAX_FORWARDED_BYTES).await {
        Ok(Some(body)) => body,
        Ok(None) => {
            return error_response(StatusCode::BAD_GATEWAY, format!("instance '{channel}' answered with more than {MAX_FORWARDED_BYTES} bytes"), &[]);
        }
        Err(_) => return error_response(StatusCode::BAD_GATEWAY, format!("instance '{channel}' stopped answering part way"), &[]),
    };
    let mut out = (status, body).into_response();
    let headers = out.headers_mut();
    if let Some(v) = content_type {
        headers.insert(header::CONTENT_TYPE, v);
    }
    // The caller's own version header is stamped over this by the version layer; the other
    // instance's is kept under a name of its own so a stale-build 404 there still explains itself.
    if let Some(v) = version {
        headers.insert("x-agentmux-forwarded-srv-version", v);
    }
    if let Ok(v) = HeaderValue::from_str(channel) {
        headers.insert("x-agentmux-forwarded-channel", v);
    }
    out
}

#[cfg(test)]
#[path = "muxspect_instances_tests.rs"]
mod tests;
