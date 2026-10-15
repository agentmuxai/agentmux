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

/// The cloud rejects anything larger (10 KB maximum). Checked locally so an oversized message fails with a
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

/// What the desktop's relay calls send as `User-Agent`, so the relay can tell
/// app versions apart (reqwest sends none of its own).
pub(crate) const USER_AGENT: &str = concat!("agentmux/", env!("CARGO_PKG_VERSION"));

/// The most agents one batch request may name (the relay's own limit).
pub(crate) const MAX_BATCH_AGENTS: usize = 50;

/// How long to stop asking a relay that answered 404 to a batch route
/// (`/reactive/pending-batch`, `/agents/lease/renew-batch`): it predates them.
const BATCH_ROUTES_RETRY: std::time::Duration = std::time::Duration::from_secs(600);

static BATCH_ROUTES_MISSING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> =
    std::sync::LazyLock::new(Default::default);

/// Whether the relay at `base` recently answered 404 to a batch route. Its
/// callers then ask per agent, as before the batch routes existed. Keyed by
/// relay, so a relay without them never switches them off for another.
pub(crate) fn batch_routes_missing(base: &str) -> bool {
    BATCH_ROUTES_MISSING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(base)
        .is_some_and(|until| *until > std::time::Instant::now())
}

/// Note a 404 from the relay at `base` to a batch route.
pub(crate) fn note_batch_routes_missing(base: &str) {
    tracing::info!("relay has no batch routes yet: asking per agent for now");
    BATCH_ROUTES_MISSING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(base.to_string(), std::time::Instant::now() + BATCH_ROUTES_RETRY);
}

/// The W3-S carried tuple (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.1):
/// the sender's WAN signature and the exact values it signed, so a receiver
/// can re-check it. Field names are the cloud's; an old cloud silently
/// drops them.
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

/// The cloud's caps: signature ≤ 128 chars, identifiers ≤ 256.
const MAX_CARRIED_SIG_CHARS: usize = 128;
const MAX_CARRIED_ID_CHARS: usize = 256;

