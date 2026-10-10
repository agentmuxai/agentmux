// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::reactive::{AgentRegistration, InjectionRequest, SupervisorAction};
use crate::backend::reactive::registry as agent_registry;
use crate::backend::subagent_watcher;
use crate::backend::base;

use super::AppState;

/// Max cross-instance HTTP forwards a single inject request may go through
/// (Tier 2a/2b/3 each increment before forwarding). A legitimate delivery
/// is always exactly one hop (caller's instance → the owning instance);
/// this only exists to bound a pathological cycle — two channels each
/// holding a stale-but-PID-alive shared-registry entry pointing at the
/// other for the same agent name would otherwise forward back and forth
/// indefinitely, hanging the original request (reagent P1 on PR #2350).
const MAX_FORWARD_HOPS: u8 = 3;

/// Echo a successfully-sent jekt into the SENDER's own pane
/// (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
///
/// Appends a `{"type":"user",...}` NDJSON line carrying the same
/// `[JEKT:...]` marker block the receiver got (re-wrapped with identical
/// fields) to the sender's `output` blockfile — live MPS append (renders
/// immediately in an open agent view), persisted history
/// (`parseHistoryLines` rebuilds on reopen), and global transcript mirror.
/// The frontend's `tryParseJekt` sees FROM == this pane's agent and renders
/// it as an *outgoing* JektBubble (stream-parser.ts direction detection —
/// this is the producer that comment says doesn't exist yet).
///
/// No-op when the sender isn't a registered agent on this instance (cron,
/// external callers) or is messaging itself (the incoming marker already
/// lands in the same pane).
///
/// `reagent_verified`/`lan_verified` should be the SAME values the original
/// delivery's `Handler::inject_message_inner` computed `effective_tier`/
/// `requires_stop` from (not re-derived) — see the call site's own doc
/// comment inline below for why passing anything else produces a
/// self-contradictory echoed marker (reagentx P1 on PR #2623).
/// The delivery-tier + verification signals an echoed marker must carry.
///
/// Grouped into a named struct rather than left as positionals because
/// `sig_verified`, `reagent_verified` and `lan_verified` are three consecutive
/// `Option<bool>`s: any two of them can be transposed at a call site and the
/// compiler cannot say a word. The failure mode is a silently mislabelled TRUST
/// field on a security marker, which is not the kind of bug to leave to
/// argument order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EchoTrust<'a> {
    pub delivery_tier: &'a str,
    pub sig_verified: Option<bool>,
    pub reagent_verified: Option<bool>,
    pub lan_verified: Option<bool>,
    pub channel_verified: Option<bool>,
}

/// The trust an echoed marker should carry after a forward, given the
/// peer's response body. The RECEIVING instance is the one that computes
/// `channel_verified` (the forwarder holds the sender's HMAC key, so §D2
/// step 2 of SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md makes it skip
/// cross-channel verification) and threads it back via
/// `InjectionResponse::channel_verified`, exactly as it already does for
/// `effective_tier`/`requires_stop`. Taking it from the body keeps the
/// sender's echoed `TRUST=` consistent with the `ESCALATE=` the same body
/// supplies — otherwise a sensitive cross-channel message could echo as
/// `TRUST=self-declared … ESCALATE=none`, which contradicts itself (codex
/// P2 on #3064). A body without the field (an older peer) leaves the
/// caller's value alone.
fn echo_trust_from_peer_body<'a>(body: &serde_json::Value, base: EchoTrust<'a>) -> EchoTrust<'a> {
    EchoTrust {
        channel_verified: body
            .get("channel_verified")
            .and_then(|v| v.as_bool())
            .or(base.channel_verified),
        ..base
    }
}

pub(super) fn echo_jekt_to_sender(
    state: &AppState,
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
    msgid: &str,
    effective_tier: Option<&str>,
    requires_stop: Option<bool>,
    trust: EchoTrust<'_>,
    priority: &str,
) {
    let EchoTrust {
        delivery_tier,
        sig_verified,
        reagent_verified,
        lan_verified,
        channel_verified,
    } = trust;
    let Some(src) = source_agent.filter(|s| !s.is_empty()) else {
        return;
    };
    if src.eq_ignore_ascii_case(target_agent) {
        return;
    }
    let Some(sender_reg) = state.reactive_handler.get_agent(src) else {
        return;
    };

    let sanitized = crate::backend::reactive::sanitize::sanitize_message(message);
    let wrapped = crate::backend::reactive::sanitize::wrap_jekt_message(
        &sanitized,
        Some(&sender_reg.agent_id),
        target_agent,
        effective_tier.unwrap_or("coord"),
        delivery_tier,
        sig_verified,
        // reagentx P1 on PR #2623: these used to be hardcoded `None, None`
        // ("sender-echo is inherently host-tier, so WAN/LAN verification is
        // meaningless here") — true for TRUST/SIG rendering alone, but as of
        // SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md
        // that assumption broke: `requires_stop` (just above) already
        // reflects the REAL delivery's verification, so hardcoding these to
        // `None` produced a self-contradictory echoed marker — `ESCALATE=none`
        // (implying a verified sender) next to `TRUST=network-claimed` with
        // no `SIG=` field (implying an unverified one). Passing the same
        // verification signals the original decision was made from keeps
        // the echoed marker internally consistent with its own `ESCALATE=`.
        reagent_verified,
        lan_verified,
        channel_verified,
        // The sender-side echo never has a WAN verdict: WAN verification
        // happens only at the receiving install (W3-S §2.3), after the echo.
        None,
        // Defaults to `true` (STOP) when the caller couldn't tell us —
        // matches `effective_tier` defaulting to the more-cautious "coord"
        // rather than assuming "info" above; never silently downgrades a
        // sensitive echo to tag-only just because this hop lost the signal.
        requires_stop.unwrap_or(true),
        msgid,
        priority,
        None,
    );
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": wrapped }
    });
    let data = format!("{line}\n");
    let global_zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(
        &Some(state.mstore.clone()),
        &sender_reg.block_id,
    );
    crate::backend::blockcontroller::shell::handle_append_block_file(
        &state.broker,
        &sender_reg.block_id,
        crate::backend::agent_session::OUTPUT_FILE,
        data.as_bytes(),
        Some(&state.filestore),
        global_zone.as_deref(),
    );
}

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
}

/// The single implementation of "POST this injection to a peer srv and echo it
/// to the sender if it lands", shared by every forwarding tier.
///
/// This existed as three hand-maintained copies (same-host same-channel,
/// same-host cross-channel, LAN peer) differing only in how the URL and key
/// were resolved and what they did on failure. That is the shape that has
/// already produced drift bugs elsewhere in this repo: a tier that forgets the
/// echo, or passes `delivery_tier` wrong, fails silently and differently from
/// its siblings. See
/// `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md` §4.
///
/// **One behaviour was unified rather than preserved:** the LAN tier used to
/// treat "anything but `success: false`" as delivered, while both host tiers
/// required `success: true`. A 200 response whose body omits `success`
/// entirely therefore counted as a successful LAN delivery — echoing a
/// delivery confirmation to the sender for a message that may never have
/// arrived. All three now require `success: true`. No real peer is affected:
/// `InjectionResponse::success` is a plain `bool` with no
/// `skip_serializing_if`, so a genuine AgentMux srv always sends it.
async fn forward_inject_to_peer(
    state: &AppState,
    req: &InjectionRequest,
    forwarded_req: &InjectionRequest,
    peer: ForwardPeer<'_>,
) -> ForwardOutcome {
    let forward_url = format!("{}/agentmux/reactive/inject", peer.url);
    let channel = peer.channel.unwrap_or("-");
    tracing::debug!(
        target = %req.target_agent,
        url = %forward_url,
        kind = peer.kind,
        channel,
        "inject forward"
    );

    let mut fwd = state.http_client.post(&forward_url).json(forwarded_req);
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
            echo_jekt_to_sender(
                state,
                req.source_agent.as_deref(),
                &req.target_agent,
                &req.message,
                body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                body.get("effective_tier").and_then(|v| v.as_str()),
                body.get("requires_stop").and_then(|v| v.as_bool()),
                echo_trust_from_peer_body(&body, peer.trust),
                req.priority.as_deref().unwrap_or("normal"),
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
            tracing::warn!(
                target = %req.target_agent,
                error = %e,
                url = %forward_url,
                kind = peer.kind,
                channel,
                "inject forward: transport failure — candidate may be stale"
            );
            ForwardOutcome::Stale
        }
    }
}

/// Max age (seconds) a signed jekt's `ts_secs` may be from "now" and still
/// verify (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, anti-replay
/// — reagentx P1 on PR #2565: `ts_secs` was bound into the signed material
/// specifically for this purpose per `jekt_sign.rs`'s own doc comments, but
/// nothing actually checked it, so a captured valid signature verified
/// forever). Generous enough for normal host-tier delivery latency and
/// modest clock skew between the signing agent process and this srv
/// instance (same machine, but not guaranteed same clock read down to the
/// second); tight enough to bound replay to a narrow window instead of
/// indefinite reuse.
const JEKT_SIG_MAX_AGE_SECS: i64 = 300;

use agentmux_common::{time::now_secs as now_unix_secs, AUTH_KEY_HEADER};

