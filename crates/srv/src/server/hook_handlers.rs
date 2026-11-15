// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! `POST /agentmux/hook/:job`: a script on this computer starts an event job's run
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2, `webhook_local`; the job side is `backend::jobs::webhook_local`).
//!
//! Any page open in a browser pane can POST to loopback, so the request has to look like a script's and not a page's.
//! The route is on the full router only, which listens on loopback; it is not behind the AuthKey (a script has none),
//! and the job's own secret is its key. In this order, a request is refused when:
//! - its peer is not a loopback address, or its `Host` is not a loopback name (a DNS rebinding page arrives under its own);
//! - it carries an `Origin` or any `Sec-Fetch-*` header (a browser sends them, `curl` and scripts do not);
//! - its content type is not `application/json` (a cross-site form cannot send that without a preflight, which is not answered);
//! - it has no secret header, or the secret is not the job's (the same answer for a job that does not exist or is off);
//! - its body is over 64 KiB, or is not a JSON object.

use std::net::{IpAddr, SocketAddr};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Extension, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::AppState;
use crate::backend::cron::{FireOutcome, JobEvent};
use crate::backend::jobs::definition::event_source;
use crate::backend::jobs::webhook_local::{accepts, ID_HEADER, MAX_ID_CHARS, SECRET_HEADER};

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": why }))).into_response()
}

/// Whether the `Host` header names this computer's loopback: `127.0.0.1`, `localhost` or `[::1]`, with or without a port.
fn loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { host.rsplit_once(':').map_or(host, |(n, _)| n) };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

/// What is wrong with a request before the secret is looked at, or nothing. Pure: the handler's first four rules.
pub(crate) fn precheck(peer: Option<IpAddr>, headers: &HeaderMap) -> Result<String, Response> {
    if peer.is_some_and(|ip| !ip.is_loopback()) {
        return Err(refuse(StatusCode::FORBIDDEN, "this address answers only on this computer"));
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !loopback_host(host) {
        return Err(refuse(StatusCode::FORBIDDEN, "this address answers only on this computer"));
    }
    if headers.contains_key(header::ORIGIN) || headers.keys().any(|k| k.as_str().starts_with("sec-fetch-")) {
        return Err(refuse(StatusCode::FORBIDDEN, "a web page may not call this; send it from a script"));
    }
    let json_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| {
        let essence = v.split(';').next().unwrap_or("").trim();
        essence.eq_ignore_ascii_case("application/json")
    });
    if !json_type {
        return Err(refuse(StatusCode::UNSUPPORTED_MEDIA_TYPE, "send Content-Type: application/json"));
    }
    match headers.get(SECRET_HEADER).and_then(|v| v.to_str().ok()).filter(|s| !s.is_empty()) {
        Some(secret) => Ok(secret.to_string()),
        None => Err(refuse(StatusCode::UNAUTHORIZED, "the secret is not right")),
    }
}

pub(crate) async fn handle_hook(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let secret = match precheck(peer.map(|Extension(ConnectInfo(a))| a.ip()), &headers) {
        Ok(s) => s,
        Err(refused) => return refused,
    };
    let Some(jobs) = super::cron::jobs_store(&state) else { return refuse(StatusCode::UNAUTHORIZED, "the secret is not right") };
    let id = job_id.clone();
    let job = tokio::task::spawn_blocking(move || jobs.cron_get(&id)).await.ok().and_then(Result::ok).flatten();
    let Some(job) = job.filter(|j| accepts(j, &secret)) else { return refuse(StatusCode::UNAUTHORIZED, "the secret is not right") };
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&body) else {
        return refuse(StatusCode::BAD_REQUEST, "send a JSON object");
    };
    let call = headers.get(ID_HEADER).and_then(|v| v.to_str().ok()).map(str::trim).filter(|s| !s.is_empty() && s.chars().count() <= MAX_ID_CHARS && !s.chars().any(char::is_control));
    let event = JobEvent {
        id: format!("wh:{}:{}", job.id, call.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_string)),
        source: event_source::WEBHOOK_LOCAL.to_string(),
        input: json!({ "body": object, "received_ms": agentmux_common::time::now_ms() }),
    };
    let fired = state.cron_scheduler.fire_event(&job, event).await;
    let status = match (&fired.outcome, &fired.waiting, &fired.run_id) {
        (Some(FireOutcome::Delivered { .. }), _, _) => "started",
        (_, Some(_), Some(_)) => "waiting",
        _ => "not_started",
    };
    (StatusCode::ACCEPTED, Json(json!({ "ok": true, "status": status, "run_id": fired.run_id }))).into_response()
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
