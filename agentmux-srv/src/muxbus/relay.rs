// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Outbound cloud relay — jekt delivery **tier 4**.
//!
//! Tiers 1–3 (local handler, same-host registry, LAN peer) all live in
//! `server/reactive.rs`. Tier 4 was, until this module existed, a comment
//! there — `// 4. Return original error (muxbus-client will fall back to cloud
//! relay)` — delegating to a "muxbus-client" that the callers agents actually
//! use are not. The MCP `SendMessage` tool POSTs to `/agentmux/reactive/inject`
//! and bails on `success != true`, so the tier chain silently stopped at three
//! while `SendMessage`'s own description promised "local → LAN → cloud". See
//! `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md` §5.
//!
//! This module makes tier 4 real and owned by the same handler as the other
//! three, so every caller inherits it instead of re-implementing it.
//!
//! ## What "success" means here
//!
//! Tier 4 is **store-and-forward, not delivery**. The cloud persists the
//! injection and broadcasts a wake signal; the recipient's srv picks it up on
//! its next sync, which may be seconds away or never (that instance may be
//! offline). A 2xx here therefore means *queued*, and [`RelayOutcome::Queued`]
//! is deliberately not named anything that reads as delivered — the tier-3
//! forward path had exactly that confusion and it produced a real bug (see
//! `forward_inject_to_peer`'s "one behaviour unified" note).

use std::sync::Arc;

use crate::backend::storage::store::Store;

/// The cloud rejects anything larger (`index.ts`: "message exceeds maximum
/// length of 10KB"). Checked locally so an oversized message fails with a
/// useful error instead of a bare 400 after a round trip.
const MAX_RELAY_MESSAGE_BYTES: usize = 10240;

/// Bounds how long a failed local delivery can block on the cloud before the
/// caller gets its answer. Tier 4 runs only after tiers 1–3 have already
/// failed, so this is additive to a request that is already slow.
const RELAY_TIMEOUT_SECS: u64 = 10;

/// Base URL of the muxbus REST API.
///
/// The env override exists for tests and for running against a local muxbus
/// server (`http://localhost:3100`, registered in the dev Cognito app client).
/// Previously private to `pkce.rs`; promoted here because tier 4 needs the same
/// resolution and two copies of "where is the cloud" is exactly the drift this
/// report set out to remove.
pub(crate) fn rest_base_url() -> String {
    std::env::var("AGENTMUX_MUXBUS_REST_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::muxbus::cloud_subscriber::MUXBUS_REST_URL.to_string())
}

/// Outcome of a tier-4 relay attempt.
///
/// There is deliberately no "cloud not configured" variant: that condition is
/// answered earlier and more cheaply by [`relay_token`] returning `None`, so
/// reaching this function at all already means tier 4 exists here.
#[derive(Debug)]
pub(crate) enum RelayOutcome {
    /// The cloud accepted and persisted the injection. **Queued, not
    /// delivered** — see the module doc.
    Queued { injection_id: Option<String> },
    /// The cloud was reachable and said no, or the request never landed.
    Failed(String),
}

/// POST one injection to the cloud relay.
///
/// Deliberately takes `base_url` and `token` rather than resolving them
/// internally: that keeps this function a pure HTTP operation with no globals,
/// so the tests can point it at a stub relay without mutating process-wide env
/// (which would race the rest of the test binary).
///
/// ## Contract (verified against `agentmux-cloud/muxbus/server/src/index.ts`)
///
/// - `source_agent` travels in the **`X-Agent-ID` header, not the body** — the
///   route 400s without it and derives the sender from it alone.
/// - `X-Client-Wrapped: true` is required. Without it the cloud wraps the
///   message in its own `[JEKT:...]` marker before storing; the receiving srv
///   then wraps it *again* in `Handler::inject_message`, so the recipient sees
///   a doubled marker. Sending raw + telling the cloud not to wrap yields
///   exactly one marker, applied by the receiver — identical to how tiers 2
///   and 3 already behave.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn relay_inject(
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    source_agent: &str,
    target_agent: &str,
    message: &str,
    priority: &str,
) -> RelayOutcome {
    if message.len() > MAX_RELAY_MESSAGE_BYTES {
        return RelayOutcome::Failed(format!(
            "message is {} bytes; the cloud relay accepts at most {}",
            message.len(),
            MAX_RELAY_MESSAGE_BYTES
        ));
    }

    let url = format!("{}/reactive/inject", base_url.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Agent-ID", source_agent)
        .header("X-Client-Wrapped", "true")
        .timeout(std::time::Duration::from_secs(RELAY_TIMEOUT_SECS))
        .json(&serde_json::json!({
            "target_agent": target_agent,
            "message": message,
            "priority": priority,
        }))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return RelayOutcome::Failed(format!("cloud relay unreachable: {e}")),
    };

    let status = resp.status();
    if status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        // The route returns `success: true` alongside the id; treat an
        // explicit non-true as a failure rather than assuming 2xx means yes —
        // the same stricter reading tier 2/3 settled on.
        if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
            return RelayOutcome::Failed(format!(
                "cloud relay returned {status} without success:true"
            ));
        }
        return RelayOutcome::Queued {
            injection_id: body
                .get("injection_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        };
    }

    // 402 is the free-tier quota wall, which is a user-actionable condition
    // rather than a bug — surface the cloud's own message verbatim so the
    // upgrade URL it includes reaches the operator.
    let detail = resp.text().await.unwrap_or_default();
    RelayOutcome::Failed(format!("cloud relay rejected: HTTP {status} — {detail}"))
}