/// Host-tier jekt sender verification (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
/// §2.2). Mutates `req.sig_verified` in place based on whether the claimed
/// `source_agent`'s stored signing key (if any) verifies `req.jekt_sig`,
/// within the anti-replay freshness window.
///
/// **Every entry point that can build a host-tier `InjectionRequest` from
/// client-supplied fields MUST call this before handing it to
/// `Handler::inject_message`** — reagentx P0 on PR #2565 found two call
/// sites (`messagebus.rs::handle_inject`, `websocket.rs`'s `bus:inject`
/// message handling) that built `InjectionRequest` with a fully
/// client-controlled `source_agent` and called `inject_message` directly,
/// bypassing this entirely — those messages rendered `TRUST=self-declared`
/// (unescalated) exactly as if this feature didn't exist. Both now call
/// this too.
///
/// **Deliberately NOT gated on `delivery_tier == "host"`** (reagentx P0 on
/// the LAN signing PR — this WAS gated that way originally, and it was the
/// bug): `delivery_tier` is a value this instance largely trusts as
/// self-declared by whoever authenticated with the full local `auth_key`
/// (needed so legitimate same-host forwarding can carry an already-`"lan"`
/// jekt through unmolested — see `resolve_delivery_tier`'s doc comment).
/// That means a request claiming `delivery_tier: "lan"` (or `"wan"`) was
/// otherwise a way to dodge THIS check entirely — impersonate a real,
/// locally-known agent by simply not calling it "host," since
/// `verify_lan_signature`/`verify_reagent_signature` only fire for their
/// own tiers and leave an unsigned claim unforced by design. Running this
/// check unconditionally closes that: it only ever does anything when
/// `agent_jekt_key_load` finds a LOCAL key for the claimed `source_agent` —
/// which is `None` for any genuinely-remote LAN/WAN sender this instance
/// never spawned (no behavior change for real network traffic), but
/// catches an unsigned/wrong-signature impersonation of an agent THIS
/// instance actually knows, regardless of what tier the request claims.
pub(super) fn verify_jekt_signature(state: &AppState, req: &mut InjectionRequest) {
    let Some(claimed) = req.source_agent.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    let Ok(Some(key)) = state.mstore.agent_jekt_key_load(&claimed) else {
        return;
    };
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
    req.sig_verified = Some(verified);
}

/// Anti-replay window for `req.reagent_ts_secs`, same purpose as
/// `JEKT_SIG_MAX_AGE_SECS` above but WAN-scoped: wider than host-tier's
/// 300s because this covers real network delivery latency, not a
/// same-machine call — matches `cloud_subscriber::REAGENT_SIG_MAX_AGE_SECS`
/// (the WS delivery path's own constant of the same value) and the review
/// notifications' own delivery window.
const REAGENT_SIG_MAX_AGE_SECS: i64 = 600;

/// WAN-tier reagent-signature verification for the HTTP
/// `/agentmux/reactive/inject` path — mirrors
/// `cloud_subscriber::sync_agent_reactive`'s in-process verification of the
/// same four fields for the desktop app's WS delivery path, but for callers
/// that deliver over HTTP instead (`@agentmuxai/muxbus-client`'s
/// `pollAndDeliverInjections`, and any future standalone poller). Before
/// this, `InjectionRequest` declared `reagent_sig`/`reagent_key_id` as
/// deserializable input fields but nothing on the HTTP path ever read them
/// — a reagent-signed notification delivered through this path arrived
/// unsigned in effect, `reagent_verified` always `None`, and could never
/// render `SIG=verified` (reagentx P1 on PR #41).
///
/// Only meaningful for `delivery_tier == "wan"` — same scoping as
/// `reagent_verified`'s doc comment ("meaningless off the WAN tier"). A
/// partial set of the four fields (e.g. a sig but no key_id) is treated the
/// same as "not signed" (`reagent_verified` stays `None`), not "signed but
/// broken" — matches `cloud_subscriber.rs`'s identical policy: a legitimate
/// sender always sends all four together, and this field never affects
/// `TIER`/`TRUST` escalation either way, so a stripped signature can't buy
/// an attacker anything a fully-absent one couldn't already.
///
/// Takes `now` explicitly (rather than calling `now_unix_secs()` itself),
/// same reasoning as `cloud_subscriber::reagent_sig_is_fresh`: the pinned
/// Ed25519 key's matching private half isn't in this repo (it lives only in
/// the cloud service), so tests can't mint a fresh signature
/// on demand the way the host-tier HMAC tests below do — only a fixed
/// offline-signed fixture at a fixed `ts_secs`. Injecting `now` lets a test
/// hold that fixture inside the freshness window without mocking the clock.
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

/// Anti-replay window for LAN `lan_sig` — reuses the WAN reasoning
/// (`REAGENT_SIG_MAX_AGE_SECS`) rather than host-tier's tighter
/// `JEKT_SIG_MAX_AGE_SECS`: LAN crosses an actual network hop (mDNS
/// discovery + an HTTP round trip through a peer instance), not a
/// same-process call, so it needs the wider real-network-delivery-latency
/// margin WAN already established rather than host-tier's same-machine one.
const LAN_SIG_MAX_AGE_SECS: i64 = REAGENT_SIG_MAX_AGE_SECS;

/// LAN-tier per-agent Ed25519 signature verification —
/// docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.4. Async, unlike
/// `verify_jekt_signature`'s synchronous local-only key lookup: the claimed
/// sender's public key lives on WHICHEVER peer instance actually hosts that
/// agent, so finding it costs a LAN round trip
/// (`LanDiscoveryController::find_agent_lan_pubkey`).
///
/// Only meaningful for `delivery_tier == "lan"` — by this point
/// `req.delivery_tier` has already been through the server-side override in
/// `handle_reactive_inject` (§3 of the spec), so this is never reachable
/// off a genuinely `lan_key`-authenticated request.
///
/// No `lan_sig` at all, or a claimed sender whose public key no peer has on
/// file → `lan_verified` stays `None` — "nothing to check against," not a
/// red flag on its own, same semantics as `sig_verified`/`reagent_verified`'s
/// `None` case. A `lan_sig` present with a public key found but the
/// signature doesn't verify → `Some(false)`, forced `TIER=sensitive`
/// unconditionally in `handler.rs` — an active attempt to forge a specific
/// agent's identity.
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
    let observed_key = match state.lan_discovery.find_agent_lan_pubkey(&claimed, &state.http_client).await {
        crate::backend::lan_discovery::LanPubkeyLookup::Found(key) => key,
        // Genuinely unknown sender — nothing to check against, not a red
        // flag on its own (same treatment self-declared senders get
        // elsewhere).
        crate::backend::lan_discovery::LanPubkeyLookup::NotFound => return,
        // reagentx P0 follow-up: the lookup was SKIPPED (rate-limited), not
        // genuinely absent — a lan_sig WAS presented (checked above), so
        // "we didn't check" must not collapse into the same benign outcome
        // as "nothing to check." Treat conservatively as a failure, same as
        // an active forgery attempt — the alternative lets an attacker
        // exhaust the rate limiter to slip a forged identity claim through
        // unverified instead of forced-sensitive.
        crate::backend::lan_discovery::LanPubkeyLookup::RateLimited => {
            tracing::warn!(
                agent_id = %claimed,
                "LAN pubkey lookup was rate-limited while verifying a signed jekt — \
                 treating as unverified/failed rather than silently passing it through"
            );
            req.lan_verified = Some(false);
            return;
        }
    };

    // Trust-on-first-use pin (reagentx P0 — mDNS peer discovery is
    // unauthenticated, so "whichever peer answers first" is not a safe
    // trust anchor on its own; see lan_peer_pubkey_pins.rs's module doc and
    // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2). The first
    // key ever observed for a claimed sender is pinned; a LATER lookup
    // returning a DIFFERENT key is itself treated as an active red flag —
    // someone is now claiming a different identity than what was already
    // established — not silently trusted as an update.
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let observed_key_b64 = BASE64.encode(&observed_key);
    let Ok(pinned_key_b64) = state.mstore.lan_peer_pubkey_pin_get_or_set(&claimed, &observed_key_b64) else {
        return;
    };
    if pinned_key_b64 != observed_key_b64 {
        tracing::warn!(
            agent_id = %claimed,
            "LAN pubkey mismatch against pinned key — possible identity spoofing attempt"
        );
        req.lan_verified = Some(false);
        return;
    }

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
    req.lan_verified = Some(verified);
}

mod channel_v2;

/// Anti-replay window for cross-channel `channel_sig` —
/// SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D6: host-tier's tighter
/// `JEKT_SIG_MAX_AGE_SECS`, NOT LAN/WAN's wider one. A cross-channel forward
/// is a same-machine HTTP call to `127.0.0.1`; it has no real-network
/// latency budget to accommodate, for the same reason host-tier doesn't.
const CHANNEL_SIG_MAX_AGE_SECS: i64 = JEKT_SIG_MAX_AGE_SECS;

/// Cross-channel (same machine, different AgentMux instance) per-agent
/// Ed25519 signature verification —
/// docs/specs/SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D2, Phase B.
///
/// Mirrors `verify_lan_signature`'s structure but resolves the claimed
/// sender's public key from the host-global shared registry
/// (`AgentEntry::jekt_public_key`, published at registration by whichever
/// instance actually spawned that agent — §D1) instead of a LAN round trip.
/// A local file read, so this is synchronous and has none of the LAN
/// lookup's rate-limiter `Skipped` hazard.
///
/// Only meaningful for `delivery_tier == "channel"` — the label a same-host
/// forward now carries (§D4, set by the forwarding instance in
/// `handle_reactive_inject`'s Tier 2a/2b). Off that tier the field stays
/// `None`, same scoping as `lan_verified`.
///
/// Ordering guarantee (§D2 step 2): a claimed sender THIS instance holds an
/// HMAC key for is a same-instance agent, and `verify_jekt_signature` owns
/// it entirely — this function returns without touching `channel_verified`,
/// so the two verifiers can never disagree about one message.
///
/// Outcomes, all three-state like its siblings:
/// - `None` — no shared-registry entry for the claimed sender anywhere (a
///   genuine bridge, or an agent that never registered), OR every entry
///   found has an empty `jekt_public_key` (written before Phase A shipped —
///   "cannot check," never "failed the check," §6). Nothing to verify
///   against; not a red flag on its own.
/// - `Some(true)` — `channel_sig` present, fresh, and verified against the
///   published key of ANY of that agent's entries (one agent name can
///   legitimately be live in several channels at once — §D2 step 5).
/// - `Some(false)` — a published key WAS found but the signature was
///   missing, stale, or wrong. The §D3 red flag; **not escalated in Phase
///   B** (see `InjectionRequest::channel_verified`'s doc comment).
pub(super) fn verify_cross_channel_signature(state: &AppState, req: &mut InjectionRequest) {
    let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() else {
        return;
    };
    verify_cross_channel_signature_in(state, req, &shared_dir, now_unix_secs());
}

