// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WhatsApp Cloud API inbound webhook receiver — axum handlers for the
//! one-time verification handshake and the per-event delivery POST.
//!
//! These are the genuinely new shape versus Discord/Telegram/Slack, which
//! only ever make *outbound* connections: this is passive HTTP the public
//! internet (Meta's servers) can reach, so every inbound POST must be
//! authenticated via HMAC signature (spec §3.2/§9) rather than AgentMux's
//! own `X-AuthKey` scheme, which Meta cannot supply. **These routes must be
//! registered outside `auth_middleware`** — see `server/mod.rs`, merged at
//! the top level alongside the unauthenticated `health` router, not inside
//! `authed_routes`. Putting them behind `X-AuthKey` would make Meta's calls
//! always fail (spec §8.2).
//!
//! ## Credential handling
//!
//! Two credentials pass through this module: the webhook verify token
//! (`hub.verify_token` on the GET handshake) and the app secret (used to
//! validate `X-Hub-Signature-256` on every POST). Neither is ever logged —
//! failures log only the fact of a mismatch, never the header value or the
//! secret. This mirrors the review findings baked into
//! `telegram/rest.rs::redact()` and `slack/rest.rs::redact_ticket()`: any
//! error string built from a value that might embed a credential must be
//! redacted (or, here, simply never constructed from the credential in the
//! first place) before it can reach a log line or an HTTP response body.

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::types::WebhookPayload;
use super::WhatsAppBridge;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;

type HmacSha256 = Hmac<Sha256>;

/// `GET /webhook/whatsapp?hub.mode=subscribe&hub.verify_token=...&hub.challenge=...`
///
/// Meta's one-time (per-URL-change) verification handshake (spec §3.1). Must
/// succeed before Meta starts delivering inbound POSTs, and Meta re-runs
/// this any time the webhook URL or verify token changes in the App
/// Dashboard.
pub async fn handle_verify(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let Some(bridge) = WhatsAppBridge::get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "whatsapp bridge not initialized".to_string(),
        )
            .into_response();
    };

    let mode = params.get("hub.mode").map(String::as_str).unwrap_or("");
    let token = params
        .get("hub.verify_token")
        .map(String::as_str)
        .unwrap_or("");
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    let token_matches = constant_time_eq(token.as_bytes(), bridge.verify_token().as_bytes());

    if mode == "subscribe" && token_matches {
        (StatusCode::OK, challenge).into_response()
    } else {
        // Never log `token` — it's the shared secret being validated.
        tracing::warn!(
            "whatsapp_bridge: webhook verification handshake rejected (mode={mode:?}, token_match={token_matches})"
        );
        (StatusCode::FORBIDDEN, "verification failed".to_string()).into_response()
    }
}

/// `POST /webhook/whatsapp` — inbound message delivery.
///
/// Body MUST be read as raw bytes (`Bytes`, not `Json<T>`) so the exact
/// signed payload is available for the HMAC check before any deserialization
/// happens (spec §3.2/§9.1) — deserializing first and re-serializing to
/// check the signature would validate a re-encoded payload, not what Meta
/// actually signed.
pub async fn handle_inbound(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    let Some(bridge) = WhatsAppBridge::get() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    let sig_header = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(bridge.app_secret(), &body, sig_header) {
        // Reject before any payload parsing or logging of body content
        // (spec §3.2 step 4). Do not log `sig_header` — while the signature
        // itself isn't directly reusable as a credential (it's a per-request
        // HMAC over this specific body), treating anything derived from
        // request headers as potentially sensitive and never echoing it into
        // logs is the safer default, consistent with the redaction posture
        // Telegram and Slack's bridges adopted after review.
        tracing::warn!("whatsapp_bridge: rejected inbound webhook — signature mismatch");
        return StatusCode::UNAUTHORIZED;
    }

    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("whatsapp_bridge: webhook payload parse error: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    for msg in payload.extract_messages() {
        bridge.record_inbound(&msg.from);

        let Some(target) = bridge.target_agent() else {
            tracing::debug!(
                "whatsapp_bridge: message from {} (no target agent configured)",
                msg.from
            );
            continue;
        };

        let envelope = format!("[WhatsApp {}]: {}", msg.from, msg.text);
        let handler = get_global_handler();
        let req = InjectionRequest {
            target_agent: target.clone(),
            message: envelope,
            source_agent: Some("whatsapp".to_string()),
            request_id: Some(msg.id.clone()),
            priority: None,
            wait_for_idle: false,
            jekt_tier: None,
            delivery_tier: Some("wan".to_string()),
            forward_hops: 0,
        };
        let result = handler.inject_message(req);
        if !result.success {
            tracing::warn!(
                "whatsapp_bridge: inject to agent {target} failed: {:?}",
                result.error
            );
        }
    }

    // Always 200 for a successfully-parsed batch, even if local injection
    // failed for one or more messages — per Meta's delivery semantics, a
    // non-2xx response causes retries and eventual (after ~7 days of
    // failures) event drop; local injection failures are our problem to
    // observe via logs, not something that should make Meta think delivery
    // itself failed (spec §5.2).
    StatusCode::OK
}

/// Validates `X-Hub-Signature-256: sha256=<hex>` = HMAC-SHA256(app_secret,
/// raw_body). Uses `Mac::verify_slice`, which performs a constant-time
/// comparison internally (backed by the `subtle` crate, already present
/// transitively via `hmac` in this workspace's dependency graph — no new
/// crate added).
fn verify_signature(app_secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

/// Constant-time byte comparison for the `hub.verify_token` handshake check
/// (spec §3.1 step 1). Hand-rolled (XOR-and-OR-accumulate) rather than
/// pulling in a dedicated crate — `subtle` is already available transitively
/// via `hmac` for the signature check above, but reaching for it here too
/// would mean threading an extra direct dependency through Cargo.toml for a
/// six-line primitive; this is the simplest correct implementation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn verify_signature_accepts_known_good_vector() {
        let secret = "app_secret_123";
        let body = b"{\"entry\":[]}";
        let header = sign(secret, body);
        assert!(verify_signature(secret, body, &header));
    }

    #[test]
    fn verify_signature_rejects_wrong_secret() {
        let header = sign("right_secret", b"hello");
        assert!(!verify_signature("wrong_secret", b"hello", &header));
    }

    #[test]
    fn verify_signature_rejects_tampered_body() {
        let header = sign("secret", b"original");
        assert!(!verify_signature("secret", b"tampered", &header));
    }

    #[test]
    fn verify_signature_rejects_missing_sha256_prefix() {
        assert!(!verify_signature("secret", b"hello", "deadbeef"));
    }

    #[test]
    fn verify_signature_rejects_malformed_hex() {
        assert!(!verify_signature("secret", b"hello", "sha256=not_valid_hex"));
    }

    #[test]
    fn verify_signature_rejects_empty_header() {
        assert!(!verify_signature("secret", b"hello", ""));
    }

    #[test]
    fn constant_time_eq_matches_equal_tokens() {
        assert!(constant_time_eq(b"my-verify-token", b"my-verify-token"));
    }

    #[test]
    fn constant_time_eq_rejects_different_tokens_same_length() {
        assert!(!constant_time_eq(b"my-verify-token1", b"my-verify-token2"));
    }

    #[test]
    fn constant_time_eq_rejects_different_lengths() {
        assert!(!constant_time_eq(b"short", b"a-much-longer-token"));
    }

    #[test]
    fn constant_time_eq_empty_vs_empty() {
        assert!(constant_time_eq(b"", b""));
    }
}
