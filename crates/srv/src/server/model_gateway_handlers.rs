// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The model gateway: `/api/v1/model/<route>/...`, a streaming pass-through from an agent's harness to
//! a model route (`docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md`, section 4).
//!
//! The caller proves itself with its gateway token, not the instance key, so these routes sit outside
//! `auth_middleware`. srv reads the route's key from the Vault, adds it to the upstream request and
//! streams the answer back unbuffered, counting the usage that passes. The key is in this function's
//! memory for one request and nowhere else.

use super::*;
use crate::backend::model_gateway::{catalog, launch, request as gw, spend, token, usage::UsageTap};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request};
use axum::http::{header, HeaderMap, HeaderName, StatusCode};
use futures_util::Stream;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Duration;

/// The largest request body forwarded; a conversation is far below it.
const MAX_REQUEST: usize = 64 * 1024 * 1024;
/// Response headers that describe the upstream's framing, which this hop replaces.
const FRAMING: &[&str] = &["content-length", "transfer-encoding", "connection", "keep-alive", "content-encoding"];

/// The client for model traffic. `AppState::http_client` ends every request 10 seconds after it starts,
/// reading the body included, which would cut off a streamed answer; this one bounds connecting and each
/// wait for the next chunk, and puts no limit on the whole answer.
fn gateway_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default()
    })
}

/// An error in the shape the harness already parses.
fn refuse(status: StatusCode, kind: &str, message: &str) -> Response {
    let body = json!({ "type": "error", "error": { "type": kind, "message": message } });
    (status, Json(body)).into_response()
}

pub(crate) async fn handle_model_gateway(State(state): State<AppState>, Path((route_id, tail)): Path<(String, String)>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let Some(grant) = gw::bearer_token(&parts.headers).and_then(|t| token::grant_for(&t)) else {
        return refuse(StatusCode::UNAUTHORIZED, "authentication_error", "the model gateway needs the agent's gateway token");
    };
    let Some(route) = catalog::route(&route_id).filter(|r| r.id == grant.route) else {
        return refuse(StatusCode::FORBIDDEN, "permission_error", "this token is not for that model route");
    };
    if !gw::path_allowed(route.harness, &tail) {
        return refuse(StatusCode::NOT_FOUND, "not_found_error", "the gateway forwards only the harness's own model paths");
    }
    let Some(model) = launch::model_on_route(&state.mstore, &grant.def_id, route) else {
        return refuse(StatusCode::FORBIDDEN, "permission_error", "this agent is not set to use that route");
    };
    let key = {
        let (store, agent, label, id) = (state.identity_store.clone(), grant.agent.clone(), route.key_label, route.id);
        tokio::task::spawn_blocking(move || crate::backend::vault::route_key::for_route(&store, label, &agent, id)).await
    };
    let key = match key {
        Ok(Ok(key)) => key,
        Ok(Err(why)) => return refuse(StatusCode::SERVICE_UNAVAILABLE, "api_error", &format!("{} has no usable key: {why}", route.label)),
        Err(_) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "the key lookup failed"),
    };
    let Ok(bytes) = axum::body::to_bytes(body, MAX_REQUEST).await else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large", "the request is too large");
    };
    let bytes = gw::with_served_model(&bytes, model.id, |m| route.model(m).is_some());
    let upstream = gateway_client()
        .request(parts.method, gw::upstream_url(route, &tail, parts.uri.query()))
        .headers(gw::forwarded_headers(&parts.headers))
        .bearer_auth(key.value.as_str())
        .body(bytes)
        .send()
        .await;
    match upstream {
        Ok(response) => relay(response, &grant, model),
        Err(e) => {
            tracing::warn!(agent = %grant.agent, route = route.id, error = %e.without_url(), "model gateway: upstream unreachable");
            refuse(StatusCode::BAD_GATEWAY, "api_error", &format!("{} could not be reached", route.label))
        }
    }
}

/// The upstream's answer, streamed through a meter that reads its usage.
fn relay(response: reqwest::Response, grant: &token::Grant, model: &'static catalog::Model) -> Response {
    let status = response.status();
    let mut headers = HeaderMap::new();
    for (name, value) in response.headers() {
        if !FRAMING.contains(&name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }
    let streaming = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream"));
    let metered = Metered { inner: Box::pin(response.bytes_stream()), tap: Some(UsageTap::new(streaming)), agent: grant.agent.clone(), model };
    let mut out = Response::new(Body::from_stream(metered));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    out.headers_mut().insert(HeaderName::from_static("x-accel-buffering"), "no".parse().unwrap());
    out
}

/// The response body, reading usage from each chunk and writing it to the agent's day when the body
/// ends or the harness hangs up.
struct Metered {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    tap: Option<UsageTap>,
    agent: String,
    model: &'static catalog::Model,
}

impl Metered {
    fn settle(&mut self) {
        let Some(tap) = self.tap.take() else { return };
        let (tokens, answered) = tap.finish();
        if tokens.input + tokens.output + tokens.cache_read + tokens.cache_creation == 0 {
            return;
        }
        let cost = spend::record(&self.agent, self.model, &tokens, agentmux_common::time::now_ms());
        tracing::info!(
            agent = %self.agent, model = self.model.id, answered = answered.as_deref().unwrap_or(""),
            input = tokens.input, output = tokens.output, cache_read = tokens.cache_read, cost_usd = cost,
            "model gateway: response"
        );
    }
}

impl Stream for Metered {
    type Item = Result<Bytes, reqwest::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let polled = self.inner.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(Some(Ok(chunk))) => {
                if let Some(tap) = self.tap.as_mut() {
                    tap.push(chunk);
                }
            }
            Poll::Ready(_) => self.settle(),
            Poll::Pending => {}
        }
        polled
    }
}

impl Drop for Metered {
    fn drop(&mut self) {
        self.settle();
    }
}