/// [`verify_cross_channel_signature`] with the shared registry dir and clock
/// injected — the testable core; the wrapper above only resolves the real
/// dir. Keeps tests off the process-global `AGENTMUX_SHARED_DIR` env var.
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
    // Identity M4d-6: a v2 signature proves which agent sent it, by UID. When
    // that is the agent this instance knows by the claimed name, the name's
    // checks are settled, including a failed HMAC check against a key it kept
    // from when that agent ran here (a key outlives a move between
    // instances). A different agent of the same name is not the name's.
    if channel_v2::settle_by_uid(state, req, &claimed, shared_dir, now_secs) {
        return;
    }
    // §D2 step 2: a same-instance sender is the HMAC path's to judge.
    if matches!(state.mstore.agent_jekt_key_load(&claimed), Ok(Some(_))) {
        return;
    }
    // Only entries that actually published a key count — a pre-Phase-A
    // entry (empty key) is "cannot check," and must not turn a missing
    // signature into `Some(false)` on a mixed-version machine (§6).
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

/// Server-derived `delivery_tier` for the LAN-key case only —
/// docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §3, revised after
/// reagentx P0 on the implementing PR: an earlier version of this override
/// also force-downgraded a "lan" claim to "host" whenever auth was via the
/// full `auth_key`, reasoning that "nothing authenticated via the full key
/// could have genuinely crossed the LAN boundary." That's false once
/// same-host forwarding (Tier 2a/2b below in `handle_reactive_inject`) is
/// considered: a jekt legitimately authenticated via `lan_key` at its FIRST
/// hop, then relayed to a sibling channel/instance on this same machine,
/// authenticates that SECOND hop with the sibling's own full `auth_key`
/// (not `lan_key` — that credential is peer-to-peer between distinct
/// machines, never shared across same-host channels). Downgrading there
/// silently discarded an already-detected LAN signature failure's
/// forced-sensitive escalation (`lan_verified`/`reagent_verified` are
/// `#[serde(skip_deserializing)]`, so they reset to `None` on every hop and
/// must be free to re-derive from whatever `delivery_tier` the forward
/// legitimately carries).
///
/// The actual gap this closes is narrower than "full auth_key can never
/// claim lan": a `lan_key` holder is the ONLY credential that can FORCE
/// `delivery_tier = "lan"` regardless of what the body says (closing the
/// original bypass — claim "host" instead to dodge LAN scrutiny entirely).
/// A full-`auth_key` caller's own claim (host/wan/lan) is otherwise trusted
/// as-is: holding the full local key already grants complete control over
/// this instance, so which tier it self-labels a request as grants nothing
/// extra — at worst it triggers MORE verification
/// (`verify_lan_signature` running), never less.
///
/// `"channel"` (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4) rides the
/// same rule: it's set by the FORWARDING instance on a same-host Tier 2a/2b
/// hop and authenticates here with this instance's full `auth_key`, so a
/// full-key caller's own `"channel"` claim is honoured as-is. Claiming it
/// gains nothing — it only adds `verify_cross_channel_signature` on top of
/// the unconditional `verify_jekt_signature`, never removes a check.
fn resolve_delivery_tier(auth_via: super::ReactiveAuthVia, claimed: Option<&str>) -> String {
    if auth_via == super::ReactiveAuthVia::LanKey {
        "lan".to_string()
    } else {
        claimed.unwrap_or("host").to_string()
    }
}

/// The `delivery_tier` a same-host Tier 2a/2b forward carries to the peer
/// instance — SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4. See the call
/// site in `handle_reactive_inject` for the full reasoning; in short: a
/// `host`-tier (or unlabelled) request becomes `channel` on the hop, and
/// anything already carrying a network tier keeps it so the peer re-derives
/// that tier's own verification.
fn same_host_forward_tier(current: Option<&str>) -> &str {
    match current {
        None | Some("host") => "channel",
        Some(other) => other,
    }
}