/// Sign a WAN jekt at the instance that relays it, for the agent its token
/// names (`docs/specs/SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md` §3). The
/// tuple rides the relay only if **every** condition holds; otherwise the
/// message is relayed exactly as before, with nothing carried, so every gap
/// degrades to "unsigned", never to a false forgery alarm at the receiver.
/// `Err` names the first condition that failed, for the "queued" log line.
///
/// 1. `signer` is the jekt's `source_agent`, established by the caller from
///    the request's own agent token (`actor::uid_has_slug`), and only at the
///    origin: a forwarded hop has no token, so `None`. Do not drop this gate:
///    without it, any local process holding the instance key could have srv
///    sign as any agent.
/// 2. `wan.db` holds the agent's key, and the directory this relay sends
///    into (this relay plus the signed-in account,
///    `wan_publish::directory_id`) is confirmed to hold it, so a receiver can
///    find it. A key published under another account or relay doesn't count.
/// 3. Both ids are well-formed and every field is within the cloud's caps.
///
/// Every signed field but the message and target is srv's own: its instance,
/// its channel, its clock. A `wan_sig`, host or timestamp in the request body
/// is not read (agents signed these themselves before; see the spec §2).
pub(crate) fn wan_sign_at_origin(
    req: &crate::backend::reactive::types::InjectionRequest,
    signer: Option<&str>,
    wan: Option<&crate::backend::storage::wan_identity::WanIdentityStore>,
    directory: Option<&str>,
    local_channel: &str,
    now_secs: i64,
) -> Result<WanCarried, &'static str> {
    use crate::backend::reactive::sanitize::validate_agent_id;

    let wan = wan.ok_or("no wan.db")?;
    // 1
    let source = req.source_agent.as_deref().filter(|s| !s.is_empty()).ok_or("no source agent")?;
    if signer != Some(source) {
        return Err(if signer.is_none() { "no agent token" } else { "token names another agent" });
    }
    // The caller's id when it sent one (the MCP does), else a fresh one: the
    // receiver's replay check only needs it unique per sender.
    let minted;
    let msg_id = match req.request_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => {
            minted = uuid::Uuid::new_v4().to_string();
            &minted
        }
    };
    // 2
    let key = wan.agent_key_load(source).ok().flatten().ok_or("no key in wan.db")?;
    let directory = directory.ok_or("no account to publish under")?;
    if !wan.agent_key_is_published_to(source, &key, directory).unwrap_or(false) {
        return Err("key not published to this directory");
    }
    // 3
    if !validate_agent_id(source) || !validate_agent_id(&req.target_agent) {
        return Err("malformed agent id");
    }
    let instance = wan
        .instance_ensure(&crate::backend::reactive::registry::local_host_label())
        .map_err(|_| "no instance")?;
    if [msg_id, &instance.instance_id, local_channel].iter().any(|s| s.len() > MAX_CARRIED_ID_CHARS) || now_secs <= 0 {
        return Err("over the cloud's caps");
    }
    let public = key.public_key_bytes().ok_or("malformed key")?;
    let private = agentmux_common::jekt_sign::decode_key(&key.private_key).ok_or("malformed key")?;
    let sig = agentmux_common::jekt_sign::sign_wan_jekt(
        &private,
        msg_id,
        source,
        &instance.instance_id,
        local_channel,
        &req.target_agent,
        now_secs,
        &req.message,
    )
    .ok_or("malformed key")?;
    if sig.len() > MAX_CARRIED_SIG_CHARS {
        return Err("over the cloud's caps");
    }
    Ok(WanCarried {
        wan_sig: sig,
        wan_msg_id: msg_id.to_string(),
        wan_ts_secs: now_secs,
        wan_source_agent: source.to_string(),
        wan_target_agent: req.target_agent.clone(),
        wan_source_host: instance.instance_id,
        wan_source_channel: local_channel.to_string(),
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
    Queued {
        injection_id: Option<String>,
        /// Whether the target is one of the sender's own account's agents,
        /// when the relay says. `Some(false)` is a likely
        /// typo: the relay accepts any name.
        target_in_account: Option<bool>,
    },
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
/// ## Contract (verified against the cloud relay)
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
    files: Option<&[agentmux_common::jekt_files::CloudFile]>,
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
    // Files sent with it, uploaded already (server::jekt_cloud).
    if let (Some(files), Some(obj)) = (files, body.as_object_mut()) {
        obj.insert("files".to_string(), serde_json::to_value(files).unwrap_or_default());
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
            target_in_account: body.get("target_in_account").and_then(|v| v.as_bool()),
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
///
/// Returns the account too, read from the same shared token: the carry gate
/// needs the directory this send goes into, and reading the account
/// separately would let a sign-in in between pair one account's token with
/// another's directory (Codex P2 on #3994).
pub(crate) async fn relay_token(
    source_agent: &str,
    store: &Arc<Store>,
    http: &reqwest::Client,
) -> Option<RelayCredential> {
    let scheduler = crate::broker::get_global()?;
    let shared = crate::muxbus::cloud_subscriber::load_valid_token(store, &scheduler).await?;
    let account_sub = crate::muxbus::pkce::token_sub(&shared);

    let token = match crate::muxbus::agent_credentials::ensure_agent_credential(source_agent, store, http).await {
        Some(per_agent) => per_agent,
        None => shared,
    };
    // `ensure_agent_credential` loads the account token itself when it
    // provisions, so a sign-in to another account in between could hand back
    // the new account's per-agent token while `account_sub` is the old one's
    // (Codex P2 on #3994). Look again; if the account moved, bind no account
    // and the send goes unsigned rather than into the wrong directory.
    let after = crate::muxbus::cloud_subscriber::load_valid_token(store, &scheduler)
        .await
        .map(|t| crate::muxbus::pkce::token_sub(&t));
    Some(RelayCredential { token, account_sub: settled_account(account_sub, after) })
}

/// The account to bind a send to: the one its credential was resolved
/// under, if it's still the signed-in one afterwards; empty otherwise.
fn settled_account(before: String, after: Option<String>) -> String {
    if after.as_deref() == Some(before.as_str()) {
        before
    } else {
        String::new()
    }
}

/// What [`relay_token`] resolved: the bearer to send with, and the account
/// (`sub`) of the shared token it was resolved under — empty if that token
/// names none, which leaves the carry gate without a directory (unsigned).
pub(crate) struct RelayCredential {
    pub token: String,
    pub account_sub: String,
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
            None, None,
        )
        .await;
        match out {
            RelayOutcome::Queued { injection_id, target_in_account } => {
                assert_eq!(injection_id.as_deref(), Some("inj-42"));
                // An older relay says nothing about the account.
                assert_eq!(target_in_account, None)
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
            None, None,
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
            None, None,
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

    /// The relay says whether the target is one of the
    /// sender's own account's agents; `false` reaches `SendMessage` as a
    /// likely-typo hint.
    #[tokio::test]
    async fn a_queued_injection_carries_whether_the_target_is_the_senders() {
        for (body, want) in [
            (r#"{"success":true,"injection_id":"inj-7","target_in_account":false}"#, Some(false)),
            (r#"{"success":true,"injection_id":"inj-8","target_in_account":true}"#, Some(true)),
        ] {
            let (url, _seen, _g) = stub_relay(axum::http::StatusCode::OK, body).await;
            let out = relay_inject(&url, &reqwest::Client::new(), "tok", "agent2", "agnetx", "hello", "normal", None, None).await;
            match out {
                RelayOutcome::Queued { target_in_account, .. } => assert_eq!(target_in_account, want, "{body}"),
                other => panic!("expected Queued, got {other:?}"),
            }
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
            None, None,
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
            None, None,
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
            None, None,
        )
        .await;
        assert!(matches!(out, RelayOutcome::Failed(_)), "got {out:?}");
    }

    // ── W3-S carry gate (§2.1) ──

    use crate::backend::reactive::types::InjectionRequest;
    use crate::backend::storage::wan_identity::WanIdentityStore;

    /// A wan.db with a published key for `camper`, and a request exactly as
    /// the MCP would send it: signed, host-verified, this instance.
    const TEST_DIR: &str = "https://relay.test#acct";

    #[test]
    fn a_send_is_bound_to_an_account_only_if_it_held_through_resolution() {
        // Codex P2 on #3994: a sign-in to another account while the per-agent
        // credential resolves can pair one account's token with the other's
        // directory. Then the send gets no account, so it goes unsigned.
        assert_eq!(settled_account("acct-a".into(), Some("acct-a".into())), "acct-a");
        assert_eq!(settled_account("acct-a".into(), Some("acct-b".into())), "");
        assert_eq!(settled_account("acct-a".into(), None), "", "logged out mid-way");
    }

    const NOW: i64 = 1_790_000_000;

    /// A jekt from "camper" to "agent2", as srv holds it at the origin, with
    /// camper's key in wan.db and published.
    fn origin_request() -> (tempfile::TempDir, WanIdentityStore, InjectionRequest) {
        let dir = tempfile::tempdir().unwrap();
        let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
        let key = wan.agent_key_ensure("camper", None).unwrap();
        wan.agent_key_mark_published("camper", &key.public_key, TEST_DIR).unwrap();
        let req = InjectionRequest {
            target_agent: "agent2".into(),
            message: "hello".into(),
            source_agent: Some("camper".into()),
            request_id: Some("msg-1".into()),
            ..Default::default()
        };
        (dir, wan, req)
    }

    #[test]
    fn srv_signs_for_the_agent_its_token_names_and_a_receiver_can_check_it() {
        let (_dir, wan, req) = origin_request();
        let carried = wan_sign_at_origin(&req, Some("camper"), Some(&wan), Some(TEST_DIR), "stable", NOW)
            .expect("every condition holds");
        let instance = wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).unwrap();
        let key = wan.agent_key_load("camper").unwrap().unwrap();
        assert_eq!(carried.wan_msg_id, "msg-1");
        assert_eq!(carried.wan_ts_secs, NOW, "srv's clock");
        assert_eq!(carried.wan_source_agent, "camper");
        assert_eq!(carried.wan_target_agent, "agent2");
        assert_eq!(carried.wan_source_host, instance.instance_id, "srv's own instance");
        assert_eq!(carried.wan_source_channel, "stable", "srv's own channel");
        assert_eq!(
            carried.wan_key_fp,
            agentmux_common::jekt_sign::wan_key_fingerprint(&key.public_key_bytes().unwrap())
        );
        // What a receiver will reconstruct verifies against the published record.
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
        assert!(
            agentmux_common::jekt_sign::verify_wan_against_record(&view, &record, "hello!").is_err(),
            "the signature covers the message"
        );
    }

    #[test]
    fn a_request_with_no_id_is_signed_under_a_fresh_one() {
        // `muxsh agent send` sends none.
        let (_dir, wan, mut req) = origin_request();
        req.request_id = None;
        let a = wan_sign_at_origin(&req, Some("camper"), Some(&wan), Some(TEST_DIR), "stable", NOW).unwrap();
        let b = wan_sign_at_origin(&req, Some("camper"), Some(&wan), Some(TEST_DIR), "stable", NOW).unwrap();
        assert!(!a.wan_msg_id.is_empty());
        assert_ne!(a.wan_msg_id, b.wan_msg_id, "unique per send, so the receiver's replay check passes");
    }

    #[test]
    fn what_a_caller_puts_in_the_body_is_not_what_srv_signs() {
        let (_dir, wan, mut req) = origin_request();
        // An agent (or an older MCP) supplying its own signature, instance,
        // channel and time: none of it is read.
        req.wan_sig = Some("forged".into());
        req.wan_source_host = Some("another-instance".into());
        req.source_channel = Some("beta".into());
        req.ts_secs = Some(1);
        let carried = wan_sign_at_origin(&req, Some("camper"), Some(&wan), Some(TEST_DIR), "stable", NOW).unwrap();
        assert_ne!(carried.wan_sig, "forged");
        assert_ne!(carried.wan_source_host, "another-instance");
        assert_eq!((carried.wan_source_channel.as_str(), carried.wan_ts_secs), ("stable", NOW));
    }

    #[test]
    fn each_condition_failing_in_turn_sends_the_message_unsigned() {
        let sign = |req: &InjectionRequest, signer: Option<&str>, wan: Option<&WanIdentityStore>, dir: Option<&str>| {
            wan_sign_at_origin(req, signer, wan, dir, "stable", NOW).err()
        };
        let (_dir, wan, req) = origin_request();
        // 1: no token (a terminal's muxsh, a script, a forwarded hop) ...
        assert_eq!(sign(&req, None, Some(&wan), Some(TEST_DIR)), Some("no agent token"));
        // ... or a token naming another agent than the jekt's source.
        assert_eq!(sign(&req, Some("agent5"), Some(&wan), Some(TEST_DIR)), Some("token names another agent"));
        let mut sourceless = req.clone();
        sourceless.source_agent = None;
        assert_eq!(sign(&sourceless, Some("camper"), Some(&wan), Some(TEST_DIR)), Some("no source agent"));
        // 2
        assert_eq!(sign(&req, Some("camper"), None, Some(TEST_DIR)), Some("no wan.db"));
        assert_eq!(sign(&req, Some("camper"), Some(&wan), None), Some("no account to publish under"));
        assert_eq!(
            sign(&req, Some("camper"), Some(&wan), Some("https://relay.test#another-account")),
            Some("key not published to this directory")
        );
        wan.agent_keys_delete(&["camper".to_string()]).unwrap();
        assert_eq!(sign(&req, Some("camper"), Some(&wan), Some(TEST_DIR)), Some("no key in wan.db"));
        // A re-minted key isn't published yet.
        wan.agent_key_ensure("camper", None).unwrap();
        assert_eq!(
            sign(&req, Some("camper"), Some(&wan), Some(TEST_DIR)),
            Some("key not published to this directory")
        );
        // 3
        let (_dir, wan, mut bad) = origin_request();
        bad.target_agent = "not a name!".into();
        assert_eq!(sign(&bad, Some("camper"), Some(&wan), Some(TEST_DIR)), Some("malformed agent id"));
    }

    #[tokio::test]
    async fn the_carried_tuple_rides_the_body_and_its_absence_keeps_the_old_shape() {
        let (url, seen, _g) = stub_relay(axum::http::StatusCode::OK, r#"{"success":true}"#).await;
        let (_dir, wan, req) = origin_request();
        let carried = wan_sign_at_origin(&req, Some("camper"), Some(&wan), Some(TEST_DIR), "stable", NOW).unwrap();
        let _ = relay_inject(&url, &reqwest::Client::new(), "tok", "camper", "agent2", "hello", "normal", Some(&carried), None)
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
            assert_eq!(body["wan_ts_secs"], serde_json::json!(NOW));
        }

        let _ = relay_inject(&url, &reqwest::Client::new(), "tok", "camper", "agent2", "hello", "normal", None, None).await;
        let c = seen.lock().unwrap();
        let keys: std::collections::BTreeSet<_> =
            c.as_ref().unwrap().body.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["message", "priority", "target_agent"].into_iter().map(String::from).collect(),
            "unsigned relays keep exactly the old body"
        );
    }

    /// Files uploaded for a jekt ride its body as the manifest, keys and blob
    /// ids included (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §4.4).
    #[tokio::test]
    async fn a_cloud_manifest_rides_the_body() {
        let (url, seen, _g) = stub_relay(axum::http::StatusCode::OK, r#"{"success":true}"#).await;
        let files = vec![agentmux_common::jekt_files::CloudFile {
            file: agentmux_common::jekt_files::JektFile { sha256: "ab".repeat(32), name: "shot.png".into(), size: 3 },
            key: "A".repeat(43) + "=",
            blob_id: "blob-0123abcd-0000-4000-8000-0123456789ab".into(),
        }];
        let _ = relay_inject(&url, &reqwest::Client::new(), "tok", "camper", "agent2", "hello", "normal", None, Some(&files)).await;
        let c = seen.lock().unwrap();
        let body = &c.as_ref().unwrap().body;
        assert_eq!(body["files"][0]["blob_id"], "blob-0123abcd-0000-4000-8000-0123456789ab");
        assert_eq!(body["files"][0]["sha256"], "ab".repeat(32), "the file's fields sit beside key and blob_id");
        assert_eq!(body["files"][0]["key"], "A".repeat(43) + "=");
    }
}
