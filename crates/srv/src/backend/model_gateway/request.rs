// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the gateway forwards: which paths, which of the harness's headers, and which model the request
//! names. Pure, so each is tested without a network.

use super::catalog::Route;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

/// The harness's own paths a route serves. Anything else is refused, so the gateway is not a way to
/// reach the rest of the upstream's API with the person's key.
pub fn path_allowed(harness: &str, tail: &str) -> bool {
    match harness {
        "claude" => matches!(tail, "v1/messages" | "v1/messages/count_tokens" | "v1/models"),
        _ => false,
    }
}

pub fn upstream_url(route: &Route, tail: &str, query: Option<&str>) -> String {
    match query {
        Some(q) if !q.is_empty() => format!("{}/{tail}?{q}", route.base()),
        _ => format!("{}/{tail}", route.base()),
    }
}

/// The headers worth forwarding. The harness's own credential is the gateway token and is never sent
/// on; the upstream gets the route's key instead.
const FORWARDED: &[&str] = &["content-type", "accept", "anthropic-version", "anthropic-beta", "user-agent"];

pub fn forwarded_headers(from: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in FORWARDED {
        let name = HeaderName::from_static(name);
        for value in from.get_all(&name) {
            out.append(name.clone(), value.clone());
        }
    }
    // Plain bytes back, so what the usage tap reads is what the harness gets.
    out.insert(HeaderName::from_static("accept-encoding"), HeaderValue::from_static("identity"));
    out
}

/// The gateway token the harness sent: `Authorization: Bearer ...` (Claude Code with
/// `ANTHROPIC_AUTH_TOKEN`) or `x-api-key`.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    text("authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .or_else(|| text("x-api-key"))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The request body with its `model` made one the route serves. A harness that insists on a Claude
/// name (the small model it uses for titles, an alias, a `[1m]` context suffix) is mapped to `chosen`,
/// the model the agent was given. A body that is not JSON, or names no model, goes on as it is.
pub fn with_served_model(body: &[u8], chosen: &str, serves: impl Fn(&str) -> bool) -> Vec<u8> {
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else { return body.to_vec() };
    let Some(named) = json.get("model").and_then(Value::as_str) else { return body.to_vec() };
    let bare = named.trim_end_matches("[1m]");
    let model = if serves(bare) { bare } else { chosen };
    if model == named {
        return body.to_vec();
    }
    json["model"] = Value::String(model.to_string());
    serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model_gateway::catalog;

    #[test]
    fn only_the_harnesss_own_paths_are_forwarded() {
        assert!(path_allowed("claude", "v1/messages"));
        assert!(path_allowed("claude", "v1/messages/count_tokens"));
        assert!(!path_allowed("claude", "v1/credits"));
        assert!(!path_allowed("claude", "../keys"));
        assert!(!path_allowed("codex", "v1/messages"));
    }

    #[test]
    fn the_upstream_address_comes_from_the_route_alone() {
        let route = catalog::route("openrouter").unwrap();
        assert_eq!(upstream_url(route, "v1/messages", None), "https://openrouter.ai/api/v1/messages");
        assert_eq!(upstream_url(route, "v1/messages", Some("beta=true")), "https://openrouter.ai/api/v1/messages?beta=true");
    }

    #[test]
    fn the_harnesss_credential_is_not_forwarded() {
        let mut from = HeaderMap::new();
        from.insert("authorization", HeaderValue::from_static("Bearer amxg_secret"));
        from.insert("x-api-key", HeaderValue::from_static("amxg_secret"));
        from.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        from.insert("content-type", HeaderValue::from_static("application/json"));
        let out = forwarded_headers(&from);
        assert!(out.get("authorization").is_none() && out.get("x-api-key").is_none());
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(out.get("accept-encoding").unwrap(), "identity");
    }

    #[test]
    fn the_token_is_read_from_either_header() {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_static("Bearer amxg_a"));
        assert_eq!(bearer_token(&h).as_deref(), Some("amxg_a"));
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("amxg_b"));
        assert_eq!(bearer_token(&h).as_deref(), Some("amxg_b"));
        assert_eq!(bearer_token(&HeaderMap::new()), None);
    }

    #[test]
    fn a_claude_name_is_mapped_to_the_chosen_model() {
        let route = catalog::route("openrouter").unwrap();
        let serves = |m: &str| route.model(m).is_some();
        let chosen = "deepseek/deepseek-v4.1-flash-1017";
        let rewritten = |model: &str| {
            let body = format!(r#"{{"model":"{model}","max_tokens":5}}"#);
            let out = with_served_model(body.as_bytes(), chosen, serves);
            serde_json::from_slice::<Value>(&out).unwrap()["model"].as_str().unwrap().to_string()
        };
        assert_eq!(rewritten("claude-haiku-4-5-20251001"), chosen);
        assert_eq!(rewritten("sonnet"), chosen);
        assert_eq!(rewritten("deepseek/deepseek-v4-pro-0813"), "deepseek/deepseek-v4-pro-0813");
        assert_eq!(rewritten("deepseek/deepseek-v4-pro-0813[1m]"), "deepseek/deepseek-v4-pro-0813");
        assert_eq!(with_served_model(b"not json", chosen, serves), b"not json");
    }
}