/// Server-side resolution of `InjectionRequest::is_transcript_request` /
/// `transcript_request_escalate_forced` — see both fields' own doc comments
/// (`backend/reactive/types.rs`) and
/// `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`. Lives here (not
/// `Handler::inject_message_inner`) because it needs `Store` access —
/// `Handler` has no `Store` access "by design," same reason
/// `sig_verified`/`lan_verified` are resolved by this same caller before
/// the request reaches the handler.
///
/// Takes `mstore: &Arc<Store>` directly (not `&AppState`) so
/// `muxbus::cloud_subscriber::sync_agent_reactive` — the WAN delivery path,
/// which calls `Handler::inject_message` directly and never goes through
/// `handle_reactive_inject`/HTTP at all — can call this exact same
/// resolution too (`pub(crate)`). Phase C (WAN) needs the identical rule 1/
/// rule 2 computation this function already does; without also wiring it
/// in there, a WAN-delivered `transcript_request` would silently skip both
/// rules entirely, not just get weaker ones.
///
/// Always re-parses `req.message` itself — never trusts anything the
/// client might have set on these two fields (impossible anyway, since
/// both are `#[serde(skip_deserializing)]`, but this function is the one
/// place that actually computes their real value from scratch).
pub(crate) fn resolve_transcript_request_tier_fields(mstore: &std::sync::Arc<crate::backend::storage::store::Store>, req: &mut InjectionRequest) {
    let Some(transcript_req) = agentmux_common::transcript_request::parse_transcript_request(&req.message) else {
        return;
    };
    let _ = transcript_req; // request_id/max_lines belong to the (not-yet-built) auto-responder, not tier resolution.
    req.is_transcript_request = true;

    // Match on the RESPONDING agent's (target_agent's) own slug — the
    // stable, AGENTMUX_AGENT_ID-derived identifier, NOT the renameable
    // display `name` (same cross-namespace hazard already documented at
    // this file's Supervisor-nudge opt-in check just above, which this
    // mirrors exactly).
    let visibility = mstore
        .agent_def_list()
        .ok()
        .and_then(|defs| defs.into_iter().find(|d| d.slug.eq_ignore_ascii_case(&req.target_agent)))
        .map(|d| d.conversation_visibility)
        .unwrap_or_else(crate::backend::storage::agents::default_conversation_visibility);

    let tier = req.delivery_tier.as_deref().unwrap_or("host");
    req.transcript_request_escalate_forced = match visibility.as_str() {
        "ask" => true,
        "trusted_peers" => {
            let requester = req.source_agent.as_deref().unwrap_or("");
            let granted = mstore
                .conversation_trust_grant_check(&req.target_agent, requester, tier)
                .unwrap_or(false);
            !granted
        }
        // "private" and any unrecognized value: fail-closed on the
        // ESCALATE-forcing question too — an agent def loaded from a
        // channel/registry state older than this feature (or IS
        // genuinely "private") never had a chance to opt into
        // relaxation, so it gets none. Rule 1's forced TIER=sensitive
        // still applies regardless (set above) — only this ADDITIONAL
        // escalate-forcing rule reads "private" as "no need to force it
        // beyond the ordinary verified-sender relaxation," since a
        // "private" agent auto-denies (once the responder auto-resolve
        // is built) and was never going to disclose anything either way.
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
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::Inject,
        req.source_agent.as_deref(),
    );
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );
    let caller_uid = super::caller::attributed_uid(caller.as_deref());
    Json(deliver(&state, auth_via, &caller_uid, req).await)
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
    req.delivery_tier = Some(resolve_delivery_tier(auth_via, req.delivery_tier.as_deref()));
    // Identity M4c-2d: the sender's UID, for the audit entry only — never
    // forwarded (`#[serde(skip)]`).
    req.audit_source_uid = caller_uid.to_string();
    // Identity M4d-6: the signed source_uid is the sender's to set; this srv
    // only counts one that isn't the UID its token named.
    if !caller_uid.is_empty() && req.source_uid.as_deref().is_some_and(|s| s != caller_uid) {
        crate::backend::agent_resolve::record_uid_fallback("m4.source_uid_mismatch");
    }

    verify_jekt_signature(&state, &mut req);
    verify_reagent_signature(&mut req, now_unix_secs());
    verify_lan_signature(&state, &mut req).await;
    verify_cross_channel_signature(&state, &mut req);
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);

    // 1. Try local ReactiveHandler first (fast path — same instance).
    let resp = state.reactive_handler.inject_message(req.clone());
    if resp.success {
        echo_local_delivery(state, &req, &resp);
        return serde_json::to_value(&resp).unwrap_or_default();
    }

    // The target is here but its spawn gate refused (not signed in). That is
    // recoverable, so hold it for sign-in instead of failing it
    // (SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md Phase 0 item 4).
    // No other tier can run it either: the agent lives on this instance.
    if resp.error.as_deref().is_some_and(super::jekt_held::is_spawn_gate_refusal) {
        if let Some(held) = hold_for_absent_target(state, auth_via, &req, &resp, HoldReason::NeedsLogin).await {
            return held;
        }
        return serde_json::to_value(&resp).unwrap_or_default();
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

    // Every forward below sends this hop-incremented request, not the
    // original `req` — a peer that also fails to find the agent locally
    // and forwards onward needs to see the accumulated hop count too.
    let mut forwarded_req = req.clone();
    forwarded_req.forward_hops = req.forward_hops.saturating_add(1);

    // SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4: a same-host hop to a
    // DIFFERENT instance (Tier 2a/2b below) is labelled `channel`, not
    // `host` — `host` is reserved for genuinely same-instance traffic, so
    // the label finally matches the guarantee (the receiving instance holds
    // no HMAC key for a sender it didn't spawn; §2.3 of the spec). The
    // receiver can't tell a forward from a local call by auth alone (the
    // forward authenticates with the receiver's own full `auth_key`), so
    // the forwarding side says so here, and `resolve_delivery_tier` on the
    // far side honours a full-key caller's claim as it always has.
    //
    // An already-`lan`/`wan` hop keeps its label: that's what lets the peer
    // re-derive its own tier-specific verification on the second hop
    // (`resolve_delivery_tier`'s doc comment, reagentx P0 on the LAN signing
    // PR). A `channel` hop forwarded again stays `channel`. Tier 3 (LAN)
    // sends `forwarded_req` untouched — the peer authenticates it via
    // `lan_key` and forces `lan` regardless of the body.
    let same_host_tier = same_host_forward_tier(req.delivery_tier.as_deref());
    let mut same_host_req = forwarded_req.clone();
    same_host_req.delivery_tier = Some(same_host_tier.to_string());

    // Whether any forwarding tier found the target alive elsewhere. A target
    // that is alive but refused is not "absent" and is never held here
    // (durable jekt spec §2.1, condition 3); a same-host entry whose process
    // is dead — every entry after an srv restart, which changes its port —
    // is not a candidate, so the first message after a restart is held.
    let mut candidate_seen = false;
    if is_not_found {
        // Tier 2: same-host, different sidecar (file registry → HTTP loopback)
        let data_dir = base::get_mux_data_dir();
        if let Some(entry) = agent_registry::lookup(&data_dir, &req.target_agent) {
            // Guard against self-forwarding loops.
            if entry.local_url != state.local_web_url {
                match forward_inject_to_peer(
                    &state,
                    &req,
                    &same_host_req,
                    ForwardPeer {
                        url: &entry.local_url,
                        auth_key: &entry.auth_key,
                        kind: "cross-instance",
                        channel: None,
                        trust: EchoTrust {
                            delivery_tier: same_host_tier,
                            sig_verified: req.sig_verified,
                            reagent_verified: req.reagent_verified,
                            lan_verified: req.lan_verified,
                            channel_verified: req.channel_verified,
                        },
                    },
                )
                .await
                {
                    ForwardOutcome::Delivered(body) => return body,
                    // A non-delivery here is ambiguous: this entry may be
                    // stale (agent unregistered without a clean shutdown), OR
                    // the owning process may be alive and simply not have
                    // registered this specific agent yet — e.g. right after
                    // that channel's srv came up. Connection-level failure is
                    // at least as plausible a transient trigger as a parsed
                    // success:false body, which is why both land here.
                    // `should_evict_on_forward_failure` combines PID-liveness
                    // with the entry's age: a dead process always evicts; a
                    // live process only protects a FRESH entry (the actual
                    // startup race), not an old one — an old entry whose
                    // process happens to still be alive for OTHER agents is
                    // presumed to be its own genuinely-dead agent (reagent P1
                    // round 2 on #2640: PID alone over-protects, since it
                    // identifies the whole srv process, not this one agent).
                    // See docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                    //
                    // Falls through to Tier 2b/3 either way (reagent P1 on
                    // #2350 — this previously returned unconditionally
                    // whenever the body parsed, regardless of success, so a
                    // stale same-channel entry never reached a later tier).
                    ForwardOutcome::Stale => {
                        if agent_registry::should_evict_on_forward_failure(&entry) {
                            tracing::warn!(
                                target = %req.target_agent,
                                pid = entry.pid,
                                "cross-instance forward: entry presumed dead — evicting and falling through"
                            );
                            agent_registry::remove(&data_dir, &req.target_agent);
                        } else {
                            // Alive but did not take it: the target exists
                            // elsewhere, so it is not held here.
                            candidate_seen = true;
                            tracing::warn!(
                                target = %req.target_agent,
                                pid = entry.pid,
                                "cross-instance forward: entry is fresh and owning process is alive — NOT evicting, falling through"
                            );
                        }
                    }
                    // Says nothing about this entry's liveness — leave it be,
                    // and do not treat the target as absent.
                    ForwardOutcome::Inconclusive => candidate_seen = true,
                }
            }
        }

        // Tier 2b: same host, DIFFERENT channel (host-global shared registry).
        // Runs when Tier 2a had no same-channel entry or its forward already
        // failed above — closes the gap issue #1916 tracked (Tier 2 previously
        // only ever reached agents in the caller's own channel). Candidates are
        // tried freshest-first (§4.3 of the cross-channel delivery spec);
        // a failed forward evicts just that channel's entry and falls through
        // to the next candidate, same evict-on-fail shape Tier 3 already uses.
        if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
            let candidates = agent_registry::lookup_all_shared(&shared_dir, &req.target_agent);
            for entry in candidates {
                // Self-forward guard, matching Tier 2a. Also loopback-only
                // (§5 of the spec): a poisoned registry entry can't redirect
                // a forward off-box, since resolve_shared_reactive_dir() is a
                // same-user local file, but defense in depth costs nothing here.
                let is_loopback = entry.local_url.starts_with("http://127.0.0.1")
                    || entry.local_url.starts_with("http://localhost")
                    || entry.local_url.starts_with("http://[::1]");
                if !is_loopback || entry.local_url == state.local_web_url {
                    continue;
                }

                match forward_inject_to_peer(
                    &state,
                    &req,
                    &same_host_req,
                    ForwardPeer {
                        url: &entry.local_url,
                        auth_key: &entry.auth_key,
                        kind: "cross-channel",
                        channel: Some(&entry.channel),
                        trust: EchoTrust {
                            delivery_tier: same_host_tier,
                            sig_verified: req.sig_verified,
                            reagent_verified: req.reagent_verified,
                            lan_verified: req.lan_verified,
                            channel_verified: req.channel_verified,
                        },
                    },
                )
                .await
                {
                    ForwardOutcome::Delivered(body) => return body,
                    // Same ambiguity and same should_evict_on_forward_failure
                    // policy as Tier 2a — PID-liveness alone over-protects an
                    // old, genuinely-dead individual agent whose srv process
                    // another agent keeps alive (reagent P1 round 2 on #2640),
                    // and a connection failure can be the same startup-race
                    // transient (target channel's srv not listening yet). See
                    // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                    // Evicting only this channel's entry lets the loop try the
                    // next candidate.
                    ForwardOutcome::Stale => {
                        if agent_registry::should_evict_on_forward_failure(&entry) {
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                pid = entry.pid,
                                "cross-channel forward: entry presumed dead — evicting and trying next candidate"
                            );
                            agent_registry::remove_shared(&shared_dir, &req.target_agent, &entry.channel);
                        } else {
                            candidate_seen = true;
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                pid = entry.pid,
                                "cross-channel forward: entry is fresh and owning process is alive — NOT evicting, trying next candidate"
                            );
                        }
                    }
                    ForwardOutcome::Inconclusive => candidate_seen = true,
                }
            }
        }

        // Tier 3: LAN peer (mDNS lookup → HTTP). Runs when tier 2 had no registry
        // entry or its forward failed. Queries each discovered LAN peer for the
        // agent; result is cached for 60s to avoid per-inject mDNS fan-out.
        if let Some((peer_url, peer_auth_key)) = state
            .lan_discovery
            .find_agent(&req.target_agent, &state.http_client)
            .await
        {
            match forward_inject_to_peer(
                &state,
                &req,
                &forwarded_req,
                ForwardPeer {
                    url: &peer_url,
                    auth_key: &peer_auth_key,
                    kind: "lan-peer",
                    channel: None,
                    trust: EchoTrust {
                        delivery_tier: "lan",
                        sig_verified: None,
                        // reagent signing is WAN-only; it never applies on the
                        // LAN forward path.
                        reagent_verified: None,
                        lan_verified: req.lan_verified,
                        channel_verified: req.channel_verified,
                    },
                },
            )
            .await
            {
                ForwardOutcome::Delivered(body) => return body,
                // The peer answered "not mine" (agent migrated away since we
                // cached it) or was unreachable — either way the discovery
                // cache entry is wrong. Unlike the registry tiers there is no
                // freshness/PID guard to weigh: the cache is cheap to refill
                // from mDNS, so evicting on any non-delivery is the right
                // trade rather than an oversight.
                // A stale route is evicted as wrong, so it is not a live
                // candidate (Codex P1 on #3632): the message may still be
                // held. Only an inconclusive answer leaves the target
                // possibly alive there.
                ForwardOutcome::Stale => {
                    state.lan_discovery.evict_agent(&req.target_agent);
                }
                ForwardOutcome::Inconclusive => candidate_seen = true,
            }
        }

        // Tier 4: cloud relay (muxbus store-and-forward). Runs when no local,
        // same-host or LAN tier could place the message.
        //
        // This tier used to be a comment here delegating to a "muxbus-client"
        // that the callers agents actually use are not — the MCP `SendMessage`
        // tool POSTs to this very endpoint and bails on `success != true`, so
        // the chain silently stopped at three while that tool's description
        // promised "local → LAN → cloud". Owning it here means every caller
        // inherits it. See `crate::muxbus::relay` and
        // REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md §5.
        //
        // Except for an agent defined here that no tier found live: that one
        // is held here unless the relay says another install of the account
        // runs it (PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md §5). Before
        // this, the relay came first for it too, so a same-computer agent got
        // the relay's 30 min expiry instead of the 24 h hold, and only a
        // sender NOT signed in to the cloud ever reached the hold.
        if !candidate_seen && route_for_absent(state, auth_via, &req, &resp).await == AbsentRoute::HoldHere {
            if let Some(held) = hold_for_absent_target(state, auth_via, &req, &resp, HoldReason::NotRunning).await {
                return held;
            }
        }
        if let Some(body) = try_cloud_relay(&state, &req).await {
            return body;
        }
        // 5. No tier knew the target: hold it for the agent's return, when
        //    the durable jekt conditions hold.
        if !candidate_seen {
            if let Some(held) = hold_for_absent_target(state, auth_via, &req, &resp, HoldReason::NotRunning).await {
                return held;
            }
        }
    }

    // 6. Every tier declined — return the original local error.
    serde_json::to_value(&resp).unwrap_or_default()
}

