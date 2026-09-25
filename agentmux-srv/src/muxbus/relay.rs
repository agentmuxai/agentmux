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

/// The W3-S carried tuple (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.1):
/// the sender's WAN signature and the exact values it signed, so a receiver
/// can re-check it. Field names are the cloud's (`wan-keys.ts`
/// `WAN_CARRIED_FIELDS`); an old cloud silently drops them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct WanCarried {
    pub wan_sig: String,
    pub wan_msg_id: String,
    pub wan_ts_secs: i64,
    pub wan_source_agent: String,
    pub wan_target_agent: String,
    pub wan_source_host: String,
    pub wan_source_channel: String,
    pub wan_key_fp: String,
}

/// The cloud's caps (`wan-keys.ts`): signature ≤ 128 chars, identifiers ≤ 256.
const MAX_CARRIED_SIG_CHARS: usize = 128;
const MAX_CARRIED_ID_CHARS: usize = 256;

/// The relay carry gate (§2.1): the tuple rides the relay only if **every**
/// condition holds; otherwise the message is relayed exactly as before, with
/// nothing carried, so every gap degrades to "unsigned", never to a false
/// forgery alarm at the receiver. `Err` names the first condition that
/// failed, for debug logging only.
///
/// 1. The sender proved itself on this host (`sig_verified == Some(true)`):
///    a local process holding the auth key can't launder a captured
///    `wan_sig` through the relay. Do not drop this gate.
/// 2. The signed instance and channel are this install's own.
/// 3. The signature verifies locally under the agent's key in `wan.db`
///    (a key purged by an agent delete fails here).
/// 4. The directory is confirmed to hold that key (`published_fp`), so a
///    receiver can find it.
/// 5. Both ids are well-formed and every field is within the cloud's caps.
pub(crate) fn wan_carry_gate(
    req: &crate::backend::reactive::types::InjectionRequest,
    wan: Option<&crate::backend::storage::wan_identity::WanIdentityStore>,
    local_channel: &str,
) -> Result<WanCarried, &'static str> {
    use crate::backend::reactive::sanitize::validate_agent_id;

    let wan = wan.ok_or("no wan.db")?;
    let sig = req.wan_sig.as_deref().ok_or("unsigned")?;
    if req.sig_verified != Some(true) {
        return Err("sender not host-verified");
    }
    let source = req.source_agent.as_deref().ok_or("no source agent")?;
    let msg_id = req.request_id.as_deref().ok_or("no msgid")?;
    let ts_secs = req.ts_secs.ok_or("no signed timestamp")?;
    let host = req.wan_source_host.as_deref().ok_or("no instance")?;
    let channel = req.source_channel.as_deref().ok_or("no channel")?;

    // 2
    let instance = wan
        .instance_ensure(&crate::backend::reactive::registry::local_host_label())
        .map_err(|_| "no instance")?;
    if host != instance.instance_id {
        return Err("signed for another instance");
    }
    if channel != local_channel {
        return Err("signed for another channel");
    }
    // 3
    let key = wan.agent_key_load(source).ok().flatten().ok_or("no key in wan.db")?;
    let public = key.public_key_bytes().ok_or("malformed key")?;
    if !agentmux_common::jekt_sign::verify_wan_jekt(
        &public,
        msg_id,
        source,
        host,
        channel,
        &req.target_agent,
        ts_secs,
        &req.message,
        sig,
    ) {
        return Err("signature does not verify locally");
    }
    // 4
    if !key.is_published() {
        return Err("key not yet published");
    }
    // 5
    if !validate_agent_id(source) || !validate_agent_id(&req.target_agent) {
        return Err("malformed agent id");
    }
    if sig.len() > MAX_CARRIED_SIG_CHARS
        || [msg_id, host, channel].iter().any(|s| s.len() > MAX_CARRIED_ID_CHARS)
        || ts_secs <= 0
    {
        return Err("over the cloud's caps");
    }
    Ok(WanCarried {
        wan_sig: sig.to_string(),
        wan_msg_id: msg_id.to_string(),
        wan_ts_secs: ts_secs,
        wan_source_agent: source.to_string(),
        wan_target_agent: req.target_agent.clone(),
        wan_source_host: host.to_string(),
        wan_source_channel: channel.to_string(),
        wan_key_fp: agentmux_common::jekt_sign::wan_key_fingerprint(&public),
    })
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
    wan: Option<&WanCarried>,
) -> RelayOutcome {
    if message.len() > MAX_RELAY_MESSAGE_BYTES {
        return RelayOutcome::Failed(format!(
            "message is {} bytes; the cloud relay accepts at most {}",
            message.len(),
            MAX_RELAY_MESSAGE_BYTES
        ));
    }

    let url = format!("{}/reactive/inject", base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "target_agent": target_agent,
        "message": message,
        "priority": priority,
    });
    // W3-S: the carried tuple rides alongside, only when the carry gate
    // passed. Without it the body is byte-for-byte the old shape.
    if let (Some(wan), Some(obj)) = (wan, body.as_object_mut()) {
        if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(wan) {
            obj.extend(fields);
        }
    }
    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Agent-ID", source_agent)
        .header("X-Client-Wrapped", "true")
        .timeout(std::time::Duration::from_secs(RELAY_TIMEOUT_SECS))
        .json(&body)
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
/// per-channel `mstore` finds nothing whenever the shared root resolves (the
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
        )
        .await;
        assert!(matches!(out, RelayOutcome::Failed(_)), "got {out:?}");
    }

    // ── W3-S carry gate (§2.1) ──

    use crate::backend::reactive::types::InjectionRequest;
    use crate::backend::storage::wan_identity::WanIdentityStore;

    /// A wan.db with a published key for `camper`, and a request exactly as
    /// the MCP would send it: signed, host-verified, this instance.
    fn signed_request() -> (tempfile::TempDir, WanIdentityStore, InjectionRequest) {
        use base64::Engine as _;
        let dir = tempfile::tempdir().unwrap();
        let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
        let instance = wan.instance_ensure("narko").unwrap();
        let key = wan.agent_key_ensure("camper", None).unwrap();
        wan.agent_key_mark_published("camper", &key.public_key).unwrap();
        let private = base64::engine::general_purpose::STANDARD.decode(&key.private_key).unwrap();
        let sig = agentmux_common::jekt_sign::sign_wan_jekt(
            &private, "msg-1", "camper", &instance.instance_id, "stable", "agent2", 1_790_000_000, "hello",
        )
        .unwrap();
        let req = InjectionRequest {
            target_agent: "agent2".into(),
            message: "hello".into(),
            source_agent: Some("camper".into()),
            request_id: Some("msg-1".into()),
            ts_secs: Some(1_790_000_000),
            sig_verified: Some(true),
            wan_sig: Some(sig),
            wan_source_host: Some(instance.instance_id.clone()),
            source_channel: Some("stable".into()),
            ..Default::default()
        };
        (dir, wan, req)
    }

    #[test]
    fn a_signed_host_verified_published_message_is_carried_as_signed() {
        let (_dir, wan, req) = signed_request();
        let carried = wan_carry_gate(&req, Some(&wan), "stable").expect("every condition holds");
        let key = wan.agent_key_load("camper").unwrap().unwrap();
        assert_eq!(carried.wan_sig, req.wan_sig.clone().unwrap());
        assert_eq!(carried.wan_msg_id, "msg-1");
        assert_eq!(carried.wan_ts_secs, 1_790_000_000);
        assert_eq!(carried.wan_source_agent, "camper");
        assert_eq!(carried.wan_target_agent, "agent2");
        assert_eq!(carried.wan_source_channel, "stable");
        assert_eq!(
            carried.wan_key_fp,
            agentmux_common::jekt_sign::wan_key_fingerprint(&key.public_key_bytes().unwrap())
        );
        // What a receiver will reconstruct verifies against the published record.
        let instance = wan.instance_ensure("narko").unwrap();
        let record = crate::muxbus::wan_publish::certify(&instance, "camper", "stable", &key).unwrap();
        let view = agentmux_common::jekt_sign::WanCarried {
            sig: &carried.wan_sig,
            msg_id: &carried.wan_msg_id,
            ts_secs: carried.wan_ts_secs,
            source_agent: &carried.wan_source_agent,
            target_agent: &carried.wan_target_agent,
            source_host: &carried.wan_source_host,
            source_channel: &carried.wan_source_channel,
            key_fp: &carried.wan_key_fp,
        };
        assert_eq!(agentmux_common::jekt_sign::verify_wan_against_record(&view, &record, "hello"), Ok(()));
    }

    #[test]
    fn each_carry_condition_failing_in_turn_sends_the_message_unsigned() {
        type Mutation = Box<dyn Fn(&mut InjectionRequest, &WanIdentityStore)>;
        let cases: Vec<(&str, Mutation)> = vec![
            ("unsigned", Box::new(|r, _| r.wan_sig = None)),
            // 1: a local process with the auth key but not the agent's host key.
            ("not host-verified", Box::new(|r, _| r.sig_verified = None)),
            ("host verification failed", Box::new(|r, _| r.sig_verified = Some(false))),
            // 2: an agent spawned before D1 signs its hostname, not the instance id.
            ("hostname instead of instance id", Box::new(|r, _| r.wan_source_host = Some("narko".into()))),
            ("another channel", Box::new(|r, _| r.source_channel = Some("beta".into()))),
            // 3
            ("tampered message", Box::new(|r, _| r.message = "hello!".into())),
            ("retargeted", Box::new(|r, _| r.target_agent = "lark".into())),
            ("key purged by an agent delete", Box::new(|_, w| {
                w.agent_keys_delete(&["camper".to_string()]).unwrap();
            })),
            // 4: re-minted key, not yet published.
            ("key not published", Box::new(|r, w| {
                use base64::Engine as _;
                w.agent_keys_delete(&["camper".to_string()]).unwrap();
                let key = w.agent_key_ensure("camper", None).unwrap();
                let private = base64::engine::general_purpose::STANDARD.decode(&key.private_key).unwrap();
                r.wan_sig = agentmux_common::jekt_sign::sign_wan_jekt(
                    &private, "msg-1", "camper", r.wan_source_host.as_deref().unwrap(), "stable", "agent2",
                    1_790_000_000, "hello",
                );
            })),
            ("no signed timestamp", Box::new(|r, _| r.ts_secs = None)),
            ("no msgid", Box::new(|r, _| r.request_id = None)),
        ];
        for (name, mutate) in cases {
            let (_dir, wan, mut req) = signed_request();
            mutate(&mut req, &wan);
            assert!(wan_carry_gate(&req, Some(&wan), "stable").is_err(), "{name} was carried");
        }
        // And with no wan.db at all.
        let (_dir, _wan, req) = signed_request();
        assert_eq!(wan_carry_gate(&req, None, "stable"), Err("no wan.db"));
    }

    #[tokio::test]
    async fn the_carried_tuple_rides_the_body_and_its_absence_keeps_the_old_shape() {
        let (url, seen, _g) = stub_relay(axum::http::StatusCode::OK, r#"{"success":true}"#).await;
        let (_dir, wan, req) = signed_request();
        let carried = wan_carry_gate(&req, Some(&wan), "stable").unwrap();
        let _ = relay_inject(&url, &reqwest::Client::new(), "tok", "camper", "agent2", "hello", "normal", Some(&carried))
            .await;
        {
            let c = seen.lock().unwrap();
            let body = &c.as_ref().unwrap().body;
            for field in [
                "wan_sig", "wan_msg_id", "wan_ts_secs", "wan_source_agent", "wan_target_agent",
                "wan_source_host", "wan_source_channel", "wan_key_fp",
            ] {
                assert!(body.get(field).is_some(), "{field} missing from the relay body");
            }
            assert_eq!(body["wan_ts_secs"], serde_json::json!(1_790_000_000));
        }

        let _ = relay_inject(&url, &reqwest::Client::new(), "tok", "camper", "agent2", "hello", "normal", None).await;
        let c = seen.lock().unwrap();
        let keys: std::collections::BTreeSet<_> =
            c.as_ref().unwrap().body.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["message", "priority", "target_agent"].into_iter().map(String::from).collect(),
            "unsigned relays keep exactly the old body"
        );
    }
}
