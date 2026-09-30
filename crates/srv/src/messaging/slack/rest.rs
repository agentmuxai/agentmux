// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slack Web API REST client — `apps.connections.open` (Socket Mode URL
//! fetch) + `chat.postMessage` for outbound delivery.
//!
//! Rate limiting: v1 implements only per-channel ~1msg/s spacing (enforced
//! by the caller in `socket.rs`'s outbound loop) and reactive
//! `429`/`Retry-After` backoff. Full token-bucket accounting for every
//! documented Slack API tier is explicitly out of scope (spec §2.6) —
//! mirrors Discord's and Telegram's `rest.rs`, neither of which does
//! header-driven throttling beyond reacting to a rate-limit signal.
//!
//! Secret-in-error-message hygiene: both Slack tokens (`xoxb-`/`xapp-`) are
//! sent as `Authorization: Bearer` headers, never embedded in request URLs —
//! unlike Telegram's `bot{token}` path convention — so `reqwest::Error`'s
//! `Display` (which can embed request URLs on connection failures) does not
//! leak either token via that vector, and no `redact()`-style scrubbing of
//! `reqwest::Error` text is needed here the way `telegram/rest.rs` needs it.
//! The one Slack-specific value that *is* both URL-embedded and secret is
//! the Socket Mode WS URL's `ticket=` query param returned by
//! `apps.connections.open`: it's a single-use, short-lived credential (spec
//! §2.1, §9.6). We never pass that URL through this module's error paths
//! (the WS connect happens in `socket.rs`), but we still provide
//! `redact_ticket` here so any code that logs the URL (today: one `debug!`
//! call in `socket.rs`) redacts it first — unnecessary exposure in a log
//! that can persist long past the ticket's few-seconds validity window is
//! not worth the convenience of an unredacted log line.

use std::time::Duration;

use super::types::{OpenConnectionResponse, PostMessageBody, PostMessageResponse};
use crate::messaging::OutboundMsg;

const SLACK_API: &str = "https://slack.com/api";
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Error from a Slack Web API call, carrying the `Retry-After` seconds when
/// present (429 responses only — Slack signals rate limits via the HTTP
/// header, not a response-body field like Telegram's `parameters.retry_after`
/// — spec §2.6).
#[derive(Debug, Clone)]
pub struct SlackApiError {
    pub message: String,
    pub retry_after: Option<u64>,
}

impl std::fmt::Display for SlackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl SlackApiError {
    fn simple(message: impl Into<String>) -> Self {
        SlackApiError {
            message: message.into(),
            retry_after: None,
        }
    }
}

/// Redacts the single-use `ticket` query param from a Socket Mode WS URL
/// before it's ever logged (spec §9.6). Leaves the rest of the URL (host,
/// `app_id`, path) intact since that part isn't secret and is useful for
/// debugging connection issues.
pub fn redact_ticket(url: &str) -> String {
    let Some(idx) = url.find("ticket=") else {
        return url.to_string();
    };
    let value_start = idx + "ticket=".len();
    let end = url[value_start..]
        .find('&')
        .map(|i| value_start + i)
        .unwrap_or(url.len());
    format!("{}ticket=***{}", &url[..idx], &url[end..])
}