/// Echo a locally delivered jekt into the sender's own pane, as every
/// successful delivery path does.
fn echo_local_delivery(
    state: &AppState,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
) {
    echo_jekt_to_sender(
        state,
        req.source_agent.as_deref(),
        &req.target_agent,
        &req.message,
        &resp.request_id,
        resp.effective_tier.as_deref(),
        resp.requires_stop,
        EchoTrust {
            delivery_tier: req.delivery_tier.as_deref().unwrap_or("host"),
            // Same `req` this call's own `effective_tier`/`requires_stop`
            // were computed from (via `inject_message`) — not hardcoded, so
            // the echoed marker's TRUST/SIG stays consistent with its own
            // ESCALATE= (reagentx P1 on PR #2623).
            sig_verified: req.sig_verified,
            reagent_verified: req.reagent_verified,
            lan_verified: req.lan_verified,
            channel_verified: req.channel_verified,
        },
        req.priority.as_deref().unwrap_or("normal"),
    );
}

/// Hold a jekt no tier could take, when every condition of
/// `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` §2.1 holds: a full-key,
/// host-tier request (a LAN caller never fills the hold); not a periodic
/// `cron` fire; and a target that is a known agent of this channel —
/// resolved to its row's UID, so a typo or a name registered later by
/// another block never receives the backlog. `None` keeps today's error.
///
/// The same rules hold a jekt the target's spawn gate refused
/// ([`HoldReason::NeedsLogin`]); the replay then waits for sign-in
/// (`jekt_held::still_needs_login`).
pub(crate) async fn hold_for_absent_target(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
    reason: HoldReason,
) -> Option<serde_json::Value> {
    use crate::backend::storage::jekt_held::{HeldJekt, HoldOutcome, HELD_TTL_MS};
    if !hold_applies(auth_via, req, resp) {
        return None;
    }
    let now = agentmux_common::time::now_ms();
    let mut held = HeldJekt {
        request_id: resp.request_id.clone(),
        target_uid: String::new(),
        target_agent: req.target_agent.clone(),
        source_agent: req.source_agent.clone().unwrap_or_default(),
        audit_source_uid: req.audit_source_uid.clone(),
        // What the recipient would receive — sanitized and truncated to the
        // delivery limit — never the unbounded body (Codex P2 on #3632).
        message: crate::backend::reactive::sanitize::sanitize_message(&req.message),
        priority: req.priority.clone().unwrap_or_default(),
        jekt_tier: req
            .jekt_tier
            .as_ref()
            .and_then(|t| serde_json::to_value(t).ok())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        delivery_tier: "host".to_string(),
        sig_verified: req.sig_verified,
        reagent_verified: req.reagent_verified,
        lan_verified: req.lan_verified,
        channel_verified: req.channel_verified,
        is_transcript_request: req.is_transcript_request,
        transcript_request_escalate_forced: req.transcript_request_escalate_forced,
        sent_at_ms: now,
        expires_at_ms: now + HELD_TTL_MS,
        attempts: 0,
        last_error: String::new(),
    };
    let mstore = state.mstore.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let (uid, _slug) = defined_here(&mstore, &held.target_agent)?;
        held.target_uid = uid;
        Some(held)
    })
    .await
    .ok()
    .flatten()?;
    // Registered under another of its names (the handler binds only the
    // display and stable ones): it is running, so deliver by UID now rather
    // than tell the sender it is not. Not for a spawn-gate refusal: that
    // target is registered, and a retry now would only be refused again.
    if reason == HoldReason::NotRunning && state.reactive_handler.has_uid_registration(&outcome.target_uid) {
        let mut by_uid = req.clone();
        by_uid.target_agent = outcome.target_uid.clone();
        let resp = state.reactive_handler.inject_message(by_uid);
        if resp.success {
            // As addressed, so the sender's echo names whom it wrote to
            // (ReAgent P1 on #3632: this path skipped the echo).
            echo_local_delivery(state, req, &resp);
        }
        // A refusal is the target's answer; but if it unregistered between
        // the check and the delivery, fall through and hold it (Codex P2).
        let raced_away = resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found"));
        if !raced_away {
            return Some(serde_json::to_value(&resp).unwrap_or_default());
        }
    }
    let target_uid = outcome.target_uid.clone();
    let mstore = state.mstore.clone();
    let outcome = tokio::task::spawn_blocking(move || mstore.jekt_held_insert(&outcome))
        .await
        .ok()?;
    let target = &req.target_agent;
    match outcome {
        Ok(HoldOutcome::Held) | Ok(HoldOutcome::AlreadyHeld) => {
            crate::backend::agent_resolve::record_uid_fallback("jekt.held");
            let error = match reason {
                HoldReason::NotRunning => format!(
                    "agent {target} is not running — held for delivery on this AgentMux instance for up to 24 h"
                ),
                HoldReason::NeedsLogin => {
                    // The send itself was refused just now: the replay waits
                    // for sign-in rather than retrying at once.
                    super::jekt_held::mark_needs_login(&target_uid);
                    format!(
                        "agent {target} is not signed in — held for delivery on this AgentMux instance for up to 24 h, delivered when it signs in"
                    )
                }
            };
            Some(json!({
                "success": false,
                "held": true,
                "held_reason": reason.as_str(),
                "request_id": resp.request_id,
                "error": error,
            }))
        }
        Ok(HoldOutcome::Full) => Some(json!({
            "success": false,
            "request_id": resp.request_id,
            "error": format!(
                "{} (hold full: too many messages already held for {target})",
                resp.error.clone().unwrap_or_default()
            ),
        })),
        Err(e) => {
            tracing::warn!(error = %e, target = %target, "durable jekt: hold failed");
            None
        }
    }
}

/// The conditions under which this instance may hold a jekt at all (durable
/// jekt spec §2.1): a full-key, host-tier request (a LAN caller never fills
/// the hold); not a periodic `cron` fire; and an id to key the row by.
fn hold_applies(
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
) -> bool {
    auth_via == super::ReactiveAuthVia::FullAuthKey
        && req.delivery_tier.as_deref() == Some("host")
        && req.source_agent.as_deref() != Some("cron")
        && !resp.request_id.is_empty()
}

/// The UID and slug of the agent defined in this channel that `target`
/// names, as the hold resolves it (`hold_for_absent_target`). The slug is
/// the agent's `AGENTMUX_AGENT_ID`, which its relay lease is keyed by.
fn defined_here(mstore: &crate::backend::storage::store::Store, target: &str) -> Option<(String, String)> {
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
fn absent_route_for(answer: &crate::muxbus::wan_lease::Holder) -> AbsentRoute {
    use crate::muxbus::wan_lease::Holder;
    match answer {
        Holder::Free | Holder::Yours | Holder::Unreachable => AbsentRoute::HoldHere,
        Holder::Other(_) | Holder::Unknown | Holder::Unsupported => AbsentRoute::Relay,
    }
}

/// Why a jekt is held (`held_reason` in the inject answer;
/// SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldReason {
    /// A known agent with no process here.
    NotRunning,
    /// The agent is here but its spawn gate refused: not signed in.
    NeedsLogin,
}

impl HoldReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            HoldReason::NotRunning => "not_running",
            HoldReason::NeedsLogin => "needs_login",
        }
    }
}

