// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::reactive::{registry as agent_registry, AgentRegistration, InjectionRequest};
use crate::backend::{base, subagent_watcher};

use super::AppState;

/// Max cross-instance forwards one inject may take (Tiers 2a/2b/3 each add one). A real delivery is
/// one hop; the cap only stops two channels that hold stale-but-alive registry entries for the same
/// agent from forwarding it back and forth forever (#2350).
const MAX_FORWARD_HOPS: u8 = 3;

pub(super) use super::jekt_echo::{echo_jekt_to_sender, EchoTrust};
use super::jekt_echo::{echo_jekt_to_sender_with_files, echo_trust_from_peer_body};

/// One forward candidate: where to POST, how to authenticate, and what the
/// echoed marker should claim about the hop if it lands.
pub(super) struct ForwardPeer<'a> {
    /// Base URL of the peer srv (no path), e.g. `http://127.0.0.1:1234`.
    pub url: &'a str,
    /// `X-AuthKey` value; skipped entirely when empty.
    pub auth_key: &'a str,
    /// Which tier resolved this candidate — log field only.
    pub kind: &'a str,
    /// Channel the candidate came from, for the cross-channel registry tier.
    /// `None` where the notion doesn't apply.
    pub channel: Option<&'a str>,
    pub trust: EchoTrust<'a>,
}

/// What one forward attempt concluded. The distinction that matters is
/// [`Stale`](ForwardOutcome::Stale) vs
/// [`Inconclusive`](ForwardOutcome::Inconclusive): only the former is evidence
/// about the *candidate*, and only the former may drive an eviction.
pub(super) enum ForwardOutcome {
    /// The peer delivered it. The sender echo has already been emitted; the
    /// caller should return this body verbatim.
    Delivered(serde_json::Value),
    /// The peer answered without delivering (`success` was not `true`), or was
    /// unreachable at the transport level. Both are consistent with a registry
    /// entry or discovery-cache entry pointing at something that is no longer
    /// there — but neither *proves* it (a startup race looks identical), so
    /// each tier applies its own eviction policy on top of this.
    Stale,
    /// Something failed that says nothing about whether the candidate is
    /// alive — a non-2xx status, or a body that wasn't JSON. **Never evict on
    /// this**; fall through to the next candidate/tier and leave the entry be.
    Inconclusive,
    /// The peer refused the jekt's files for a reason no other candidate
    /// would change (`jekt_files::refused`): end the search with its answer.
    Refused(serde_json::Value),
    /// The request went out and no answer came in time (a peer that polls fine but does not take the message).
    NoAnswer,
}

/// POSTs `forwarded_req` to a peer srv and echoes it to the sender if it lands: the one
/// implementation every forwarding tier shares. As three copies, a tier that forgot the echo or
/// passed `delivery_tier` wrong failed silently and differently from its siblings
/// (`docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md` §4).
///
/// Delivered means `success: true` in the body, in every tier. The LAN tier used to accept a body
/// that omitted `success`, which echoed a delivery confirmation for a message that may never have
/// arrived. A real peer always sends it (`InjectionResponse::success` has no
/// `skip_serializing_if`).
pub(super) async fn forward_inject_to_peer(
    state: &AppState,
    req: &InjectionRequest,
    forwarded_req: &InjectionRequest,
    peer: ForwardPeer<'_>,
) -> ForwardOutcome {
    // A jekt with files carries them, after it (file transfer spec §4).
    let route = if forwarded_req.files.is_some() { "inject-files" } else { "inject" };
    let forward_url = format!("{}/agentmux/reactive/{route}", peer.url);
    let channel = peer.channel.unwrap_or("-");
    tracing::debug!(
        target = %req.target_agent,
        url = %forward_url,
        kind = peer.kind,
        channel,
        "inject forward"
    );

    let mut fwd = state.http_client.post(&forward_url);
    fwd = if forwarded_req.files.is_some() {
        match super::jekt_files::transfer_body(forwarded_req).await {
            Ok(body) => fwd.body(body).timeout(super::jekt_files::TRANSFER_TIMEOUT),
            Err(e) => return ForwardOutcome::Refused(super::jekt_files::refused(req, e)),
        }
    } else {
        fwd.json(forwarded_req)
    };
    if !peer.auth_key.is_empty() {
        fwd = fwd.header(AUTH_KEY_HEADER, peer.auth_key);
    }

    match fwd.send().await {
        Ok(r) if r.status().is_success() => {
            let Ok(body) = r.json::<serde_json::Value>().await else {
                tracing::warn!(
                    target = %req.target_agent,
                    url = %forward_url,
                    kind = peer.kind,
                    channel,
                    "inject forward: 2xx with an unparseable body — not evicting"
                );
                return ForwardOutcome::Inconclusive;
            };
            if super::jekt_hold::peer_refused(&body) {
                return ForwardOutcome::Refused(body);
            }
            if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
                tracing::warn!(
                    target = %req.target_agent,
                    url = %forward_url,
                    kind = peer.kind,
                    channel,
                    "inject forward: peer did not deliver — candidate may be stale"
                );
                return ForwardOutcome::Stale;
            }
            echo_jekt_to_sender_with_files(
                state,
                req.source_agent.as_deref(),
                &req.target_agent,
                &req.message,
                body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                body.get("effective_tier").and_then(|v| v.as_str()),
                body.get("requires_stop").and_then(|v| v.as_bool()),
                echo_trust_from_peer_body(&body, peer.trust),
                req.priority.as_deref().unwrap_or("normal"),
                req.files_delivery.as_ref(),
            );
            ForwardOutcome::Delivered(body)
        }
        Ok(r) => {
            tracing::warn!(
                target = %req.target_agent,
                status = %r.status(),
                url = %forward_url,
                kind = peer.kind,
                channel,
                "inject forward: non-success HTTP status — not evicting"
            );
            ForwardOutcome::Inconclusive
        }
        Err(e) => {
            tracing::warn!(target = %req.target_agent, error = %e, url = %forward_url, kind = peer.kind, channel, timed_out = e.is_timeout(), "inject forward: transport failure, candidate may be stale");
            if e.is_timeout() { ForwardOutcome::NoAnswer } else { ForwardOutcome::Stale }
        }
    }
}

/// Max age (seconds) of a signed jekt's `ts_secs` for it to verify
/// (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, anti-replay, #2565). Without the check a
/// captured valid signature verified forever. Wide enough for same-machine latency and clock skew,
/// narrow enough to bound replay.
const JEKT_SIG_MAX_AGE_SECS: i64 = 300;

use agentmux_common::{time::now_secs as now_unix_secs, AUTH_KEY_HEADER};

/// Host-tier jekt sender verification (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2): sets
/// `req.sig_verified` by checking `req.jekt_sig` against the claimed `source_agent`'s stored
/// signing key, inside the freshness window.
///
/// **Every entry point that builds a host-tier `InjectionRequest` from client-supplied fields MUST
/// call this before `Handler::inject_message`** (#2565): `messagebus.rs::handle_inject` and
/// `websocket.rs`'s `bus:inject` once skipped it and rendered `TRUST=self-declared` for an
/// impersonated sender.
///
/// **Deliberately not gated on `delivery_tier == "host"`** (#2588). `delivery_tier` is
/// self-declared by any caller holding the full local `auth_key` (see `resolve_delivery_tier`), so
/// gating on it let a request claiming `"lan"` or `"wan"` dodge this check and impersonate a
/// locally known agent. Ungated, it acts only when `agent_jekt_key_load` finds a LOCAL key for the
/// claimed sender: never for a genuine remote LAN/WAN sender, always for an impersonation of an
/// agent this instance knows.
pub(super) fn verify_jekt_signature(state: &AppState, req: &mut InjectionRequest) {
    let Some(claimed) = req.source_agent.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    if super::actor::jekt_sender_by_token(&state.mstore, req, &claimed) {
        req.sig_verified = Some(true); // identity M4d-4
        return;
    }
    if former_slug::refuse_former_sender(state, req) {
        return;
    }
    let Ok(Some(key)) = state.mstore.agent_jekt_key_load(&claimed) else {
        return;
    };
    crate::backend::agent_resolve::record_uid_fallback("m4d.host_hmac_by_name");
    let msgid = req.request_id.clone().unwrap_or_default();
    let ts = req.ts_secs.unwrap_or(0);
    let within_freshness_window =
        ts > 0 && (now_unix_secs() - ts).abs() <= JEKT_SIG_MAX_AGE_SECS;
    let verified = within_freshness_window
        && req.jekt_sig.as_deref().map_or(false, |sig| {
            agentmux_common::jekt_sign::verify_jekt(
                &key, &msgid, &claimed, &req.target_agent, ts, &req.message, sig,
            )
        });
    if !verified { tracing::warn!(agent_id = %claimed, fresh = within_freshness_window, "host jekt signature missing or did not verify"); }
    req.sig_verified = Some(verified);
}

/// Anti-replay window for `req.reagent_ts_secs`: WAN-scoped, so wider than host-tier's 300 s
/// because it covers real network latency. Same value as
/// `cloud_subscriber::REAGENT_SIG_MAX_AGE_SECS`.
const REAGENT_SIG_MAX_AGE_SECS: i64 = 600;

