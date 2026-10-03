// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/fleet` and `GET /agentmux/fleet/events`: this instance's
//! agent names for a LAN peer holding the `lan_key`, as a snapshot and as a
//! Server-Sent Events stream (`backend::fleet_feed`,
//! `docs/specs/SPEC_LAN_FLEET_FEED_2026_10_03.md`). Names plus instance
//! metadata a LAN peer already sees in the mDNS record; nothing else about an
//! agent.

use super::*;

/// Does `If-None-Match` name `etag`? Accepts a list, weak validators, `*`,
/// and a tag a client sent without its quotes.
fn if_none_match_matches(header_value: Option<&axum::http::HeaderValue>, etag: &str) -> bool {
    let Some(value) = header_value.and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let bare = etag.trim_matches('"');
    value.split(',').map(str::trim).any(|tag| {
        let tag = tag.strip_prefix("W/").unwrap_or(tag);
        tag == "*" || tag == etag || tag == bare
    })
}

/// The current snapshot, `304 Not Modified` (no body) when the client's
/// `If-None-Match` already names it.
pub(super) async fn handle_fleet(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let feed = &state.fleet_feed;
    let snapshot = feed.snapshot();
    let etag = format!("\"{}\"", feed.event_id(&snapshot));
    if if_none_match_matches(headers.get(header::IF_NONE_MATCH), &etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        feed.body_json(&snapshot),
    )
        .into_response()
}

/// The event stream: `retry`, the snapshot unless `Last-Event-ID` is already
/// current, then one `fleet` event per change and a heartbeat comment every
/// 15 s. 503 once `fleet_feed::MAX_STREAMS` streams are open.
pub(super) async fn handle_fleet_events(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    let Some(stream) = state.fleet_feed.open_stream(last_event_id) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "too many fleet event streams"})),
        )
            .into_response();
    };
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::if_none_match_matches;
    use axum::http::HeaderValue;

    #[test]
    fn if_none_match_handles_lists_weak_tags_and_wildcards() {
        let etag = "\"abc:2\"";
        let check = |v: &str| if_none_match_matches(Some(&HeaderValue::from_str(v).unwrap()), etag);
        assert!(check("\"abc:2\""));
        assert!(check("W/\"abc:2\""));
        assert!(check("\"abc:1\", \"abc:2\""));
        assert!(check("*"));
        assert!(check("abc:2"), "a client that dropped the quotes");
        assert!(!check("\"abc:1\""));
        assert!(!check(""));
        assert!(!if_none_match_matches(None, etag));
    }
}