/// Tier 4. `Some(body)` when the cloud accepted the injection (and the sender
/// echo has been emitted); `None` when tier 4 doesn't apply or didn't take,
/// leaving the caller to return its original error.
///
/// **Queued is not delivered.** The cloud persists the message and wakes
/// subscribed sidecars; the recipient's srv picks it up on its next sync, which
/// may be seconds away or never if that instance is offline. `success: true`
/// here therefore means "handed to the relay" — which is the strongest claim
/// any WAN tier can make, and is what `DELIVERY=wan` on the echoed marker
/// already tells the sender.
async fn try_cloud_relay(state: &AppState, req: &InjectionRequest) -> Option<serde_json::Value> {
    // Never re-relay something that ARRIVED over the cloud. An agent unknown to
    // every instance would otherwise bounce between the relay and each
    // subscriber indefinitely, re-queueing on every hop. `forward_hops` cannot
    // catch this: the cloud hands the message to a fresh inbound request with
    // the count reset, so this tier needs its own guard.
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
    // W3-S (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.1): carry the
    // sender's WAN signature as signed, but only through the carry gate —
    // any unmet condition relays unsigned, exactly as before.
    // The gate's refusal reason rides on the "queued" line below: a
    // signature that silently fails to ride is otherwise invisible, and the
    // receiver can't tell it from a sender that never signed.
    // The directory this send goes into: this relay plus the account of the
    // credential it's sent with. The publisher binds each publication to the
    // same id.
    let directory = Some(credential.account_sub.as_str())
        .filter(|sub| !sub.is_empty())
        .map(|sub| crate::muxbus::wan_publish::directory_id(&crate::muxbus::relay::rest_base_url(), sub));
    let (carried, unsigned_reason) = match crate::muxbus::relay::wan_carry_gate(
        req,
        state.mstore.wan_identity().as_deref(),
        directory.as_deref(),
        &crate::backend::reactive::registry::local_channel_id(),
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
        carried.as_ref(),
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
            echo_jekt_to_sender(
                state,
                Some(source),
                &req.target_agent,
                &req.message,
                &request_id,
                None, // no receiver-side escalation to mirror: nobody has
                None, // seen the message yet, so neither field is known
                EchoTrust {
                    delivery_tier: "wan",
                    // Signing on this path is issue #2586's unbuilt WAN half —
                    // an outbound relay carries no proof of sender identity, and
                    // claiming otherwise here would put a TRUST value on the
                    // echoed marker that the receiving side will not agree with.
                    sig_verified: None,
                    reagent_verified: None,
                    lan_verified: None,
                    channel_verified: None,
                },
                priority,
            );
            let body = crate::backend::reactive::types::InjectionResponse {
                deferred: None,
                success: true,
                request_id,
                block_id: None,
                error: None,
                timestamp: now_unix_secs().max(0) as u64,
                // No receiver has seen the message yet, so there is no applied
                // tier or STOP decision to report — deliberately left unset
                // rather than guessed at.
                effective_tier: None,
                requires_stop: None,
                channel_verified: None,
            };
            let mut body = serde_json::to_value(&body).unwrap_or_default();
            // Passed through for `SendMessage`, which warns of a likely typo
            // on `false` (PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md R-3).
            if let (Some(in_account), Some(obj)) = (target_in_account, body.as_object_mut()) {
                obj.insert("target_in_account".to_string(), serde_json::Value::Bool(in_account));
            }
            Some(body)
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

/// Agent NAMES only, for LAN peers — deliberately NOT
/// [`handle_reactive_agents`], which serializes full `AgentRegistration`
/// records (`block_id`, `tab_id`, `registered_at`, `last_seen`,
/// `registration_nonce`). Those are internal delivery/routing details and
/// stay behind full auth.
///
/// This route is reachable with the broadcast `lan_key`
/// (`lan_or_full_auth_middleware`), so treat its output as public to anyone
/// on the local network: the LAN trust work deliberately shrank what a captured `lan_key` is worth, and
/// this widens it by exactly one thing — enumerating agent names. That was
/// an explicit, repo-owner-confirmed decision (2026-09-08): a `lan_key`
/// holder could already confirm any *specific* name via
/// `/agentmux/reactive/agent?id=…`, so this makes enumeration cheap rather
/// than newly possible, and it's what lets a peer show which agents live on
/// another host instead of the empty `agents: []` every LAN peer reported
/// before. **Do not extend this response with anything beyond names, kinds
/// and status** without revisiting those decisions.
///
/// Two widenings since, both activity or placement metadata, never content.
/// `agent_kinds` (`{name: "host" | "container"}`, an agent whose block can't
/// be read left out): where an agent runs, by the owner's decision of
/// 2026-10-06 (agentmux-mobile's SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06
/// §5.2, §11). `agent_status` (`{name: {state, since_ms}}`, an agent with no
/// known state left out) and `now_ms`: whether an agent is busy and since
/// when, nothing it says or does, by the owner's decision D1 of 2026-10-07
/// (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §3.4,
/// §11). The same fields are on the fleet feed this route is the fallback for.
pub(super) async fn handle_reactive_agent_names(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agents = state.reactive_handler.list_agents();
    let kinds = crate::backend::fleet_source::kinds_by_name(
        agents.iter().map(|a| (a.agent_id.as_str(), a.block_id.as_str())),
        |block_id| crate::backend::operator_config_seed::agent_kind_of_block(&state.mstore, block_id),
    );
    let status: std::collections::BTreeMap<&str, _> = agents
        .iter()
        .filter_map(|a| {
            crate::backend::agent_state::agent_status_of_block(&state.mstore, &a.block_id)
                .map(|s| (a.agent_id.as_str(), s))
        })
        .collect();
    let status = serde_json::to_value(status).unwrap_or_else(|_| json!({}));
    let names: Vec<String> = agents.iter().map(|a| a.agent_id.clone()).collect();
    Json(json!({
        "agents": names,
        "agent_kinds": kinds,
        "now_ms": agentmux_common::time::now_ms_u64(),
        "agent_status": status,
    }))
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
    match state.reactive_handler.get_agent(id) {
        Some(agent) => {
            // Merged in, not part of AgentRegistration's own serialization —
            // this is the LAN pubkey-lookup half of
            // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2. The
            // public half of an Ed25519 keypair is not secret; handing it to
            // whichever peer already holds a valid lan_key (this whole route
            // is gated by lan_or_full_auth_middleware) costs nothing and
            // lets that peer verify this agent's future outgoing LAN
            // signatures. `None` when the agent has no LAN key minted yet
            // (never sent a LAN jekt since this shipped) — omitted from the
            // JSON entirely rather than rendered `null`, so existing callers
            // of this endpoint that don't know about this field see no
            // change in shape.
            let mut value = serde_json::to_value(&agent).unwrap_or_default();
            if let Ok(Some(pubkey)) = state.mstore.agent_lan_public_key_load(id) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("lan_public_key".to_string(), json!(pubkey));
                }
            }
            // Identity M4d-5: the registration's UID key, separately; the
            // name-keyed key above stays the query id's (spec §6.5.10).
            if let Some(Ok(Some(pubkey))) = agent.uid.as_deref().map(|uid| state.mstore.agent_uid_lan_public_key_load(uid)) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("uid_public_key".to_string(), json!(pubkey));
                }
            }
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
    Json(req): Json<RegisterRequest>,
) -> Response {
    tracing::info!(
        agent_id = %req.agent_id,
        block_id = %req.block_id,
        "reactive register request"
    );
    // Identity M2 (spec §4.4.1 row 3): this is a presence signal keyed by
    // block. The name the frontend sends becomes the display binding, but
    // identity is NOT taken from it — it may be a stale meta value or an
    // id the agent process itself emitted over OSC. The UID comes from the
    // row the server created for this block, read on `spawn_blocking` like
    // every other store read in an async handler (incident #1782).
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
    // One live instance per agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24
    // Phase 1, I9): a pane whose agent runs in another AgentMux instance on
    // this host must not register it — the cross-instance registries written
    // below are how other senders route this agent's jekts, and delivery
    // prefers the freshest entry, so registering here would pull the live
    // instance's messages to a pane that is refused its turns. The frontend
    // only logs a failed registration; it does not retry.
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
            // Refresh the block's OWN captured identity too (reagentx P1 on
            // #2697): this HTTP path can (re-)register an existing block_id
            // under a different agent_id (a rename, or a reconfigured
            // cmd:env) — without this, the controller's `agent_id()` stays
            // stale at whatever it was captured as at spawn time, and
            // `inject_message_inner`'s recipient-identity check (#2695)
            // would then falsely reject the agent's own, correctly-addressed
            // messages as a mismatch. `get_controller` returning `None`
            // (block not tracked, or a controller type that doesn't
            // implement `agent_id()`/`set_agent_id()`) is a harmless no-op.
            if let Some(ctrl) = blockcontroller::get_controller(&req.block_id) {
                ctrl.set_agent_id(Some(req.agent_id.clone()));
            }

            // Also write to cross-instance file registry so other AgentMux
            // instances can forward inject requests to this one.
            let data_dir = base::get_mux_data_dir();
            agent_registry::write(&data_dir, &req.agent_id, &state.local_web_url, &req.block_id);

            // And to the host-global shared registry (Tier 2b) so instances
            // running in OTHER channels on this host can reach this agent
            // too — closes issue #1916 (Tier 2 previously only ever reached
            // the caller's own channel).
            agent_registry::write_shared_from_env(
                &req.agent_id,
                &state.local_web_url,
                &req.block_id,
            );

            // Auto-watch this agent's Claude Code config dir for subagent JSONL files.
            // Pass block_id so subagent events are stamped with the owning pane,
            // letting the frontend route ⚡ panels to that pane only. See
            // `resolve_claude_config_dir`'s doc comment for why this must read
            // the block's own `cmd:env`, not just guess a path convention.
            let block = state.mstore.get::<crate::backend::obj::Block>(&req.block_id).ok().flatten();
            let empty_meta = crate::backend::obj::MetaMapType::new();
            // Identity-bound agents' real CLAUDE_CONFIG_DIR is never the
            // stale `cmd:env` snapshot below — see
            // `resolve_claude_config_dir`'s doc comment and
            // SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md.
            let bound_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                &state.mstore,
                &state.id_store,
                &state.identity_store,
                &req.block_id,
            );
            let config_dir = subagent_watcher::resolve_claude_config_dir(
                block.as_ref().map(|b| &b.meta).unwrap_or(&empty_meta),
                &req.agent_id,
                bound_dir,
            );
            if let Some(config_dir) = config_dir {
                state.subagent_watcher.watch_agent(&req.agent_id, &req.block_id, config_dir.clone());

                // Codex P1 on PR #2980: the resolution above and the
                // `watch_agent` call are not atomic with an
                // `agent_identity_link` write landing on a different
                // thread — a bind can commit in the narrow window between
                // computing `config_dir` and `watch_agent` actually
                // installing the watch. If that happens, the bind's own
                // `recheck_all_watched_agents` call can run and find
                // nothing yet in `watched_agents` to correct, and no LATER
                // bind event is guaranteed to retrigger it. Immediately
                // re-resolving fresh right here — after the install has
                // definitely committed — closes that window: if the bind
                // already landed, this catches it on the spot instead of
                // depending on an external trigger that might never come
                // again. Safe/cheap to do unconditionally: `recheck_config_dir`
                // is a same-ref no-op when nothing actually changed. Scoped
                // to THIS call site specifically (not inside `watch_agent`
                // itself) because this is the one place `config_dir` is
                // actually derived from identity/binding resolution in the
                // first place — see `watch_agent`'s own doc comment for why
                // baking this into the shared primitive broke its other
                // callers.
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

                // If this block already has a persisted session id, it's
                // resuming a prior conversation (not starting fresh) —
                // backfill just THAT session's own subagents, so a
                // reopened pane shows what it already had without
                // flooding in every OTHER session this agent identity has
                // ever run. A brand-new session has nothing to backfill;
                // watch_agent's live watcher picks up subagents as the
                // Task tool spawns them.
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

            // Notify cloud subscriber so it can subscribe for cloud-push delivery
            if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                sub.add_agent(&req.agent_id);
            }

            // Notify the Swarm view so it calls AgentTrackedBlocksCommand and
            // shows this pane. We use a dedicated event name so useProcessCount
            // (which subscribes to agent:process-added / agent:process-exited)
            // doesn't treat this as a phantom OS process and show a spurious ⚙ N
            // badge or trigger the kill-tree modal on pane close.
            state.broker.publish(crate::backend::mps::MuxEvent {
                event: "agent:reactive-registered".to_string(),
                scopes: vec![format!("block:{}", req.block_id)],
                sender: String::new(),
                persist: 0,
                data: Some(json!({ "block_id": req.block_id })),
            });

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