/// WAN-tier reagent-signature check for the HTTP `/agentmux/reactive/inject` path, mirroring
/// `cloud_subscriber::sync_agent_reactive` for the WS path. Before it, nothing on this path read
/// `reagent_sig`/`reagent_key_id`, so a signed notification arrived effectively unsigned and never
/// rendered `SIG=verified` (#41).
///
/// Only for `delivery_tier == "wan"`. A partial set of the four fields reads as "not signed"
/// (`reagent_verified` stays `None`), as in `cloud_subscriber.rs`: a legitimate sender sends all
/// four, and a stripped signature looks exactly like an absent one, so it buys an attacker nothing.
///
/// Takes `now` so a test can hold its fixed offline-signed fixture inside the window: the private
/// half of the pinned Ed25519 key isn't in this repo (same as
/// `cloud_subscriber::reagent_sig_is_fresh`).
pub(super) fn verify_reagent_signature(req: &mut InjectionRequest, now: i64) {
    if req.delivery_tier.as_deref() != Some("wan") {
        return;
    }
    let (Some(sig), Some(key_id), Some(msg_id), Some(ts_secs)) =
        (req.reagent_sig.as_deref(), req.reagent_key_id.as_deref(), req.reagent_msg_id.as_deref(), req.reagent_ts_secs)
    else {
        return;
    };
    let within_freshness_window = ts_secs > 0 && (now - ts_secs).abs() <= REAGENT_SIG_MAX_AGE_SECS;
    let verified = within_freshness_window
        && agentmux_common::jekt_sign::verify_trusted_reagent_jekt(
            key_id,
            msg_id,
            req.source_agent.as_deref().unwrap_or(""),
            &req.target_agent,
            ts_secs,
            &req.message,
            sig,
        );
    req.reagent_verified = Some(verified);
}

/// Anti-replay window for `lan_sig`: the WAN value, since LAN crosses a real network hop (mDNS plus
/// an HTTP round trip), not a same-machine call.
const LAN_SIG_MAX_AGE_SECS: i64 = REAGENT_SIG_MAX_AGE_SECS;

/// LAN-tier per-agent Ed25519 signature check (docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md
/// §2.4). Async because the claimed sender's public key lives on whichever peer hosts that agent: a
/// LAN round trip (`LanDiscoveryController::find_agent_lan_pubkey`), unlike
/// `verify_jekt_signature`'s local lookup.
///
/// Only for `delivery_tier == "lan"`; `handle_reactive_inject` has already overridden the tier
/// server-side (spec §3), so this never runs off a request that wasn't `lan_key`-authenticated.
///
/// No `lan_sig`, or no peer has the sender's public key: `lan_verified` stays `None` ("nothing to
/// check against", as for `sig_verified`/`reagent_verified`). A `lan_sig` with a key found that
/// doesn't verify: `Some(false)`, which `handler.rs` forces to `TIER=sensitive`, an active attempt
/// to forge an agent's identity.
pub(super) async fn verify_lan_signature(state: &AppState, req: &mut InjectionRequest) {
    if req.delivery_tier.as_deref() != Some("lan") {
        return;
    }
    let Some(sig) = req.lan_sig.as_deref() else {
        return;
    };
    let Some(claimed) = req.source_agent.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    let observed_key = match lan_signer::resolve(state, &claimed).await {
        lan_signer::SignerKey::Pinned(key) => key,
        lan_signer::SignerKey::Unknown | lan_signer::SignerKey::Unproven => return,
        lan_signer::SignerKey::Refused => {
            req.lan_verified = Some(false);
            return;
        }
    };

    let msgid = req.request_id.clone().unwrap_or_default();
    let ts = req.ts_secs.unwrap_or(0);
    let within_freshness_window = ts > 0 && (now_unix_secs() - ts).abs() <= LAN_SIG_MAX_AGE_SECS;
    let verified = within_freshness_window
        && agentmux_common::jekt_sign::verify_lan_jekt(
            &observed_key,
            &msgid,
            &claimed,
            &req.target_agent,
            ts,
            &req.message,
            sig,
        );
    if !verified { tracing::warn!(agent_id = %claimed, fresh = within_freshness_window, "LAN jekt signature did not verify"); }
    req.lan_verified = Some(verified);
    lan_v2::claim_uid(state, req, &claimed, &observed_key).await;
}

mod channel_v2; mod detached;
mod flow_notes;
mod forward_tiers;
pub(super) mod peer_hold;
pub(crate) mod lan_signer;
mod registration_flags;
pub(crate) mod former_slug;
mod lan_v2;
pub(crate) mod lan_continuity;
pub(crate) mod lan_refusals;
pub(super) mod replay;

/// Anti-replay window for `channel_sig` (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D6):
/// host-tier's tight one, since a cross-channel forward is a same-machine call to `127.0.0.1`.
const CHANNEL_SIG_MAX_AGE_SECS: i64 = JEKT_SIG_MAX_AGE_SECS;

/// Cross-channel (same machine, other AgentMux instance) per-agent Ed25519 signature check
/// (docs/specs/SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D2, Phase B). Like
/// `verify_lan_signature`, but the sender's public key comes from the host-global shared registry
/// (`AgentEntry::jekt_public_key`, published at registration, §D1): a local file read, so it is
/// synchronous and has no LAN rate-limiter `Skipped` hazard.
///
/// Only for `delivery_tier == "channel"`, the label a same-host forward carries (§D4, set in
/// `handle_reactive_inject`'s Tier 2a/2b); `channel_verified` stays `None` otherwise.
///
/// Ordering (§D2 step 2): a sender this instance holds an HMAC key for is a same-instance agent
/// that `verify_jekt_signature` owns entirely, so this returns without touching `channel_verified`
/// and the two verifiers can never disagree.
///
/// Outcomes:
///
/// - `None`: no shared-registry entry for the sender, or every entry has an empty `jekt_public_key`
///   (written before Phase A: "cannot check", never "failed", §6).
///
/// - `Some(true)`: `channel_sig` is present, fresh and verifies against the published key of ANY of
///   the agent's entries (one name can be live in several channels, §D2 step 5).
///
/// - `Some(false)`: a published key was found but the signature was missing, stale or wrong. The
///   §D3 red flag; not escalated in Phase B (see `InjectionRequest::channel_verified`).
pub(super) fn verify_cross_channel_signature(state: &AppState, req: &mut InjectionRequest) {
    let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() else {
        return;
    };
    verify_cross_channel_signature_in(state, req, &shared_dir, now_unix_secs());
}

/// [`verify_cross_channel_signature`] with the shared registry dir and clock injected: the testable
/// core, which keeps tests off the process-global `AGENTMUX_SHARED_DIR`.
pub(super) fn verify_cross_channel_signature_in(
    state: &AppState,
    req: &mut InjectionRequest,
    shared_dir: &std::path::Path,
    now_secs: i64,
) {
    if req.delivery_tier.as_deref() != Some("channel") {
        return;
    }
    let Some(claimed) = req.source_agent.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    // Identity M4d-6: a v2 signature proves the sender's UID. When it is the agent this instance
    // knows by the claimed name, the name's checks are settled, even a failed HMAC against a key
    // kept from when that agent ran here; a different agent of the same name is not the name's.
    if channel_v2::settle_by_uid(state, req, &claimed, shared_dir, now_secs) {
        return;
    }
    // §D2 step 2: a same-instance sender is the HMAC path's to judge.
    if matches!(state.mstore.agent_jekt_key_load(&claimed), Ok(Some(_))) {
        return;
    }
    // Only entries that published a key count: an empty one (pre-Phase-A) is "cannot check", and
    // must not turn a missing signature into `Some(false)` on a mixed-version machine (§6).
    let published_keys: Vec<Vec<u8>> = agent_registry::lookup_all_shared(shared_dir, &claimed)
        .into_iter()
        .filter(|e| !e.jekt_public_key.is_empty())
        .filter_map(|e| agentmux_common::jekt_sign::decode_key(&e.jekt_public_key))
        .collect();
    if published_keys.is_empty() {
        return;
    }

    let msgid = req.request_id.clone().unwrap_or_default();
    let ts = req.ts_secs.unwrap_or(0);
    let within_freshness_window = ts > 0 && (now_secs - ts).abs() <= CHANNEL_SIG_MAX_AGE_SECS;
    let source_channel = req.source_channel.as_deref().unwrap_or("");
    let verified = within_freshness_window
        && req.channel_sig.as_deref().is_some_and(|sig| {
            published_keys.iter().any(|key| {
                agentmux_common::jekt_sign::verify_channel_jekt(
                    key,
                    &msgid,
                    &claimed,
                    source_channel,
                    &req.target_agent,
                    ts,
                    &req.message,
                    sig,
                )
            })
        });
    if !verified {
        tracing::warn!(
            agent_id = %claimed,
            source_channel = %source_channel,
            has_signature = req.channel_sig.is_some(),
            fresh = within_freshness_window,
            "cross-channel jekt: claimed sender has a published key but the signature did not verify"
        );
    }
    req.channel_verified = Some(verified);
}