/// Resolve the credential to relay as, preferring one bound to this specific
/// sender over the shared account token — the same precedence the inbound
/// path (`cloud_subscriber::sync_agent_reactive`) already uses, so a
/// binding-enforced account behaves consistently in both directions.
///
/// `None` means the account isn't logged in to muxbus at all, i.e. tier 4
/// doesn't exist here.
///
/// **`store` must be the one muxbus credentials actually live in —
/// `AppState::id_store`,** the same store `CloudSubscriber::init_global` and
/// every `muxbus.login`/`status`/`disconnect` handler use. Passing the
/// per-channel `wstore` finds nothing whenever the shared root resolves (the
/// normal case), so tier 4 would silently never fire for a logged-in user
/// (reagent #3023 P0). Note this deliberately does NOT follow `AppState`'s
/// general steer toward `identity_store` for new muxbus call sites: the
/// credentials are written to `id_store`, and reading from a store the writer
/// doesn't use would reintroduce the same bug whenever
/// `isolated_auth_enabled()` redirects one but not the other.
///
/// Ordering matters. The shared account token is a purely local read, so it is
/// checked FIRST: no credential means the account isn't logged in, and there is
/// nothing for a per-agent credential to be provisioned against. Doing it the
/// other way round makes `ensure_agent_credential` attempt a cloud
/// provisioning round trip (`POST /agents/provision`) on a logged-out instance
/// — once per failed local inject, i.e. on the hot path of every message to an
/// unknown agent.
pub(crate) async fn relay_token(
    source_agent: &str,
    store: &Arc<Store>,
    http: &reqwest::Client,
) -> Option<String> {
    let scheduler = crate::broker::get_global()?;
    let shared = crate::muxbus::cloud_subscriber::load_valid_token(store, &scheduler).await?;

    match crate::muxbus::agent_credentials::ensure_agent_credential(source_agent, store, http).await
    {
        Some(per_agent) => Some(per_agent),
        None => Some(shared),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub relay capturing the request it receives, so the tests can assert
    /// on the wire contract (headers especially) and not just the outcome.
    struct Captured {
        agent_id: Option<String>,
        client_wrapped: Option<String>,
        authorization: Option<String>,
        body: serde_json::Value,
    }

    async fn stub_relay(
        status: axum::http::StatusCode,
        response_body: &'static str,
    ) -> (
        String,
        std::sync::Arc<std::sync::Mutex<Option<Captured>>>,
        tokio_util::sync::DropGuard,
    ) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = seen.clone();
        let app = axum::Router::new().route(
            "/reactive/inject",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, body: axum::Json<serde_json::Value>| {
                    let sink = sink.clone();
                    async move {
                        let get = |k: &str| {
                            headers
                                .get(k)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_string)
                        };
                        *sink.lock().unwrap() = Some(Captured {
                            agent_id: get("x-agent-id"),
                            client_wrapped: get("x-client-wrapped"),
                            authorization: get("authorization"),
                            body: body.0,
                        });
                        (
                            status,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            response_body,
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { child.cancelled().await })
                .await;
        });
        (format!("http://{addr}"), seen, token.drop_guard())
    }

    #[tokio::test]
    async fn a_queued_injection_returns_its_id() {
        let (url, _seen, _g) = stub_relay(
            axum::http::StatusCode::OK,
            r#"{"success":true,"injection_id":"inj-42"}"#,
        )
        .await;
        let out = relay_inject(
            &url,
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            "hello",
            "normal",
        )
        .await;
        match out {
            RelayOutcome::Queued { injection_id } => {
                assert_eq!(injection_id.as_deref(), Some("inj-42"))
            }
            other => panic!("expected Queued, got {other:?}"),
        }
    }

    /// The wire contract the cloud route actually enforces, pinned so a future
    /// edit can't quietly drop a header the server 400s (or double-wraps)
    /// without a test failing.
    #[tokio::test]
    async fn the_sender_travels_as_a_header_and_wrapping_is_declined() {
        let (url, seen, _g) =
            stub_relay(axum::http::StatusCode::OK, r#"{"success":true}"#).await;
        let _ = relay_inject(
            &url,
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            "hello",
            "urgent",
        )
        .await;

        let c = seen.lock().unwrap();
        let c = c.as_ref().expect("stub relay saw no request");
        assert_eq!(c.agent_id.as_deref(), Some("agent2"), "X-Agent-ID is required by the route");
        assert_eq!(
            c.client_wrapped.as_deref(),
            Some("true"),
            "without this the cloud wraps the marker and the receiver wraps it again"
        );
        assert_eq!(c.authorization.as_deref(), Some("Bearer tok"));
        // source_agent is NOT a body field — the route reads it from the header.
        assert!(c.body.get("source_agent").is_none());
        assert_eq!(c.body.get("target_agent").and_then(|v| v.as_str()), Some("clare"));
        assert_eq!(c.body.get("message").and_then(|v| v.as_str()), Some("hello"));
        assert_eq!(c.body.get("priority").and_then(|v| v.as_str()), Some("urgent"));
    }

    #[tokio::test]
    async fn a_quota_rejection_surfaces_the_clouds_own_message() {
        let (url, _seen, _g) = stub_relay(
            axum::http::StatusCode::PAYMENT_REQUIRED,
            r#"{"error":"quota_exceeded","upgrade_url":"https://cloud.agentmux.ai/billing"}"#,
        )
        .await;
        let out = relay_inject(
            &url,
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            "hello",
            "normal",
        )
        .await;
        match out {
            RelayOutcome::Failed(e) => {
                assert!(e.contains("402"), "status should be visible: {e}");
                assert!(e.contains("upgrade_url"), "cloud's own body should pass through: {e}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A 2xx that doesn't say `success: true` must not count — the same
    /// stricter reading the tier-2/3 forward helper settled on after the LAN
    /// tier was found treating a body without `success` as a delivery.
    #[tokio::test]
    async fn a_2xx_without_success_true_is_a_failure() {
        let (url, _seen, _g) =
            stub_relay(axum::http::StatusCode::OK, r#"{"injection_id":"inj-1"}"#).await;
        let out = relay_inject(
            &url,
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            "hello",
            "normal",
        )
        .await;
        assert!(matches!(out, RelayOutcome::Failed(_)), "got {out:?}");
    }

    /// Checked locally so the caller gets a useful message instead of a bare
    /// 400 after a wasted round trip.
    #[tokio::test]
    async fn an_oversized_message_is_rejected_before_any_request() {
        let big = "x".repeat(MAX_RELAY_MESSAGE_BYTES + 1);
        let out = relay_inject(
            "http://127.0.0.1:1", // never contacted
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            &big,
            "normal",
        )
        .await;
        match out {
            RelayOutcome::Failed(e) => assert!(e.contains("10240"), "{e}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreachable_relay_fails_rather_than_hanging() {
        let url = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let out = relay_inject(
            &url,
            &reqwest::Client::new(),
            "tok",
            "agent2",
            "clare",
            "hello",
            "normal",
        )
        .await;
        assert!(matches!(out, RelayOutcome::Failed(_)), "got {out:?}");
    }
}