/// Mint-or-reuse `agent_id`'s host-tier jekt signing key and return it,
/// base64-encoded — the same value `agent_config::inject_jekt_signing_keys_into_mcp_json`
/// writes into a real agent's `.mcp.json` at spawn time, exposed directly
/// here instead of only reachable through `agent.open`'s WebSocket
/// (`WshRpcEngine`) path. That path also spawns a real provider session,
/// which makes it unusable for anything that just wants to exercise
/// host-tier jekt signing/verification (tests, external harnesses) without
/// the cost and side effects of a live LLM session.
///
/// reagentx P0 on this PR's first pass: gating this behind the same
/// `X-AuthKey` as `register`/`unregister` is NOT the same trust level as
/// those routes — `X-AuthKey`/`AGENTMUX_AUTH_KEY` is injected into every
/// spawned agent's own subprocess env for its bashwrap/MCP tool calls
/// (`agent_handlers/input.rs`), so any agent already holds it. Returning
/// another agent's raw signing key to any caller with that shared key is
/// secret-key exfiltration enabling cryptographic impersonation — it
/// directly breaks the invariant `agent_jekt_keys.rs` documents ("never
/// returned over any RPC ... only the agent it claims to be from ever held
/// the key") and the entire premise `TRUST=host-verified` depends on.
///
/// There is no way to additionally verify "the HTTP caller genuinely IS
/// `agent_id`" from this request alone — that's the exact problem host-tier
/// signing exists to solve, so this endpoint can't lean on it without being
/// circular. Instead: **disabled unless `AGENTMUX_ENABLE_TEST_ENDPOINTS=1`
/// was set in the srv process's own environment before it started** — set
/// by whoever launches the instance, not settable by a running agent's tool
/// calls (unlike `X-AuthKey`, which every spawned agent already holds
/// regardless of what's set here). Off by default, so no real user's real
/// instance with real agents ever exposes it; on only for a deliberately
/// isolated test/verification instance where every "agent" is synthetic.
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
    agent_id: String,
    /// Identity M2 (spec §4.4.4 Q7): optional, but both frontend callers
    /// send it. With it, teardown is by block. Without it, a name held by
    /// several live blocks is refused with HTTP 409 and the candidates —
    /// never a `success: true` that tore down nothing (§10 constraint 3).
    #[serde(default)]
    block_id: String,
}

pub(super) async fn handle_reactive_unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Response {
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

    // Name-keyed side state is torn down for EVERY name the block held
    // (display and stable). The name the caller asked by is used only when
    // no block was found at all (the pre-M2 behaviour for an unknown name):
    // when a block WAS torn down, its own bindings are authoritative — the
    // caller's name may be a stale display value that another live block
    // now legitimately holds, and removing that block's file entry and cloud
    // subscription would knock it off cross-instance routing.
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
        // Drop the subagent filesystem watcher (handle + channel + task) — the
        // symmetric teardown for the watch_agent() call in the register handler.
        // Passes block_id so a shared-agent-id watcher with another still-open
        // dependent block survives this one's teardown.
        state.subagent_watcher.unwatch_agent(name, block_id.as_deref());
        // Notify cloud subscriber so it stops subscribing for this agent
        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
            sub.remove_agent(name);
        }
    }

    // Symmetric refresh: tell the Swarm view this pane is gone.
    if let Some(bid) = block_id {
        state.broker.publish(crate::backend::mps::MuxEvent {
            event: "agent:reactive-unregistered".to_string(),
            scopes: vec![format!("block:{}", bid)],
            sender: String::new(),
            persist: 0,
            data: Some(json!({ "block_id": bid })),
        });
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

/// Server-side ceiling on `max_lines` — protects a Supervisor's transcript
/// pull (and this route in general) from an unbounded read of a huge
/// session file. Callers wanting more must paginate some other way; this
/// route is a "recent tail" primitive, not a full-history export.
const TRANSCRIPT_MAX_LINES_CAP: usize = 500;

#[derive(serde::Deserialize)]
pub(super) struct TranscriptQuery {
    agent: String,
    #[serde(default = "default_transcript_max_lines")]
    max_lines: usize,
    /// Set ONLY by [`handle_reactive_transcript_cross_channel`] on the
    /// single forwarded request it ever sends — caps cross-channel
    /// resolution at exactly one hop. Without this, a stale-but-PID-alive
    /// shared-registry entry (pointing back at this same instance, or at a
    /// second instance whose own entry for the same agent points back
    /// here) would forward indefinitely — the exact failure mode
    /// `handle_reactive_inject`'s `MAX_FORWARD_HOPS`/`forward_hops` guard
    /// exists to prevent for jekt delivery (reagent P1, codex P1 on
    /// PR #2715). A bare bool is sufficient here (unlike inject's integer
    /// hop counter) because this route is architecturally single-hop by
    /// design — the owning instance found via `lookup_all_shared` always
    /// has the agent on ITS OWN host tier, never a further cross-channel
    /// hop of its own — so "already forwarded once" and "hop limit
    /// reached" are the same condition.
    #[serde(default)]
    forwarded: bool,
}
fn default_transcript_max_lines() -> usize {
    100
}

/// `GET /agentmux/reactive/transcript?agent=<name>&max_lines=<n>` — read the
/// tail of a registered agent's session output, for a Warden Supervisor
/// watcher agent to inspect on its own poll interval (v1 is pull/poll, not
/// push — see
/// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md).
///
/// As of `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
/// Phase A, a miss on this instance's own host-tier registry falls back to
/// the host-global cross-channel shared registry and forwards a single-hop
/// HTTP GET to the owning channel's own instance — same auth
/// (`entry.auth_key` as `X-AuthKey`) and loopback-only pattern
/// `handle_reactive_inject`'s Tier 2b already uses (see that handler for
/// the security rationale). Response carries `"tier"` so callers can tell
/// which tier answered.
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

    let reg = match state.reactive_handler.lookup_by_name(&params.agent) {
        crate::backend::reactive::handler::LookupOutcome::One(reg) => reg,
        // Identity M2 (spec §4.4.3): a name several live blocks hold is a
        // purely LOCAL collision — refuse it with the candidates, never
        // fall through to cross-channel forwarding as if it were unknown.
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
                // Already one hop in — see TranscriptQuery::forwarded's doc
                // comment. The owning instance's own host-tier lookup just
                // missed too, so this agent genuinely isn't registered
                // anywhere reachable; 404, do not attempt a second forward.
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

    let (raw_bytes, _total_line_count) = match crate::backend::session_archive::read_session_output(
        &state.mstore,
        &state.filestore,
        &block_id,
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("read_session_output: {e}")})),
            )
                .into_response();
        }
    };

    let text = String::from_utf8_lossy(&raw_bytes);
    let all_lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let requested = params.max_lines.min(TRANSCRIPT_MAX_LINES_CAP).max(1);
    let truncated = all_lines.len() > requested;
    let lines: Vec<String> = all_lines
        .iter()
        .rev()
        .take(requested)
        .rev()
        .map(|l| l.to_string())
        .collect();

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