/// Server-derived `delivery_tier` for the LAN-key case only
/// (docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §3): a `lan_key` caller is forced to
/// `"lan"` whatever the body says, so claiming `"host"` can't dodge LAN scrutiny. Any other
/// caller's claim (host/wan/lan/channel) is kept.
///
/// A full-`auth_key` caller is not downgraded from `"lan"`: a jekt that arrived by `lan_key` and
/// was relayed to a sibling instance on this machine authenticates that second hop with the
/// sibling's full `auth_key` (`lan_key` is peer-to-peer between machines only). Downgrading
/// discarded an already-detected LAN signature failure's forced-sensitive escalation
/// (`lan_verified`/`reagent_verified` are `#[serde(skip_deserializing)]`, so they re-derive on
/// every hop). Honouring the claim costs nothing: the full key already grants full control, and a
/// self-labelled tier only adds verification, never removes it.
///
/// `"channel"` (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4) follows the same rule: the
/// forwarding instance sets it on a same-host Tier 2a/2b hop, and it only adds
/// `verify_cross_channel_signature`.
pub(super) fn resolve_delivery_tier(auth_via: super::ReactiveAuthVia, claimed: Option<&str>) -> String {
    if auth_via == super::ReactiveAuthVia::LanKey {
        "lan".to_string()
    } else {
        claimed.unwrap_or("host").to_string()
    }
}

/// The `delivery_tier` a same-host Tier 2a/2b forward carries
/// (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4): `host` or unlabelled becomes `channel`; a
/// network tier is kept so the peer re-derives that tier's verification.
fn same_host_forward_tier(current: Option<&str>) -> &str {
    match current {
        None | Some("host") => "channel",
        Some(other) => other,
    }
}

/// Server-side resolution of `InjectionRequest::is_transcript_request` /
/// `transcript_request_escalate_forced` (fields documented in `backend/reactive/types.rs`;
/// SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md). Here, not in
/// `Handler::inject_message_inner`, because it needs `Store`, which `Handler` lacks by design.
///
/// Takes `&Arc<Store>` so `muxbus::cloud_subscriber::sync_agent_reactive`, the WAN path that calls
/// `Handler::inject_message` directly, applies the same rules; without it a WAN
/// `transcript_request` would skip both.
///
/// Always recomputes from `req.message`, never from anything the client set on the two fields.
pub(crate) fn resolve_transcript_request_tier_fields(mstore: &std::sync::Arc<crate::backend::storage::store::Store>, req: &mut InjectionRequest) {
    let Some(transcript_req) = crate::backend::reactive::transcript_request::parse_transcript_request(&req.message) else {
        return;
    };
    let _ = transcript_req; // request_id/max_lines belong to the (not-yet-built) auto-responder, not tier resolution.
    req.is_transcript_request = true;

    // The RESPONDING agent's (target_agent's) own definition: by its slug,
    // the stable AGENTMUX_AGENT_ID-derived identifier, else by its display
    // name's route name (a fork targeted as `Loap-3`, #4744). A target naming
    // several agents fails closed, as `ask`.
    let defs = mstore.agent_defs_for_target(&req.target_agent).unwrap_or_default();
    let (slug, visibility) = match defs.as_slice() {
        [def] => (def.slug.clone(), def.conversation_visibility.clone()),
        [] => (req.target_agent.clone(), crate::backend::storage::agents::default_conversation_visibility()),
        _ => (req.target_agent.clone(), "ask".to_string()),
    };

    let tier = req.delivery_tier.as_deref().unwrap_or("host");
    req.transcript_request_escalate_forced = match visibility.as_str() {
        "ask" => true,
        "trusted_peers" => {
            let requester = req.source_agent.as_deref().unwrap_or("");
            let granted = mstore
                .conversation_trust_grant_check(&slug, requester, tier)
                .unwrap_or(false);
            !granted
        }
        // "private" and any unrecognized value fail closed on the ESCALATE-forcing question too: a
        // def from a state older than this feature never opted into relaxation. Rule 1's forced
        // TIER=sensitive still applies; only this extra rule reads "private" as "no need to force
        // more", since a "private" agent auto-denies (once the responder auto-resolve is built) and
        // discloses nothing either way.
        _ => false,
    };
}

#[cfg(test)]
#[path = "tests/reactive/transcript_request_tier_resolution_tests.rs"]
mod transcript_request_tier_resolution_tests;

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Extension(auth_via): Extension<super::ReactiveAuthVia>,
    caller: Option<Extension<super::caller::Caller>>,
    Json(req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    if let Err(error) = super::lan_watch::acting(req.source_agent.as_deref(), super::actor::require_actor(&state, caller.as_deref(), super::actor::ActorSite::Inject, req.source_agent.as_deref())).await {
        return Json(serde_json::json!({ "success": false, "error": error }));
    }
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );
    let caller_uid = super::caller::attributed_uid(caller.as_deref());
    Json(detached::deliver_to_completion(&state, auth_via, &caller_uid, req).await)
}

/// Deliver an inject — verify, try this instance, then forward
/// cross-instance, cross-channel, LAN and cloud relay — exactly as
/// `POST /agentmux/reactive/inject` does. Shared with the cron scheduler's
/// in-process fire (identity M4c-3, spec §6.5.9), which passes its tier
/// explicitly (`auth_via`) in place of the route's `ReactiveAuthVia`
/// extension. `caller_uid` is the sender's attributed UID, audited on this
/// instance only (`InjectionRequest::audit_source_uid` never rides a hop);
/// `""` when Unattributed.
pub(crate) async fn deliver(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    caller_uid: &str,
    mut req: InjectionRequest,
) -> serde_json::Value {
    let mut replay = match admit(state, auth_via, caller_uid, &mut req).await {
        Ok(replay) => replay,
        Err(answer) => return flow_notes::noted(state, &req, answer),
    };

    // 1. Try local ReactiveHandler first (fast path — same instance).
    let resp = super::lan_watch::during("local delivery", super::cron::inject_here_following_runs(state, &req)).await;
    if resp.success {
        replay.delivered();
        flow_notes::note_delivered(state, &req);
        echo_local_delivery(state, &req, &resp);
        return super::jekt_files::with_count(serde_json::to_value(&resp).unwrap_or_default(), &req);
    }

    // The target is here but not signed in, or its pane has no controller yet: held, not failed.
    if let Some(answer) = super::jekt_hold::hold_for_refusal_here(state, auth_via, &req, &resp, &mut replay).await {
        return flow_notes::noted(state, &req, answer);
    }

    // 2. On "agent not found", check cross-instance file registry and forward.
    let is_not_found = resp
        .error
        .as_deref()
        .map(|e| e.starts_with("agent not found"))
        .unwrap_or(false);

    if is_not_found && req.forward_hops >= MAX_FORWARD_HOPS {
        tracing::warn!(
            target = %req.target_agent,
            hops = req.forward_hops,
            "reactive inject: forward-hop limit reached, not forwarding further"
        );
        return serde_json::to_value(&resp).unwrap_or_default();
    }

    if is_not_found {
        if let Some(body) = super::lan_watch::during("forwarding", forward_tiers::Tiers::new(state, auth_via, caller_uid, &req, &resp).run(&mut replay)).await {
            return body;
        }
    }

    // 6. Every tier declined — return the original local error.
    serde_json::to_value(&resp).unwrap_or_default()
}

/// What a jekt must pass before any delivery is tried: its signatures, the checks on its
/// files and its text, and the holds for an agent that cannot take it now. `Err` is the
/// answer to send back; `Ok` is the claim on its request id, released when `deliver` ends.
async fn admit(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    caller_uid: &str,
    req: &mut InjectionRequest,
) -> Result<replay::Reservation, serde_json::Value> {
    req.delivery_tier = Some(resolve_delivery_tier(auth_via, req.delivery_tier.as_deref()));
    // Identity M4c-2d: the sender's UID, for the audit only (`#[serde(skip)]`).
    req.audit_source_uid = caller_uid.to_string();
    super::actor::source_from_token(&state.mstore, caller_uid, req);
    // Identity M4d-6: count a signed source_uid other than the token's UID.
    if !caller_uid.is_empty() && req.source_uid.as_deref().is_some_and(|s| s != caller_uid) {
        crate::backend::agent_resolve::record_uid_fallback("m4.source_uid_mismatch");
    }

    verify_jekt_signature(state, req);
    verify_reagent_signature(req, now_unix_secs());
    super::lan_watch::during("sender signature check", verify_lan_signature(state, req)).await;
    verify_cross_channel_signature(state, req);
    lan_v2::log_receipt(req);
    let mut replay = replay::reserve(req, now_unix_secs());
    req.route_unsigned_target();
    resolve_transcript_request_tier_fields(&state.mstore, req);

    // Files (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §3.3, §6): refused unless the
    // msgid binds them, then prepared here, since the handler does no I/O.
    if let Err(error) = super::jekt_files::check(req) {
        return Err(super::jekt_files::refused(req, error));
    }
    // A reply that says nothing starts a turn for nothing (SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md §6).
    if let Some(refused) = super::jekt_ack::refuse_empty_reply(state, auth_via, req) {
        return Err(refused);
    }
    crate::backend::reactive::sent_audit::record(req);
    // An agent here out of quota or signed out would lose it in a failed turn, and a Claude
    // session gone cold should not be woken by a routine one: hold it, or refuse it where it
    // can't be held (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §5, SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_
    // 2026_10_25.md). Before `prepare`, which charges inline attachments to the session.
    if let Some(held) = super::lan_watch::during("hold check", hold_before_delivery(state, auth_via, req, &mut replay)).await {
        return Err(held);
    }

    super::jekt_files::prepare(req).await;
    Ok(replay)
}

