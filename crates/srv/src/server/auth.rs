// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Origin checks and the auth middlewares.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;
use agentmux_common::AUTH_KEY_HEADER;

// ---- Origin checks ----

/// A loopback origin ([`is_loopback_origin`]), or one a headless srv was
/// told to accept with `--allowed-origin` (a reverse proxy's public origin).
/// Used by CORS and by [`ws_origin_guard`].
pub(crate) fn is_allowed_origin(origin: &str) -> bool {
    is_loopback_origin(origin) || crate::headless::is_extra_allowed_origin(origin)
}

/// `http://127.0.0.1` or `http://localhost`, optionally with a numeric
/// port: where the desktop frontend is served from (the CEF host's loopback
/// server, or the Vite dev server). A headless srv behind a reverse proxy can
/// also accept the proxy's origin; see [`is_allowed_origin`], which CORS and
/// [`ws_origin_guard`] use.
pub(crate) fn is_loopback_origin(origin: &str) -> bool {
    ["http://127.0.0.1", "http://localhost"].iter().any(|host| {
        origin.strip_prefix(host).is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix(':')
                    .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        })
    })
}

/// Refuse a WebSocket upgrade whose `Origin` is neither loopback nor allowed
/// by `--allowed-origin` ([`is_allowed_origin`]).
///
/// CORS does not apply to WebSocket upgrades, so without this any web page
/// holding the auth key could open `/ws` (cross-site WebSocket hijacking).
/// The key is not sent automatically the way a cookie would be, so this is
/// defense in depth rather than a fix for a reachable attack. Browsers always
/// send `Origin` on an upgrade; srv's native clients (launcher, agentmux-mcp)
/// send none, so a missing header is allowed.
pub(super) async fn ws_origin_guard(req: Request<Body>, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).map(|v| v.to_str().unwrap_or(""));
    match origin {
        Some(o) if !is_allowed_origin(o) => {
            tracing::warn!(origin = %o, "refused /ws upgrade from an origin that isn't allowed");
            (StatusCode::FORBIDDEN, Json(json!({"error": "origin not allowed"}))).into_response()
        }
        _ => next.run(req).await,
    }
}

// ---- Auth Middleware ----

/// Auth middleware matching Go pkg/authkey/authkey.go:18-42.
pub(super) async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }

    let auth_key = req
        .headers()
        .get(AUTH_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 2026-05-11 audit (C3): the query-string `?authkey=` fallback
    // bypasses CORS preflight and is preserved in browser history,
    // navigation `Referer` headers, server access logs, etc. — a CSRF
    // amplifier whenever the key leaks. It is allowed **only** on the
    // WebSocket upgrade route (`/ws`), where the browser WS API doesn't
    // permit custom headers and there is no other practical channel
    // for the key. Every other route requires the header.
    let auth_key = auth_key.or_else(|| {
        if req.uri().path() != "/ws" {
            return None;
        }
        req.uri().query().and_then(|q| {
            q.split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == "authkey")
                .map(|(_, v)| v.to_string())
        })
    });

    match auth_key {
        Some(key) if secret_eq(key.as_bytes(), state.auth_key.as_bytes()) => {
            // Identity M4a: `caller_middleware` derives a `Caller` only for a
            // request marked as full-key authenticated — never by default.
            req.extensions_mut().insert(ReactiveAuthVia::FullAuthKey);
            next.run(req).await
        }
        Some(key) => match crate::backend::container_credential::grant_for(&key) {
            Some(grant) => admit_container_agent(grant, req, next).await,
            None => unauthorized(),
        },
        None => unauthorized(),
    }
}

pub(super) fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response()
}

/// A container agent's token: allowed only on
/// [`crate::backend::container_credential::container_route_allowed`]. There it
/// stands in for the instance key it replaced (so jekt tiers and identity
/// behave as for any agent), with the caller pinned to the grant's agent by
/// `caller_middleware`.
pub(super) async fn admit_container_agent(
    grant: crate::backend::container_credential::ContainerGrant,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    use crate::backend::container_credential::{container_route_allowed, route_needs_identity};
    let path = req.uri().path().to_string();
    let refused = if !container_route_allowed(&path) {
        Some("this route is not available to container agents")
    } else if route_needs_identity(&path) && grant.agent_token.is_none() {
        Some("this container agent has no identity token")
    } else {
        None
    };
    if let Some(reason) = refused {
        tracing::warn!(block_id = %grant.block_id, %path, reason, "container agent request refused");
        return (StatusCode::FORBIDDEN, Json(json!({"error": reason}))).into_response();
    }
    req.extensions_mut().insert(ReactiveAuthVia::FullAuthKey);
    req.extensions_mut().insert(grant);
    next.run(req).await
}

/// Auth middleware for the two LAN-forwarding-relevant reactive routes
/// (`/agentmux/reactive/agent`, `/agentmux/reactive/inject`) — accepts
/// EITHER `state.auth_key` (the normal, full-access case: every other
/// caller of these two routes, e.g. agentmux-mcp's SendMessage tool for a
/// plain host-tier inject) OR `state.lan_key` (a LAN peer forwarding a
/// jekt or looking up which agents this instance hosts).
///
/// Deliberately a SEPARATE middleware from `auth_middleware`, applied to a
/// standalone router merged at the top level rather than nested inside
/// `authed_routes` — nesting would put `authed_routes`'s own
/// `route_layer(auth_middleware)` on the outside, rejecting the LAN key
/// before this middleware ever ran. This is why these two routes live in
/// their own `lan_forward_routes` router in `router()`, not in
/// `reactive_routes`.
///
/// `state.lan_key` grants access to ONLY the `lan_forward_routes` — never the rest
/// of `/agentmux/service`, `/agentmux/file`, shell creation, credential/
/// identity endpoints, etc. See `Config::lan_key`'s doc comment for why
/// this exists.
/// Which of the two credentials `lan_or_full_auth_middleware` accepted
/// authenticated this specific request. Inserted into the request's
/// extensions so `handle_reactive_inject` can force `delivery_tier = "lan"`
/// whenever `LanKey` authenticated the request, regardless of what the JSON
/// body itself claims — closing the bypass where a `lan_key`-only holder
/// could otherwise just self-label a request "host" to dodge LAN's stricter
/// verification entirely. See
/// docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §3 for the full
/// rationale, including why the reverse (downgrading a "lan" claim seen
/// under `FullAuthKey`) is deliberately NOT done — reagentx P0 found that
/// same-host forwarding (Tier 2a/2b in `reactive.rs`) legitimately
/// re-authenticates an already-LAN-originated jekt with a sibling
/// instance's own full `auth_key`, so downgrading there silently discarded
/// an already-detected signature failure's forced-sensitive escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReactiveAuthVia {
    FullAuthKey,
    LanKey,
}

pub(super) async fn lan_or_full_auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }

    let auth_key = req
        .headers()
        .get(AUTH_KEY_HEADER)
        .and_then(|v| v.to_str().ok());

    match auth_key {
        Some(key) if secret_eq(key.as_bytes(), state.auth_key.as_bytes()) => {
            req.extensions_mut().insert(ReactiveAuthVia::FullAuthKey);
            next.run(req).await
        }
        Some(key) if secret_eq(key.as_bytes(), state.lan_key.as_bytes()) => {
            req.extensions_mut().insert(ReactiveAuthVia::LanKey);
            next.run(req).await
        }
        Some(key) => match crate::backend::container_credential::grant_for(key) {
            Some(grant) => admit_container_agent(grant, req, next).await,
            None => unauthorized(),
        },
        None => unauthorized(),
    }
}
