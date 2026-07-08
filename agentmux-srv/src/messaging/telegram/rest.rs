// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telegram Bot API REST client — `getUpdates` long-polling plus
//! `sendMessage`/`editMessageText` for outbound delivery.
//!
//! Rate limiting: v1 implements only per-chat 1 msg/s spacing (enforced by
//! the caller in `poller.rs`) and reactive `429`/`retry_after` backoff. Full
//! token-bucket accounting for the 20/min-per-group and ~30/s-global tiers is
//! explicitly out of scope (spec §2.5) — mirrors Discord's `rest.rs`, which
//! logs 429s and returns an error rather than doing header-driven throttling.
//!
//! Never log the full request URL: the bot token is embedded in the path
//! (`https://api.telegram.org/bot{TOKEN}/...`), unlike Discord's
//! `Authorization` header. Log method names only, never the URL.

use std::time::Duration;

use super::types::{EditMessageTextBody, Message, SendMessageBody, TelegramResponse};

const TELEGRAM_API: &str = "https://api.telegram.org";

/// Telegram's own long-poll wait, in seconds (passed as the `timeout` query param).
const POLL_TIMEOUT_SECS: u64 = 30;
/// HTTP client timeout for `getUpdates` — must exceed `POLL_TIMEOUT_SECS` with
/// margin, or the client aborts the request right as data would have arrived
/// (spec §9 — long-poll timeout vs HTTP client timeout mismatch).
const POLL_CLIENT_TIMEOUT_SECS: u64 = 35;
/// Per-request timeout for send/edit calls. These share a `tokio::select!`
/// with the long poll, so a stuck send must not block inbound polling for
/// anywhere near 30s.
const SEND_TIMEOUT_SECS: u64 = 10;

/// Error from a Telegram Bot API call, carrying enough of the `429` envelope
/// for the caller to implement per-chat backoff (spec §2.5).
#[derive(Debug, Clone)]
pub struct TelegramApiError {
    pub message: String,
    /// Present on 429 responses: seconds to wait before retrying.
    pub retry_after: Option<u64>,
    /// True when the 429's `parameters.scope` is `"chat"` — pause only that
    /// chat's queue. False/absent → treat as a global-scope limit.
    pub chat_scoped: bool,
}

impl std::fmt::Display for TelegramApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl TelegramApiError {
    fn simple(message: impl Into<String>) -> Self {
        TelegramApiError {
            message: message.into(),
            retry_after: None,
            chat_scoped: false,
        }
    }
}

/// Strips the bot token out of an error message before it's ever logged or
/// returned. `reqwest::Error`'s `Display` embeds the request URL on
/// send/connection failures, and the URL contains `bot{token}` — so `{e}`
/// must never reach a log line or a `Result<_, String>` unredacted. This is
/// the single choke point every error-construction site routes through, so
/// callers can't accidentally skip it.
fn redact(token: &str, s: String) -> String {
    s.replace(token, "***")
}