/// Echo a locally delivered jekt into the sender's own pane, as every
/// successful delivery path does.
pub(super) fn echo_local_delivery(
    state: &AppState,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
) {
    echo_jekt_to_sender_with_files(
        state,
        req.source_agent.as_deref(),
        &req.target_agent,
        &req.message,
        &resp.request_id,
        resp.effective_tier.as_deref(),
        resp.requires_stop,
        // Same `req` the `effective_tier`/`requires_stop` came from, so the marker's TRUST/SIG
        // matches its ESCALATE=.
        EchoTrust::of(req),
        req.priority.as_deref().unwrap_or("normal"),
        req.files_delivery.as_ref(),
    );
}

/// The UID and slug of the agent defined in this channel that `target`
/// names, as the hold resolves it (`hold_for_absent_target`). The slug is
/// the agent's `AGENTMUX_AGENT_ID`, which its relay lease is keyed by.
pub(super) fn defined_here(mstore: &crate::backend::storage::store::Store, target: &str) -> Option<(String, String)> {
    let target = target.trim();
    let uid = match mstore.instance_get(target) {
        Ok(Some(row)) => row.id,
        _ => match mstore.agents_matching_name(target) {
            Ok(rows) if rows.len() == 1 => rows[0].id.clone(),
            _ => return None,
        },
    };
    let slug = match mstore.agent_def_get(&uid) {
        Ok(Some(def)) if !def.slug.is_empty() => def.slug,
        _ => target.to_string(),
    };
    Some((uid, slug))
}

/// Where a jekt goes when no tier found its target live
/// (`PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md` §5-§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbsentRoute {
    /// Hold it on this instance (24 h) and deliver when the agent opens here.
    HoldHere,
    /// Leave it to the cloud relay, then the hold as a fallback (the order
    /// before this plan).
    Relay,
}

/// Decide [`AbsentRoute`] for a target no tier found live. Holding here is
/// the default for an agent defined in this channel, so messaging works
/// without the relay; the relay overrides it only when it says another
/// install of the account holds the agent's lease. Anything the relay
/// cannot answer plainly keeps the old order (relay first), so an older
/// relay changes nothing.
pub(crate) async fn route_for_absent(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
) -> AbsentRoute {
    use crate::muxbus::wan_lease;
    if !hold_applies(auth_via, req, resp) {
        return AbsentRoute::Relay;
    }
    let mstore = state.mstore.clone();
    let target = req.target_agent.clone();
    let Some((_uid, slug)) = tokio::task::spawn_blocking(move || defined_here(&mstore, &target)).await.ok().flatten() else {
        // Not an agent of this channel: it can only be on another install.
        return AbsentRoute::Relay;
    };
    // This instance itself recently heard the lease is held elsewhere.
    if wan_lease::held_elsewhere(&slug).is_some() {
        return AbsentRoute::Relay;
    }
    // No relay for this sender (no source agent, not signed in): hold.
    let Some(source) = req.source_agent.as_deref().filter(|s| !s.is_empty()) else {
        return AbsentRoute::HoldHere;
    };
    let Some(credential) = crate::muxbus::relay::relay_token(source, &state.id_store, &state.http_client).await else {
        return AbsentRoute::HoldHere;
    };
    let answer = wan_lease::holder(
        &crate::muxbus::relay::rest_base_url(),
        &slug,
        source,
        &credential.token,
        &state.http_client,
    )
    .await;
    let route = absent_route_for(&answer);
    tracing::debug!(target = %req.target_agent, lease = ?answer, ?route, "jekt: routing an absent local agent");
    route
}

/// The relay's lease answer as a route: hold here unless it says another
/// install runs the agent or cannot say; an unreachable relay holds here
/// (there is no relay to send through anyway).
pub(super) fn absent_route_for(answer: &crate::muxbus::wan_lease::Holder) -> AbsentRoute {
    use crate::muxbus::wan_lease::Holder;
    match answer {
        Holder::Free | Holder::Yours | Holder::Unreachable => AbsentRoute::HoldHere,
        Holder::Other(..) | Holder::Unknown | Holder::Unsupported => AbsentRoute::Relay,
    }
}

/// Why a jekt is held (`server::hold_reason`).
pub(crate) use super::hold_reason::HoldReason;
pub(in crate::server) use super::jekt_hold::hold_for_absent_target;
use super::jekt_hold::{hold_applies, hold_before_delivery};

/// Tier 4. `Some(body)` when the cloud accepted the injection (sender echo emitted); `None` when it
/// doesn't apply or didn't take, leaving the caller to return its original error.
///
/// **Queued is not delivered:** `success: true` means "handed to the relay", the strongest claim
/// any WAN tier makes (and what `DELIVERY=wan` on the echoed marker says). The recipient's srv
/// picks it up on its next sync, or never if that instance is offline.
pub(super) async fn try_cloud_relay(state: &AppState, req: &InjectionRequest, caller_uid: &str) -> Option<serde_json::Value> {
    // Never re-relay what ARRIVED over the cloud: an agent unknown everywhere would bounce between
    // the relay and every subscriber forever. `forward_hops` can't catch it (the cloud resets the
    // count), so this tier needs its own guard.
    if req.delivery_tier.as_deref() == Some("wan") {
        return None;
    }

    // The cloud route derives the sender from `X-Agent-ID` and 400s without it,
    // so a caller with no source agent (cron, external bridge) has no tier 4.
    let source = req.source_agent.as_deref().filter(|s| !s.is_empty())?;

    let credential = crate::muxbus::relay::relay_token(source, &state.id_store, &state.http_client).await;
    let Some(credential) = credential else {
        tracing::debug!(
            target = %req.target_agent,
            "cloud relay skipped: not logged in to muxbus"
        );
        return None;
    };

    let priority = req.priority.as_deref().unwrap_or("normal");
    // SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md: srv signs at the origin for the agent the
    // request's token names (`wan_signer`). Any unmet condition relays unsigned; the reason rides
    // on the "queued" log line so the receiver can tell it from a sender that never signed.
    // `directory` is this relay plus the credential's account.
    let directory = Some(credential.account_sub.as_str())
        .filter(|sub| !sub.is_empty())
        .map(|sub| crate::muxbus::wan_publish::directory_id(&crate::muxbus::relay::rest_base_url(), sub));
    let (carried, unsigned_reason) = match crate::muxbus::relay::wan_sign_at_origin(
        req,
        super::actor::wan_signer(&state.mstore, caller_uid, req),
        state.mstore.wan_identity().as_deref(),
        directory.as_deref(),
        &crate::backend::reactive::registry::local_channel_id(),
        agentmux_common::time::now_secs(),
    ) {
        Ok(carried) => (Some(carried), ""),
        Err(reason) => (None, reason),
    };
    let outcome = crate::muxbus::relay::relay_inject(
        &crate::muxbus::relay::rest_base_url(),
        &state.http_client,
        &credential.token,
        source,
        &req.target_agent,
        &req.message,
        priority,
        carried.as_ref(), req.cloud_files.as_deref(), req.jekt_tier.as_ref(), req.expires_at_ms,
    )
    .await;

    match outcome {
        crate::muxbus::relay::RelayOutcome::Queued { injection_id, target_in_account } => {
            let request_id = injection_id.unwrap_or_default();
            tracing::info!(
                target = %req.target_agent,
                injection_id = %request_id,
                signed = carried.is_some(),
                unsigned_reason,
                // The relay answers with an `inj-w-` id when it kept a valid
                // carried tuple; any other id means the message travels
                // unsigned, including when it dropped a tuple as invalid.
                cloud_kept_signature = request_id.starts_with("inj-w-"),
                "cloud relay: queued for WAN delivery"
            );
            echo_jekt_to_sender_with_files(
                state,
                Some(source),
                &req.target_agent,
                &req.message,
                &request_id,
                None, // no receiver-side escalation to mirror: nobody has
                None, // seen the message yet, so neither field is known
                EchoTrust {
                    delivery_tier: "wan",
                    // Signing here is #2586's unbuilt WAN half: an outbound relay carries no proof
                    // of sender identity, and claiming one would put a TRUST value on the marker
                    // the receiver won't agree with.
                    sig_verified: None,
                    reagent_verified: None,
                    lan_verified: None,
                    channel_verified: None,
                },
                priority, req.files_delivery.as_ref(),
            );
            // Watched for the sender's sake: a message nobody picks up is reported, not forgotten
            // (muxbus::relay_watch). The LAN peers go in the answer: the target is on none of them.
            crate::muxbus::relay_watch::watch(state.reactive_handler, source, &req.target_agent, &request_id);
            let peers = state.lan_discovery.get_instances().iter().map(crate::backend::lan::peer_words::describe_peer).collect();
            let body = crate::muxbus::relay_watch::queued_answer(request_id.clone(), target_in_account, peers, now_unix_secs().max(0) as u64);
            Some(flow_notes::queued(state, req, &request_id, body))
        }
        crate::muxbus::relay::RelayOutcome::Failed(e) => {
            tracing::warn!(
                target = %req.target_agent,
                error = %e,
                signed = carried.is_some(),
                unsigned_reason,
                "cloud relay failed"
            );
            None
        }
    }
}