/// Parses an HTTP `Retry-After` header value (seconds, per RFC 9110 — Slack
/// always sends the delta-seconds form, never the HTTP-date form).
fn parse_retry_after(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

/// POST apps.connections.open — `Authorization: Bearer {app_token}`. Returns
/// the ephemeral Socket Mode WS URL. Must be called before the initial
/// connect and before every reconnect (spec §2.1) — the URL is single-use
/// and short-lived; never cache/reuse it.
pub async fn open_connection(http: &reqwest::Client, app_token: &str) -> Result<String, String> {
    let resp = http
        .post(format!("{SLACK_API}/apps.connections.open"))
        .header("Authorization", format!("Bearer {app_token}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("slack rest: apps.connections.open http error: {e}"))?;

    let status = resp.status();
    let body: OpenConnectionResponse = resp
        .json()
        .await
        .map_err(|e| format!("slack rest: apps.connections.open parse error: {e}"))?;

    if !status.is_success() || !body.ok {
        return Err(format!(
            "slack rest: apps.connections.open failed ({status}): {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    body.url
        .ok_or_else(|| "slack rest: apps.connections.open ok but no url".to_string())
}

/// POST chat.postMessage — `Authorization: Bearer {bot_token}`. MUST check
/// `body.ok`, not just HTTP status: Slack's REST responses are HTTP 200 even
/// on many logical failures (e.g. `channel_not_found`, `not_in_channel`) —
/// spec §2.4.
pub async fn post_message(
    http: &reqwest::Client,
    bot_token: &str,
    channel_id: &str,
    msg: &OutboundMsg,
) -> Result<(), SlackApiError> {
    let body = PostMessageBody {
        channel: channel_id.to_string(),
        text: msg.text.clone(),
        blocks: msg.blocks.clone(),
    };

    let resp = http
        .post(format!("{SLACK_API}/chat.postMessage"))
        .header("Authorization", format!("Bearer {bot_token}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|e| SlackApiError::simple(format!("slack rest: chat.postMessage http error: {e}")))?;

    let status = resp.status();
    let retry_after = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after)
    } else {
        None
    };

    let parsed: PostMessageResponse = resp.json().await.map_err(|e| SlackApiError {
        message: format!("slack rest: chat.postMessage parse error: {e}"),
        retry_after,
    })?;

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || !parsed.ok {
        return Err(SlackApiError {
            message: format!(
                "slack rest: chat.postMessage failed ({status}): {}",
                parsed.error.unwrap_or_else(|| "unknown error".to_string())
            ),
            retry_after,
        });
    }

    Ok(())
}

/// POST to a slash-command `response_url`. Stub for the deferred-response
/// path (spec §2.5, §4.3) — not called anywhere in v1 scope; slash commands
/// are explicitly out of scope for this PR. Kept so the module shape doesn't
/// need rework when they land.
#[allow(dead_code)]
pub async fn post_response_url(
    http: &reqwest::Client,
    response_url: &str,
    body: serde_json::Value,
) -> Result<(), String> {
    let resp = http
        .post(response_url)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("slack rest: response_url post http error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("slack rest: response_url post failed: {}", resp.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_ticket_strips_value_keeps_rest_of_url() {
        let url = "wss://wss-primary.slack.com/link/?ticket=abc123secret&app_id=A1";
        let redacted = redact_ticket(url);
        assert!(!redacted.contains("abc123secret"), "ticket leaked into: {redacted}");
        assert!(redacted.contains("ticket=***"));
        assert!(redacted.contains("app_id=A1"));
        assert!(redacted.starts_with("wss://wss-primary.slack.com/link/?"));
    }

    #[test]
    fn redact_ticket_handles_ticket_as_last_param() {
        let url = "wss://wss-primary.slack.com/link/?app_id=A1&ticket=abc123secret";
        let redacted = redact_ticket(url);
        assert!(!redacted.contains("abc123secret"));
        assert!(redacted.ends_with("ticket=***"));
    }

    #[test]
    fn redact_ticket_noop_when_no_ticket_param() {
        let url = "wss://wss-primary.slack.com/link/?app_id=A1";
        assert_eq!(redact_ticket(url), url);
    }

    #[test]
    fn parse_retry_after_parses_plain_seconds() {
        assert_eq!(parse_retry_after("30"), Some(30));
    }

    #[test]
    fn parse_retry_after_trims_whitespace() {
        assert_eq!(parse_retry_after("  5  "), Some(5));
    }

    #[test]
    fn parse_retry_after_none_on_http_date_form() {
        // Slack always sends delta-seconds, but guard against the HTTP-date
        // form anyway rather than panicking/misbehaving on it.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }
}