/// Build a dedicated client for `getUpdates` with a timeout that safely
/// exceeds Telegram's own 30s long-poll wait. Kept distinct from the client
/// used for sends (spec §9).
pub fn build_poll_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(POLL_CLIENT_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// GET /bot{token}/getUpdates
///
/// Returns raw JSON values rather than typed `Update`s deliberately: a single
/// malformed update must not abort the whole batch (spec §9), so per-item
/// typed parsing happens in `poller.rs`'s loop, where a parse failure on one
/// item can be logged and skipped without losing the `update_id` needed to
/// advance the offset past it.
pub async fn get_updates(
    http: &reqwest::Client,
    token: &str,
    offset: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let url = format!("{TELEGRAM_API}/bot{token}/getUpdates");
    let resp = http
        .get(&url)
        .query(&[
            ("timeout", POLL_TIMEOUT_SECS.to_string()),
            ("offset", offset.to_string()),
            (
                "allowed_updates",
                "[\"message\",\"callback_query\"]".to_string(),
            ),
        ])
        .send()
        .await
        .map_err(|e| redact(token, format!("telegram rest: getUpdates http error: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::CONFLICT {
        return Err(
            "telegram: another getUpdates poller is active for this token (409) — check for a duplicate AgentMux instance or a registered webhook"
                .to_string(),
        );
    }

    let body: TelegramResponse<Vec<serde_json::Value>> = resp
        .json()
        .await
        .map_err(|e| redact(token, format!("telegram rest: getUpdates parse error: {e}")))?;

    if !body.ok {
        return Err(format!(
            "telegram rest: getUpdates failed: {}",
            body.description.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    Ok(body.result.unwrap_or_default())
}

/// POST /bot{token}/sendMessage
pub async fn send_message(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
) -> Result<Message, TelegramApiError> {
    let url = format!("{TELEGRAM_API}/bot{token}/sendMessage");
    let body = SendMessageBody {
        chat_id,
        text: escape_html(text),
        parse_mode: "HTML",
    };
    post_for_message(http, token, &url, &body, "sendMessage").await
}

/// POST /bot{token}/editMessageText
pub async fn edit_message_text(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<Message, TelegramApiError> {
    let url = format!("{TELEGRAM_API}/bot{token}/editMessageText");
    let body = EditMessageTextBody {
        chat_id,
        message_id,
        text: escape_html(text),
        parse_mode: "HTML",
    };
    post_for_message(http, token, &url, &body, "editMessageText").await
}

/// Dispatches to `send_message` or `edit_message_text` depending on whether
/// `edit_message_id` is set (spec §2.3 — streaming-output simulation).
pub async fn send_or_edit(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    edit_message_id: Option<i64>,
) -> Result<Message, TelegramApiError> {
    match edit_message_id {
        Some(message_id) => edit_message_text(http, token, chat_id, message_id, text).await,
        None => send_message(http, token, chat_id, text).await,
    }
}

async fn post_for_message<B: serde::Serialize>(
    http: &reqwest::Client,
    token: &str,
    url: &str,
    body: &B,
    method_name: &str,
) -> Result<Message, TelegramApiError> {
    let resp = http
        .post(url)
        .timeout(Duration::from_secs(SEND_TIMEOUT_SECS))
        .json(body)
        .send()
        .await
        .map_err(|e| {
            TelegramApiError::simple(redact(token, format!("telegram rest: {method_name} http error: {e}")))
        })?;

    let status = resp.status();

    let parsed: TelegramResponse<Message> = resp.json().await.map_err(|e| {
        TelegramApiError::simple(redact(token, format!("telegram rest: {method_name} parse error: {e}")))
    })?;

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || !parsed.ok {
        let retry_after = parsed.parameters.as_ref().and_then(|p| p.retry_after);
        let chat_scoped = parsed
            .parameters
            .as_ref()
            .and_then(|p| p.scope.as_deref())
            .map(|s| s == "chat")
            .unwrap_or(false);
        return Err(TelegramApiError {
            message: format!(
                "telegram rest: {method_name} failed ({status}): {}",
                parsed.description.unwrap_or_else(|| "unknown error".to_string())
            ),
            retry_after,
            chat_scoped,
        });
    }

    parsed
        .result
        .ok_or_else(|| TelegramApiError::simple(format!("telegram rest: {method_name} ok but no result")))
}

/// Escapes the three characters HTML `parse_mode` treats specially. Per spec
/// §2.2, Telegram's HTML mode only requires escaping `<`, `>`, `&` — simpler
/// than MarkdownV2. Order matters: `&` must be escaped first so the escape
/// sequences for `<`/`>` aren't themselves re-escaped.
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_escapes_ampersand_first() {
        assert_eq!(escape_html("<b>a & b</b>"), "&lt;b&gt;a &amp; b&lt;/b&gt;");
    }

    #[test]
    fn escape_html_noop_on_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
    }

    #[test]
    fn escape_html_handles_all_three_chars_together() {
        assert_eq!(escape_html("a<b>c&d"), "a&lt;b&gt;c&amp;d");
    }

    #[test]
    fn redact_strips_token_from_url_bearing_error_text() {
        let token = "123456:ABC-secret";
        let msg = format!(
            "telegram rest: getUpdates http error: error sending request for url (https://api.telegram.org/bot{token}/getUpdates?timeout=30)"
        );
        let redacted = redact(token, msg);
        assert!(!redacted.contains(token), "token leaked into: {redacted}");
        assert!(redacted.contains("bot***"));
    }
}