pub(super) async fn handle_reactive_agents(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agents = state.reactive_handler.list_agents();
    Json(serde_json::to_value(&agents).unwrap_or(json!([])))
}

/// Agent NAMES only, for LAN peers: not [`handle_reactive_agents`], whose
/// full `AgentRegistration` records (`block_id`, `tab_id`, `registration_nonce`,
/// ...) are routing details that stay behind full auth.
///
/// Reachable with the broadcast `lan_key` (`lan_or_full_auth_middleware`), so
/// its output is public to anyone on the local network. **Do not extend it**
/// without the owner's decision.
///
/// Built by [`crate::backend::lan::discovery::agent_names_body`], which lists
/// the fields and the decision behind each.
pub(super) async fn handle_reactive_agent_names(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let settings = state.config_watcher.get_full_config();
    let agents = state.reactive_handler.list_agents();
    let account_of_block = crate::identity::stable_account::stable_id_of_block(&state.mstore, &state.id_store, &state.identity_store);
    Json(crate::backend::lan::discovery::agent_names_body(&state.mstore, &settings.settings, &agents, account_of_block))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentQuery {
    id: Option<String>,
}

pub(super) async fn handle_reactive_agent(
    State(state): State<AppState>,
    Query(params): Query<AgentQuery>,
) -> Response {
    let id = match &params.id {
        Some(id) if !id.is_empty() => id.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing id param"})),
            )
                .into_response()
        }
    };
    if let Some(sender) = crate::backend::lan::swarm_broadcast::sender_lookup(&state.mstore, &state.hostname, id) {
        return Json(sender).into_response();
    }
    match state.reactive_handler.get_delivery_target(id) {
        Some(agent) => {
            // Merged in, not part of AgentRegistration's serialization: the LAN pubkey-lookup half
            // of docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2. A public key isn't
            // secret, and this route is already gated by lan_or_full_auth_middleware. Omitted
            // entirely (not `null`) when no LAN key is minted yet, so existing callers see no
            // change in shape.
            let mut value = serde_json::to_value(&agent).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("jekt_features".into(), json!([agentmux_common::jekt_files::FEATURE]));
            }
            if let Ok(Some(pubkey)) = state.mstore.agent_lan_public_key_load(id) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("lan_public_key".to_string(), json!(pubkey));
                    lan_continuity::add_cert(&state, id, &pubkey, obj); // so a peer pinned to an older key can move its pin
                }
            }
            // Identity M4d-5: the registration's UID key, separately; the
            // name-keyed key above stays the query id's (spec §6.5.10).
            if let Some(Ok(Some(pubkey))) = agent.uid.as_deref().map(|uid| state.mstore.agent_uid_lan_public_key_load(uid)) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("uid_public_key".to_string(), json!(pubkey));
                }
            }
            // A former slug: its agent's current name and key (`former_slug`).
            former_slug::renamed_lookup(&state, id, &mut value);
            Json(value).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "agent not found"})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}
fn default_audit_limit() -> usize {
    100
}