/// Fallback path for [`handle_reactive_transcript`] when the target agent
/// isn't on this instance's own host-tier registry — checks the host-global
/// cross-channel shared registry and, on a hit, forwards to the owning
/// channel's own instance. A miss here (not found on this channel OR any
/// other channel on this host) 404s exactly as the host-only lookup always
/// did — this does not reach LAN or WAN (Phase A scope; see spec Phase B/C).
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
    // Skip any entry pointing back at THIS instance before picking one —
    // a stale-but-PID-alive self-registration (the registration race /
    // incomplete-cleanup case codex flagged on PR #2715) would otherwise
    // forward a request to ourselves, which re-enters this exact function
    // and repeats. Filtering here (not just checking the single freshest
    // pick) also handles a self-entry merely being the FRESHEST of several
    // candidates — same defense-in-depth `handle_reactive_inject`'s Tier 2b
    // applies. Combined with `TranscriptQuery::forwarded` above (which
    // still caps this at one hop even in an exotic multi-instance cycle
    // this filter alone wouldn't catch — e.g. instance A's entry points to
    // B and B's own entry for the same agent points back to A).
    let Some(entry) = crate::backend::reactive::registry::lookup_all_shared(&shared_dir, &params.agent)
        .into_iter()
        .find(|e| !is_self_registration(&e.local_url, &state.local_web_url))
    else {
        return not_found();
    };

    let query: Vec<(&str, String)> = vec![
        ("agent", params.agent.clone()),
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

#[derive(serde::Deserialize)]
pub(super) struct SupervisorDecisionRequest {
    target_agent: String,
    /// "nudge" | "decline".
    action: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    /// The calling Supervisor agent's own identity — same shape as
    /// `InjectRequest::source_agent` (`SendMessage`/`Loop`). `None` for
    /// callers that omit it (e.g. cron-driven).
    #[serde(default)]
    source_agent: Option<String>,
}

/// Default/cap for how many of an agent's sessions a single search may open.
///
/// Full-parsing every session of a long-lived agent is not viable: the session
/// that motivated `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` was 14.7 MB /
/// 6682 records and was one of several. Candidates are taken newest-first, so
/// the default answers the common "what did I do recently" question without
/// reading years of history.
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
    /// The caller's name, as the MCP sends it. **Never whose history is
    /// searched**: that is the row of the caller's per-agent token
    /// (`X-Agent-Token`, identity M4c-2c,
    /// SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9), and a
    /// request without one is refused. A name is self-declared — every local
    /// agent shares the instance `auth_key` — so resolving the owner from it
    /// could only guess, and a guess answered "no history" with confidence.
    /// Kept for the actor counters (M4a-2).
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

/// `GET /agentmux/reactive/history/search` — search an agent's own past
/// conversations (`SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md`).
///
/// Distinct from `/reactive/transcript`, which returns the tail of the LIVE
/// session only. This reads sessions already persisted on disk, including
/// sessions that ended — which is the point: an agent's memory of what it did
/// is bounded by its context window while its actual actions are not, so after
/// a compaction or session reset it will answer questions about its own past
/// confidently and wrongly, with nothing marking the boundary.
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
    // An empty query is only meaningful alongside a `tool` filter ("every call
    // to X"); on its own it would match everything and return a truncated
    // firehose, which reads as a real answer.
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

    // Identity M4c-2c (spec §6.5.9): the owner is the caller's own row, from
    // its per-agent token — never a name. Every agent AgentMux spawns carries
    // one; a request without it can't say whose history it is, and resolving
    // a self-declared name instead could only guess (on 2026-09-24 a guess
    // answered "no history" for an agent whose sessions were on disk).
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

    // Parsing sessions is blocking filesystem work; keep it off the async
    // runtime's worker threads, same posture as other disk-heavy handlers.
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

/// `POST /agentmux/reactive/supervisor-decision` — a Warden Supervisor
/// watcher agent's decision about a target agent it just polled (see
/// `GetAgentTranscript`). `action: "nudge"` delivers a fixed continuation
/// message (not caller-supplied text — see `SupervisorAction::Nudge`'s
/// doc) to `target_agent` through the same path `SendMessage`/`Loop` use
/// and audits it as a Supervisor-originated entry; `action: "decline"`
/// sends nothing and just audits the decision. A nudge that would exceed
/// the consecutive-nudge ceiling is refused with HTTP 429 — the calling
/// agent should treat that as a signal to stop and escalate to a human
/// instead of retrying.
pub(super) async fn handle_reactive_supervisor_decision(
    State(state): State<AppState>,
    caller: Option<Extension<super::caller::Caller>>,
    Json(req): Json<SupervisorDecisionRequest>,
) -> Response {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::SupervisorDecision,
        req.source_agent.as_deref(),
    );
    if req.target_agent.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing target_agent"})),
        )
            .into_response();
    }

    let action = match req.action.as_str() {
        "nudge" => SupervisorAction::Nudge,
        "decline" => SupervisorAction::Decline,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("unknown action: {other} (expected \"nudge\" or \"decline\")")})),
            )
                .into_response();
        }
    };

    // Entitlement gate (reagentx P1 on PR #2557): a Nudge must not deliver
    // unless the target has actually opted in via `auto_continue_enabled`.
    // `Handler` (backend::reactive) has no `Store` access by design — this
    // check belongs at the HTTP boundary where `state.mstore` is available,
    // not inside `record_supervisor_decision`. Decline never delivers
    // anything, so it isn't gated.
    //
    // Match on `d.slug`, NOT `d.name` (reagentx P0, round 3 — every
    // delivery path keys registration off `AGENTMUX_AGENT_ID`, which
    // `agent_open.rs` sets to the agent's stable `slug`, not its
    // renameable display `name`. Matching on `name` here let a renamed
    // agent's own opt-in go unrecognized, and — worse — let one agent's
    // slug collide with an unrelated agent's current display name,
    // authorizing a nudge off the wrong definition's flag. Same
    // name/slug cross-namespace hazard `storage/agents/mod.rs`'s
    // `instance_get_by_name_and_by_slug_never_cross_the_others_namespace`
    // regression-tests for the read path.)
    if matches!(action, SupervisorAction::Nudge) {
        let opted_in = state
            .mstore
            .agent_def_list()
            .ok()
            .and_then(|defs| {
                defs.into_iter()
                    .find(|d| d.slug.eq_ignore_ascii_case(&req.target_agent))
            })
            .map(|d| d.auto_continue_enabled != 0)
            .unwrap_or(false);
        if !opted_in {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": format!(
                        "target agent '{}' has not opted in to auto_continue_enabled",
                        req.target_agent
                    )
                })),
            )
                .into_response();
        }
    }

    let reason = req.reason.unwrap_or_default();
    let request_id = req
        .request_id
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    match state.reactive_handler.record_supervisor_decision(
        &req.target_agent,
        action,
        &reason,
        &request_id,
        req.source_agent.as_deref(),
        &super::caller::attributed_uid(caller.as_deref()),
    ) {
        Ok(resp) => Json(serde_json::to_value(&resp).unwrap_or_default()).into_response(),
        Err(e) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

// ---- WS RPC: reactive.registrations (issue #2696, Stash UI indicator) ----

/// Whether a host-global shared-registry entry is THIS instance's own
/// registration (as opposed to a genuinely different instance/channel on
/// the same host) — every agent unconditionally writes itself into that
/// same registry, so a "remote"/"elsewhere" listing must exclude its own
/// entry or it fires on every healthy agent (reagentx P1 on #2698). Same
/// comparison this file already uses for Tier 2a/2b forwarding.
pub(super) fn is_self_registration(entry_local_url: &str, this_instances_local_url: &str) -> bool {
    entry_local_url == this_instances_local_url
}

/// One cross-instance/channel entry from the host-global shared registry
/// (`backend::reactive::registry::AgentEntry`), narrowed to what the
/// frontend actually needs to render a "registered elsewhere too" badge —
/// deliberately excludes `local_url`/`auth_key`, which are internal
/// forwarding plumbing, not UI-relevant.
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

/// Summary of the most recent `identity-mismatch` audit entry for this
/// agent_id, if any (see #2695's `Handler::inject_message_inner` check) —
/// narrowed from `AuditLogEntry` to what the Stash badge needs.
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
    engine.register_typed(
        "reactive.registrations",
        move |params: ReactiveRegistrationsParams, _ctx| {
            let state = state.clone();
            async move {

                let local = state.reactive_handler.get_agent(&params.agent_id);

                // Every agent (including this instance's own) unconditionally
                // writes itself into the same host-global shared registry
                // (write_shared_from_env, called from both the PTY-shell and
                // persistent auto-register paths and from handle_reactive_
                // register above) — so without filtering, `remote` always
                // includes THIS instance's own entry alongside any genuinely
                // other instance, and the "registered elsewhere too" badge
                // would fire on every healthy agent (reagentx P1). Same
                // self-filter this file already uses for Tier 2a/2b forwarding
                // (`entry.local_url == state.local_web_url` in
                // `handle_reactive_inject`).
                let remote = crate::registry::resolve_shared_reactive_dir()
                    .map(|shared_dir| {
                        agent_registry::lookup_all_shared(&shared_dir, &params.agent_id)
                            .into_iter()
                            .filter(|e| !is_self_registration(&e.local_url, &state.local_web_url))
                            // A crashed sibling instance's entry otherwise
                            // lingers until the next startup-only
                            // cleanup_stale_shared sweep (bootstrap/network.rs) —
                            // up to hours later — showing a false "Also
                            // registered elsewhere" badge in the meantime
                            // (reagentx P2). PID-liveness is authoritative
                            // here (same-host by construction, per
                            // pid_alive's own doc comment), so check it live
                            // instead of waiting for that sweep.
                            .filter(|e| agent_registry::pid_alive(e.pid))
                            .map(|e| RemoteRegistrationEntry {
                                channel: e.channel,
                                pid: e.pid,
                                updated_at: e.updated_at,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let target_lower = params.agent_id.to_lowercase();
                let recent_mismatch = state
                    .reactive_handler
                    .get_audit_log(100)
                    .into_iter()
                    .find(|e| {
                        e.outcome.as_deref() == Some("identity-mismatch")
                            && e.target_agent.to_lowercase() == target_lower
                    })
                    .map(|e| MismatchAuditSummary {
                        timestamp: e.timestamp,
                        block_id: e.block_id,
                        error_message: e.error_message,
                    });

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

/// PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md §5-§6: the relay's lease
/// answer as a route. Holding here is the default; only "another install
/// runs it" or an answer the relay cannot give plainly sends it to the relay.
#[cfg(test)]
mod absent_route_tests {
    use super::*;
    use crate::muxbus::wan_lease::Holder;

    #[test]
    fn the_lease_answer_picks_the_route() {
        assert_eq!(absent_route_for(&Holder::Free), AbsentRoute::HoldHere);
        assert_eq!(absent_route_for(&Holder::Yours), AbsentRoute::HoldHere);
        // No relay to send through: keep it here.
        assert_eq!(absent_route_for(&Holder::Unreachable), AbsentRoute::HoldHere);
        assert_eq!(absent_route_for(&Holder::Other("computer area54".into())), AbsentRoute::Relay);
        // An older relay, or one that won't say: the order before this plan.
        assert_eq!(absent_route_for(&Holder::Unsupported), AbsentRoute::Relay);
        assert_eq!(absent_route_for(&Holder::Unknown), AbsentRoute::Relay);
    }
}

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