pub(super) async fn handle_reactive_audit(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Json<serde_json::Value> {
    let log = state.reactive_handler.get_audit_log(params.limit);
    Json(serde_json::to_value(&log).unwrap_or(json!([])))
}

#[derive(serde::Deserialize)]
pub(super) struct RegisterRequest {
    agent_id: String,
    block_id: String,
    tab_id: Option<String>,
}

pub(super) async fn handle_reactive_register(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(mut req): Json<RegisterRequest>,
) -> Response {
    let sent_name = req.agent_id.clone();
    req.agent_id = crate::backend::reactive::sanitize::route_name(&sent_name).unwrap_or_default();
    tracing::info!(agent_id = %req.agent_id, block_id = %req.block_id, "reactive register request");
    // Identity M2 (spec §4.4.1 row 3): a presence signal keyed by block. The sent name, as its
    // route name (above), becomes the display binding and the cross-instance registries' name, but
    // identity is NOT taken from it (it may be a stale meta value or an id the agent emitted over
    // OSC). The UID comes from the row the server made for this block, read on `spawn_blocking`
    // like every async-handler store read (#1782).
    let uid = {
        let mstore = state.mstore.clone();
        let block_id = req.block_id.clone();
        tokio::task::spawn_blocking(move || crate::backend::agent_resolve::uid_for_block(&mstore, &block_id))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "reactive register: uid lookup task failed — registering by name");
                None
            })
    };
    if let Some(refusal) = super::pane_binding::register_refusal(&state, caller.as_deref(), &req.block_id, uid.as_deref(), &sent_name, &req.agent_id).await {
        return refusal;
    }
    // One live instance per agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 Phase 1, I9): a pane
    // whose agent runs in another instance on this host must not register it. The cross-instance
    // registries written below send this agent's jekts to the freshest entry, so registering here
    // would pull the live instance's messages to a pane refused its turns. The frontend only logs a
    // failed registration; it doesn't retry.
    if let Some(uid) = uid.as_deref() {
        if let Err(refusal) = crate::backend::agent_admission::check_before_spawn(
            crate::backend::agent_admission::lease_store_for(state.mstore.shared_agent_registry()),
            uid,
            &req.agent_id,
            &state.boot_id,
        )
        .await
        {
            tracing::warn!(
                agent_id = %req.agent_id,
                block_id = %req.block_id,
                uid,
                "reactive register refused: agent is live in another AgentMux instance on this host"
            );
            return (
                StatusCode::CONFLICT,
                Json(json!({ "success": false, "error": refusal })),
            )
                .into_response();
        }
    }
    match state.reactive_handler.register_agent_full(
        &req.agent_id,
        &req.block_id,
        req.tab_id.as_deref(),
        0,
        None,
        uid.as_deref(),
        "registration.no_uid.http_register",
    ) {
        Ok(()) => {
            // Refresh the block's own captured identity too (#2697): this path can re-register a
            // block_id under a different agent_id (a rename, a changed cmd:env), and a stale
            // controller `agent_id()` would make `inject_message_inner`'s recipient check (#2695)
            // reject the agent's own correctly-addressed messages. `None` (untracked block, or a
            // controller without `agent_id()`/`set_agent_id()`) is a no-op.
            if let Some(ctrl) = blockcontroller::get_controller(&req.block_id) {
                ctrl.set_agent_id(Some(req.agent_id.clone()));
            }

            // Also write to the cross-instance file registry so other AgentMux instances can
            // forward inject requests here.
            let data_dir = base::get_mux_data_dir();
            agent_registry::write(&data_dir, &req.agent_id, &state.local_web_url, &req.block_id);

            // And to the host-global shared registry (Tier 2b), so instances in OTHER channels
            // reach this agent too (#1916).
            agent_registry::write_shared_from_env(
                &req.agent_id,
                &state.local_web_url,
                &req.block_id,
            );

            // Auto-watch this agent's Claude Code config dir for subagent JSONL files. `block_id`
            // stamps subagent events with the owning pane so the frontend routes ⚡ panels to it
            // only. The dir comes from the block's own `cmd:env` (see `resolve_claude_config_dir`).
            let block = state.mstore.get::<crate::backend::obj::Block>(&req.block_id).ok().flatten();
            let empty_meta = crate::backend::obj::MetaMapType::new();
            // An identity-bound agent's real CLAUDE_CONFIG_DIR is never the stale `cmd:env`
            // snapshot (`resolve_claude_config_dir`,
            // SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md).
            let bound_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                &state.mstore,
                &state.id_store,
                &state.identity_store,
                &req.block_id,
            );
            crate::identity::foreign_dir::heal_block_config_dir_env(&state.mstore, block.as_ref(), bound_dir.as_deref());
            let config_dir = subagent_watcher::resolve_claude_config_dir(
                block.as_ref().map(|b| &b.meta).unwrap_or(&empty_meta),
                &req.agent_id,
                bound_dir,
            );
            if let Some(config_dir) = config_dir {
                state.subagent_watcher.watch_agent(&req.agent_id, &req.block_id, config_dir.clone());

                // Re-resolve now that the watch is installed (#2980): an `agent_identity_link`
                // write on another thread can commit between resolving `config_dir` and
                // `watch_agent`, and the bind's own `recheck_all_watched_agents` then finds nothing
                // to correct, with no later bind guaranteed to retrigger it. `recheck_config_dir`
                // is a no-op when nothing changed. Here, not in `watch_agent`, because this is the
                // one site that derives `config_dir` from identity binding (baking it into
                // `watch_agent` broke its other callers).
                let fresh_bound_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                    &state.mstore,
                    &state.id_store,
                    &state.identity_store,
                    &req.block_id,
                );
                if let Some(fresh_dir) = subagent_watcher::resolve_claude_config_dir(
                    block.as_ref().map(|b| &b.meta).unwrap_or(&empty_meta),
                    &req.agent_id,
                    fresh_bound_dir,
                ) {
                    state.subagent_watcher.recheck_config_dir(&req.agent_id, fresh_dir);
                }

                // A block with a persisted session id is resuming: backfill only THAT session's
                // subagents, so a reopened pane shows what it had without flooding in every other
                // session of this identity. A new session has nothing to backfill; the live watcher
                // picks up subagents as the Task tool spawns them.
                let session_id = block.as_ref().map(|b| {
                    crate::backend::obj::meta_get_string(
                        &b.meta,
                        crate::backend::blockcontroller::core::META_SESSION_ID,
                        "",
                    )
                }).unwrap_or_default();
                if !session_id.is_empty() {
                    state.subagent_watcher.scan_session_subagents(
                        &req.agent_id,
                        &req.block_id,
                        &config_dir,
                        &session_id,
                    );
                }
            }

            // Subscribe the display and stable names; cloud senders address the stable one.
            if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                state.reactive_handler.names_for_block(&req.block_id).iter().for_each(|n| sub.add_agent(n));
            }

            // Notify the Swarm view so it calls AgentTrackedBlocksCommand and shows this pane. A
            // dedicated event name, because `useProcessCount` (on agent:process-added/-exited)
            // would take this for a phantom OS process and show a spurious ⚙ N badge or the
            // kill-tree modal on pane close.
            state.broker.publish_event::<crate::backend::mps_events::AgentReactiveRegistered>(&crate::backend::mps_events::BlockRefEvent { block_id: req.block_id.clone() }, vec![format!("block:{}", req.block_id)], 0);

            Json(json!({"success": true})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct EnsureSigningKeyRequest {
    agent_id: String,
}

/// Mints or reuses `agent_id`'s host-tier jekt signing key and returns it base64-encoded: the value
/// `agent_config::inject_jekt_signing_keys_into_mcp_json` writes into a real agent's `.mcp.json` at
/// spawn, exposed here so tests and external harnesses can exercise host-tier signing without
/// `agent.open`'s WebSocket path, which also spawns a live provider session.
///
/// **Disabled unless `AGENTMUX_ENABLE_TEST_ENDPOINTS=1` was set in the srv process's own
/// environment at start** (#2635). `X-AuthKey` is not enough: it is injected into every spawned
/// agent's env (`agent_handlers/input.rs`), so any agent holds it, and returning another agent's
/// raw key would be exfiltration enabling impersonation, breaking the invariant
/// `agent_jekt_keys.rs` documents (a key goes only to its own agent, by its identity token) and
/// `TRUST=host-verified`. Nothing in the request can prove the caller IS `agent_id` (that is the
/// problem host-tier signing solves, so leaning on it is circular); the env flag is set by whoever
/// launches the instance, which a running agent's tool calls can't do. Off by default, on only for
/// an isolated test instance of synthetic agents.
pub(super) async fn handle_reactive_ensure_signing_key(
    State(state): State<AppState>,
    Json(req): Json<EnsureSigningKeyRequest>,
) -> Response {
    if std::env::var("AGENTMUX_ENABLE_TEST_ENDPOINTS").as_deref() != Ok("1") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not found"})),
        )
            .into_response();
    }
    match state.mstore.agent_jekt_key_ensure(&req.agent_id) {
        Ok(key) => {
            use base64::Engine as _;
            let key_b64 = base64::engine::general_purpose::STANDARD.encode(&key);
            Json(json!({ "agent_id": req.agent_id, "jekt_key_b64": key_b64 })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct UnregisterRequest {
    pub(super) agent_id: String,
    /// Identity M2 (spec §4.4.4 Q7): optional, but both frontend callers
    /// send it. With it, teardown is by block. Without it, a name held by
    /// several live blocks is refused with HTTP 409 and the candidates —
    /// never a `success: true` that tore down nothing (§10 constraint 3).
    #[serde(default)]
    pub(super) block_id: String,
}

/// Tear down a pane's registration. Reached only from the window's own socket (`agent.pane.unregister`)
/// and, for an agent's own pane, from `pane_binding::handle_unregister`.
pub(super) async fn unregister_pane(state: &AppState, req: UnregisterRequest) -> Response {
    use crate::backend::reactive::handler::UnregisterOutcome;

    let (block_id, names): (Option<String>, Vec<String>) = if !req.block_id.trim().is_empty() {
        let names = state.reactive_handler.unregister_block(req.block_id.trim());
        (Some(req.block_id.trim().to_string()), names)
    } else {
        match state.reactive_handler.unregister_agent(&req.agent_id) {
            UnregisterOutcome::Removed { block_id, names } => (Some(block_id), names),
            UnregisterOutcome::NotFound => (None, Vec::new()),
            UnregisterOutcome::Ambiguous(candidates) => {
                let candidates: Vec<serde_json::Value> = candidates
                    .iter()
                    .map(
                        |c| json!({ "uid": c.uid, "block_id": c.block_id, "agent_id": c.agent_id }),
                    )
                    .collect();
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "success": false,
                        "error": format!(
                            "ambiguous agent name: '{}' is held by {} live agents; pass block_id",
                            req.agent_id,
                            candidates.len()
                        ),
                        "candidates": candidates,
                    })),
                )
                    .into_response();
            }
        }
    };

    // Name-keyed side state is torn down for EVERY name the block held (display and stable). The
    // caller's name is used only when no block was found at all (the pre-M2 behaviour for an
    // unknown name): a torn-down block's own bindings are authoritative, since the caller's name
    // may be a stale display value another live block now holds, and removing that block's file
    // entry and cloud subscription would knock it off cross-instance routing.
    let mut all_names = names.clone();
    if block_id.is_none() {
        all_names.push(req.agent_id.clone());
    }
    let data_dir = base::get_mux_data_dir();
    for name in &all_names {
        // Also remove from cross-instance file registry.
        agent_registry::remove(&data_dir, name);
        // And from the host-global shared registry (Tier 2b).
        agent_registry::remove_shared_from_env(name);
        // Drop the subagent filesystem watcher (handle, channel, task): the symmetric teardown for
        // `watch_agent()` in the register handler. `block_id` lets a watcher shared with another
        // still-open block survive this one's teardown.
        state.subagent_watcher.unwatch_agent(name, block_id.as_deref());
        // Notify cloud subscriber so it stops subscribing for this agent
        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
            sub.remove_agent(name);
        }
    }

    // Symmetric refresh: tell the Swarm view this pane is gone.
    if let Some(bid) = block_id {
        state.broker.publish_event::<crate::backend::mps_events::AgentReactiveUnregistered>(&crate::backend::mps_events::BlockRefEvent { block_id: bid.to_string() }, vec![format!("block:{}", bid)], 0);
    }

    Json(json!({"success": true})).into_response()
}

pub(super) async fn handle_reactive_poller_stats(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let stats = state.poller.stats();
    Json(serde_json::to_value(&stats).unwrap_or(json!({})))
}

#[derive(serde::Deserialize)]
pub(super) struct PollerConfigRequest {
    url: Option<String>,
    token: Option<String>,
}

pub(super) async fn handle_reactive_poller_config(
    State(state): State<AppState>,
    Json(req): Json<PollerConfigRequest>,
) -> Json<serde_json::Value> {
    state.poller.reconfigure(req.url, req.token);
    Json(json!({"success": true}))
}

pub(super) async fn handle_reactive_poller_status(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let status = state.poller.status();
    Json(serde_json::to_value(&status).unwrap_or(json!({})))
}

/// Server-side ceiling on `max_lines`, so a transcript pull can't read a huge session file
/// unbounded. This route is a "recent tail" primitive, not a full-history export.
const TRANSCRIPT_MAX_LINES_CAP: usize = 500;

#[derive(serde::Deserialize)]
pub(super) struct TranscriptQuery {
    agent: String,
    #[serde(default = "default_transcript_max_lines")]
    max_lines: usize,
    /// Set ONLY by [`handle_reactive_transcript_cross_channel`] on its single forwarded request,
    /// capping cross-channel resolution at one hop. Without it a stale-but-PID-alive
    /// shared-registry entry (pointing back at this instance, or at a second whose entry points
    /// back here) would forward forever, the failure `handle_reactive_inject`'s `MAX_FORWARD_HOPS`
    /// prevents for jekts (#2715). A bool is enough, unlike inject's counter: the owning instance
    /// found via `lookup_all_shared` has the agent on its own host tier, never a further hop, so
    /// "already forwarded once" and "hop limit reached" are the same condition.
    #[serde(default)]
    forwarded: bool,
}
fn default_transcript_max_lines() -> usize {
    100
}

/// `GET /agentmux/reactive/transcript?agent=<name>&max_lines=<n>`: the tail of a registered agent's
/// session output (the `Agents` tool's `transcript` action). A pull primitive; nothing is pushed.
///
/// As of SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md Phase A, a miss on this
/// instance's host-tier registry falls back to the host-global cross-channel registry and forwards
/// a single-hop GET to the owning channel's instance, with the same auth (`entry.auth_key` as
/// `X-AuthKey`) and loopback-only pattern as `handle_reactive_inject`'s Tier 2b. The response's
/// `"tier"` says which tier answered.
pub(super) async fn handle_reactive_transcript(
    State(state): State<AppState>,
    Query(params): Query<TranscriptQuery>,
) -> Response {
    if params.agent.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing agent param"})),
        )
            .into_response();
    }

    let reg = match state.reactive_handler.lookup_delivery_target(&params.agent) {
        crate::backend::reactive::handler::LookupOutcome::One(reg) => reg,
        // Identity M2 (spec §4.4.3): a name several live blocks hold is a purely LOCAL collision:
        // refuse it with the candidates, never fall through to cross-channel forwarding as if it
        // were unknown.
        crate::backend::reactive::handler::LookupOutcome::Ambiguous(candidates) => {
            let candidates: Vec<serde_json::Value> = candidates
                .iter()
                .map(|c| json!({ "uid": c.uid, "block_id": c.block_id, "agent_id": c.agent_id }))
                .collect();
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "ambiguous agent name: '{}' is held by {} live agents; address one by uid",
                        params.agent,
                        candidates.len()
                    ),
                    "candidates": candidates,
                })),
            )
                .into_response();
        }
        crate::backend::reactive::handler::LookupOutcome::NotFound => {
            if params.forwarded {
                // Already one hop in (see `TranscriptQuery::forwarded`): the owning instance's
                // host-tier lookup missed too, so the agent isn't registered anywhere reachable.
                // 404, no second forward.
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "agent not found"})),
                )
                    .into_response();
            }
            return handle_reactive_transcript_cross_channel(&state, &params).await;
        }
    };
    let block_id = reg.block_id.clone();

    // A bounded tail, off the async workers (transcript_tail.rs): the file can be hundreds of MB.
    let requested = params.max_lines.min(TRANSCRIPT_MAX_LINES_CAP).max(1);
    let (mstore, filestore, bid) = (state.mstore.clone(), state.filestore.clone(), block_id.clone());
    let read = tokio::task::spawn_blocking(move || super::transcript_tail::tail_lines(&mstore, &filestore, &bid, requested)).await;
    let (lines, truncated) = match read {
        Ok(Ok(t)) => (t.lines, t.truncated),
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("read_session_output: {e}")}))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("transcript read: {e}")}))).into_response(),
    };

    let turn_active = crate::backend::blockcontroller::get_block_controller_status(&block_id)
        .map(|s| s.turn_active)
        .unwrap_or(false);

    Json(json!({
        "agent": reg.agent_id,
        "block_id": block_id,
        "tier": "host",
        "turn_active": turn_active,
        "lines": lines,
        "truncated": truncated,
    }))
    .into_response()
}

/// Fallback for [`handle_reactive_transcript`] when the agent isn't in this instance's host-tier
/// registry: looks in the host-global cross-channel registry and, on a hit, forwards to the owning
/// channel's instance. A miss on every channel of this host 404s as the host-only lookup always
/// did; it does not reach LAN or WAN (Phase A scope).
async fn handle_reactive_transcript_cross_channel(
    state: &AppState,
    params: &TranscriptQuery,
) -> Response {
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "agent not found"})),
        )
            .into_response()
    };

    let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() else {
        return not_found();
    };
    // Skip any entry pointing back at THIS instance before picking one: a stale-but-PID-alive
    // self-registration (the registration race or incomplete cleanup, #2715) would forward the
    // request to ourselves and re-enter this function, including when it is merely the FRESHEST of
    // several candidates (the same defence as `handle_reactive_inject`'s Tier 2b).
    // `TranscriptQuery::forwarded` still caps it at one hop in a multi-instance cycle this filter
    // can't catch (A's entry points to B, B's back to A).
    // A former slug's agent is registered, and asked for, under its current one.
    let alias = former_slug::owner(state, &params.agent).map(|o| o.slug);
    let (agent, entries) = former_slug::shared_entries(&shared_dir, &params.agent, alias.as_deref());
    let Some(entry) = entries.into_iter().find(|e| !is_self_registration(&e.local_url, &state.local_web_url)) else {
        return not_found();
    };

    let query: Vec<(&str, String)> = vec![
        ("agent", agent),
        ("max_lines", params.max_lines.to_string()),
        ("forwarded", "true".to_string()),
    ];
    let resp = state
        .http_client
        .get(format!("{}/agentmux/reactive/transcript", entry.local_url))
        .header(AUTH_KEY_HEADER, &entry.auth_key)
        .query(&query)
        .send()
        .await;

    let Ok(resp) = resp else {
        return not_found();
    };
    if !resp.status().is_success() {
        return not_found();
    }
    let Ok(mut body) = resp.json::<serde_json::Value>().await else {
        return not_found();
    };
    if let Some(obj) = body.as_object_mut() {
        obj.insert("tier".to_string(), json!("cross-channel"));
        obj.insert("channel".to_string(), json!(entry.channel));
    }
    Json(body).into_response()
}

#[cfg(test)]
#[path = "tests/reactive/transcript_cross_channel_tests.rs"]
mod transcript_cross_channel_tests;

/// Default/cap for how many of an agent's sessions one search may open
/// (SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md). Full-parsing every session of a long-lived agent
/// isn't viable (the session that motivated the spec was 14.7 MB / 6682 records, one of several),
/// so candidates are taken newest-first and the default answers "what did I do recently".
const HISTORY_SEARCH_DEFAULT_MAX_SESSIONS: usize = 20;
const HISTORY_SEARCH_MAX_SESSIONS_CAP: usize = 100;
const HISTORY_SEARCH_DEFAULT_LIMIT: usize = 50;
const HISTORY_SEARCH_LIMIT_CAP: usize = 200;

/// SearchHistory's answer to a request that carries no per-agent token.
const HISTORY_SEARCH_NEEDS_IDENTITY: &str = "history.search: this request carries no agent identity \
     (X-Agent-Token), so whose history to search is unknown. Every agent AgentMux spawns carries \
     one; a process started some other way, or an agent from before identity tokens, must be \
     reopened from AgentMux.";

fn default_history_max_sessions() -> usize {
    HISTORY_SEARCH_DEFAULT_MAX_SESSIONS
}
fn default_history_limit() -> usize {
    HISTORY_SEARCH_DEFAULT_LIMIT
}

#[derive(serde::Deserialize)]
pub(super) struct HistorySearchQuery {
    /// The caller's name, as the MCP sends it. **Never whose history is searched**: that is the row
    /// of the caller's per-agent token (`X-Agent-Token`, identity M4c-2c,
    /// SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9), and a request without one is
    /// refused. A name is self-declared (every local agent shares the instance `auth_key`), so
    /// resolving the owner from it could only guess, and a guess answered "no history" with
    /// confidence. Kept for the actor counters (M4a-2).
    #[serde(default)]
    agent: String,
    query: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    since: Option<i64>,
    #[serde(default)]
    until: Option<i64>,
    #[serde(default = "default_history_max_sessions")]
    max_sessions: usize,
    #[serde(default = "default_history_limit")]
    limit: usize,
    /// Also search sessions found only by working directory or account,
    /// which the agent's own record doesn't name.
    #[serde(default)]
    include_inferred: bool,
}

/// `GET /agentmux/reactive/history/search`: search an agent's own past conversations
/// (SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md). Unlike `/reactive/transcript` (the live session's
/// tail), it reads sessions persisted on disk, including ended ones: after a compaction or session
/// reset an agent answers questions about its own past confidently and wrongly, with nothing
/// marking the boundary.
pub(super) async fn handle_reactive_history_search(
    State(state): State<AppState>,
    caller: Option<Extension<super::caller::Caller>>,
    Query(params): Query<HistorySearchQuery>,
) -> Response {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::HistorySearch,
        Some(params.agent.as_str()).filter(|a| !a.trim().is_empty()),
    );
    // An empty query makes sense only with a `tool` filter ("every call to X"); alone it would
    // match everything and return a truncated firehose that reads as a real answer.
    if params.query.trim().is_empty() && params.tool.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "query must be non-empty unless a tool filter is given"})),
        )
            .into_response();
    }
    if let Some(role) = params.role.as_deref() {
        if role != "user" && role != "assistant" {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "role must be 'user' or 'assistant'"})),
            )
                .into_response();
        }
    }

    let opts = crate::backend::history::index::HistorySearchOptions {
        query: params.query.clone(),
        role: params.role.clone(),
        tool: params.tool.clone(),
        limit: params.limit.clamp(1, HISTORY_SEARCH_LIMIT_CAP),
        since_ms: params.since.map(crate::backend::history::unix_time_to_ms),
        until_ms: params.until.map(crate::backend::history::unix_time_to_ms),
        max_sessions: Some(params.max_sessions.clamp(1, HISTORY_SEARCH_MAX_SESSIONS_CAP)),
        include_inferred: params.include_inferred,
    };

    // Identity M4c-2c (spec §6.5.9): the owner is the caller's own row, from its per-agent token,
    // never a name. A request without one can't say whose history it is, and resolving a
    // self-declared name could only guess (on 2026-09-24 a guess answered "no history" for an agent
    // whose sessions were on disk).
    let Some(uid) = caller.as_deref().and_then(super::caller::Caller::uid) else {
        crate::backend::agent_resolve::record_uid_fallback("history.search_refused_unattributed");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": HISTORY_SEARCH_NEEDS_IDENTITY})),
        )
            .into_response();
    };
    let row = match super::app_api::SelfOwner::caller_row(uid, &state.mstore) {
        Ok(row) => row,
        Err(e) => {
            // A gone row is the caller's problem; a store fault is ours.
            let status = if e.starts_with("store:") {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(json!({"error": format!("history.search: {e}")})))
                .into_response();
        }
    };

    // Parsing sessions is blocking filesystem work: keep it off the async workers.
    let history = state.history_service.clone();
    let store = state.identity_store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let owner = crate::backend::history::HistoryOwner::of_row(&row);
        history.search_for_agent(&store, &owner, &opts)
    })
    .await;

    match result {
        Ok(Ok(outcome)) => (StatusCode::OK, Json(json!(outcome))).into_response(),
        // Retryable, and distinct from a fault: an index still building is
        // not an empty history.
        Ok(Err(crate::backend::history::HistorySearchError::IndexBuilding)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(axum::http::header::RETRY_AFTER, "30")],
            Json(json!({"error": crate::backend::history::HISTORY_INDEX_BUILDING})),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("history search task failed: {e}")})),
        )
            .into_response(),
    }
}

// ---- WS RPC: reactive.registrations (issue #2696, Stash UI indicator) ----

/// Whether a host-global shared-registry entry is THIS instance's own registration. Every agent
/// writes itself into that registry, so a "registered elsewhere" listing must exclude its own entry
/// or it fires on every healthy agent (#2698). Same comparison as Tier 2a/2b forwarding.
pub(super) fn is_self_registration(entry_local_url: &str, this_instances_local_url: &str) -> bool {
    entry_local_url == this_instances_local_url
}

/// One cross-instance/channel entry from the host-global shared registry
/// (`backend::reactive::registry::AgentEntry`), narrowed to what the frontend needs for a
/// "registered elsewhere too" badge: `local_url`/`auth_key` are internal forwarding plumbing and
/// stay out.
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct RemoteRegistrationEntry {
    channel: String,
    // u32 is < 2^53 so ts-rs maps it to `number` already; only 64-bit
    // integers become `bigint`. See updated_at below.
    pid: u32,
    #[ts(type = "number")]
    updated_at: u64,
}

/// The most recent `identity-mismatch` audit entry for this agent_id, if any (#2695's
/// `Handler::inject_message_inner` check), narrowed to what the Stash badge needs.
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct MismatchAuditSummary {
    #[ts(type = "number")]
    timestamp: u64,
    block_id: String,
    error_message: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct ReactiveRegistrationsResult {
    /// This instance's own registration for the agent, if any — same data
    /// `GET /agentmux/reactive/agent` exposes, reused here so the frontend
    /// makes one call instead of two.
    local: Option<AgentRegistration>,
    /// Every OTHER instance/channel on this host currently claiming this
    /// same agent_id, freshest first — a non-empty list here is the actual
    /// risk signal the Stash badge exists to surface (issue #2694's root
    /// cause was exactly two panes racing to hold the same agent_id).
    remote: Vec<RemoteRegistrationEntry>,
    recent_mismatch: Option<MismatchAuditSummary>,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct ReactiveRegistrationsParams {
    pub agent_id: String,
}

/// Registers `reactive.registrations`, called by the Stash "Registration"
/// tab (`AgentIdentityLinksPanel`'s sibling — see `frontend/app/store/
/// rpc-api/reactive.ts`) to answer "is this agent's jekt identity healthy
/// right now": where it's registered locally, whether any OTHER
/// instance/channel on this host also claims the same agent_id (the
/// collision shape #2694 fixed one specific cause of), and whether a
/// recent delivery hit the #2695 identity-mismatch guard.
pub fn register_reactive_ws_handlers(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    // Typed registration (SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.1): the
    // params/result structs carry `#[derive(ts_rs::TS)]`, so the frontend
    // consumes generated bindings instead of four hand-written interfaces
    // that each said "mirrors agentmux-srv's ...".
    registration_flags::register(engine, state.clone());
    engine.register_typed(
        "reactive.registrations",
        move |params: ReactiveRegistrationsParams, _ctx| {
            let state = state.clone();
            async move {

                let local = state.reactive_handler.get_agent(&params.agent_id);

                let (remote, recent_mismatch) = registration_flags::problems_of(&state, &local, &params.agent_id);
                let result = ReactiveRegistrationsResult {
                    local,
                    remote,
                    recent_mismatch,
                };
                Ok(result)
            }
        },
    );
}

/// `verify_jekt_signature` unit tests (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
/// §2.2, reagentx review on PR #2565). Deliberately test the extracted
/// function directly rather than the full `handle_inject`/websocket
/// handlers it's now called from: `server::tests::test_state()`'s
/// `reactive_handler` is a *global* singleton shared across every test in
/// the binary (`backend_reactive::get_global_handler()`), so exercising it
/// end-to-end here risks cross-test interference on shared agent
/// registrations. `verify_jekt_signature` itself only touches `state.mstore`
/// (key lookup), not the handler, so it's safe to test in isolation with no
/// such risk — and it's the one piece of logic actually being fixed here;
/// the two call sites (messagebus.rs, websocket.rs) are a one-line "call
/// this before inject_message" wiring, visible directly in their diffs.
#[cfg(test)]
#[path = "tests/reactive/verify_jekt_signature_tests.rs"]
mod verify_jekt_signature_tests;

/// `verify_reagent_signature` unit tests (reagentx P1 on PR #41 —
/// `InjectionRequest` declared `reagent_sig`/`reagent_key_id` as
/// deserializable input fields but nothing on the HTTP
/// `/agentmux/reactive/inject` path ever verified them, so a reagent-signed
/// notification delivered through `@agentmuxai/muxbus-client`'s
/// `pollAndDeliverInjections` arrived unsigned in effect).
#[cfg(test)]
#[path = "tests/reactive/verify_reagent_signature_tests.rs"]
mod verify_reagent_signature_tests;

#[cfg(test)]
#[path = "tests/reactive/verify_lan_signature_tests.rs"]
mod verify_lan_signature_tests;

#[cfg(test)]
#[path = "tests/reactive/registration_flags_tests.rs"]
mod registration_flags_tests;

/// SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §9 — the unit half of the
/// test plan (items 1–8 and the D5 replay cases 9–10, driven through the
/// verifier rather than the raw primitives). Entries are written straight to
/// a temp shared-registry dir with an explicit public key, bypassing the
/// process-global pubkey resolver (a `OnceLock` another test binary-wide
/// test owns), so each test is hermetic and order-independent.
#[cfg(test)]
#[path = "tests/reactive/verify_cross_channel_signature_tests.rs"]
mod verify_cross_channel_signature_tests;

#[cfg(test)]
#[path = "tests/reactive/resolve_delivery_tier_tests.rs"]
mod resolve_delivery_tier_tests;

#[cfg(test)]
#[path = "tests/reactive/is_self_registration_tests.rs"]
mod is_self_registration_tests;

/// `forward_inject_to_peer` unit tests.
///
/// These exercise the real HTTP path against a throwaway loopback peer rather
/// than mocking the client, because the behaviour under test *is* the
/// response-classification: which peer answers count as delivery, which count
/// as evidence the candidate is stale, and which must never evict.
///
/// `source_agent` is left `None` throughout so `echo_jekt_to_sender` returns at
/// its first guard. That keeps these tests off the *global* singleton
/// `reactive_handler` (`test_state()` shares it across the whole test binary —
/// see the `verify_jekt_signature_tests` module comment), so nothing here can
/// interfere with another test's agent registrations.
#[cfg(test)]
#[path = "tests/reactive/forward_inject_tests.rs"]
mod forward_inject_tests;

/// `deliver`'s forwarding tiers, end to end, against stub peers.
#[cfg(test)]
#[path = "tests/reactive/deliver_tiers_tests.rs"]
mod deliver_tiers_tests;

/// PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md §5-§6: the relay's lease
/// answer as a route. Holding here is the default; only "another install
/// runs it" or an answer the relay cannot give plainly sends it to the relay.
#[cfg(test)]
#[path = "tests/reactive/absent_route_tests.rs"]
mod absent_route_tests;

/// Tier-4 (cloud relay) gating tests.
///
/// These cover the decisions `try_cloud_relay` makes *before* it would touch
/// the network — the guards that keep it from firing when it must not. The
/// wire contract itself (headers, body shape, status handling) is tested
/// against a stub relay in `crate::muxbus::relay`'s own test module.
///
/// A `test_state()` has no muxbus credential, so any path that reaches token
/// resolution returns `None` and no request is ever made — which is also why
/// these assert `None` rather than mocking the relay: the point is *which*
/// guard stopped it, and each test isolates one.
#[cfg(test)]
#[path = "tests/reactive/cloud_relay_gate_tests.rs"]
mod cloud_relay_gate_tests;

/// Request-shape guard for `reactive.registrations`, added as part of the
/// typed-registration migration.
///
/// `register_typed` deserializes the payload BEFORE the handler runs, so the
/// Req type has to accept exactly what the frontend stub sends. On
/// `bookmarks.list` that distinction shipped a P0 — a unit Req compiled fine
/// and then rejected the stub's `{}` at runtime, because serde accepts `()`
/// only from `null` (PR #3293). Neither `tsc` nor `check-rpc-bindings.sh`
/// catches that class: they verify generated-type drift, not runtime payload
/// shape. So every migrated command gets a test fed the real payload.
#[cfg(test)]
#[path = "tests/reactive/reactive_registrations_req_tests.rs"]
mod reactive_registrations_req_tests;
