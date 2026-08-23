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
/// fields) to the sender's `output` blockfile — live WPS append (renders
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
pub(super) fn echo_jekt_to_sender(
    state: &AppState,
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
    msgid: &str,
    effective_tier: Option<&str>,
    requires_stop: Option<bool>,
    delivery_tier: &str,
    sig_verified: Option<bool>,
    reagent_verified: Option<bool>,
    lan_verified: Option<bool>,
    priority: &str,
) {
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
        // Defaults to `true` (STOP) when the caller couldn't tell us —
        // matches `effective_tier` defaulting to the more-cautious "coord"
        // rather than assuming "info" above; never silently downgrades a
        // sensitive echo to tag-only just because this hop lost the signal.
        requires_stop.unwrap_or(true),
        msgid,
        priority,
    );
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": wrapped }
    });
    let data = format!("{line}\n");
    let global_zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(
        &Some(state.wstore.clone()),
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

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

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
    let Ok(Some(key)) = state.wstore.agent_jekt_key_load(&claimed) else {
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
/// (the WS delivery path's own constant of the same value) and the
/// github-consumer Lambda's own REVIEW_NOTIFICATION_TTL_SECONDS delivery
/// window in the agentmux-cloud repo.
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
/// agentmux-cloud's Secrets Manager), so tests can't mint a fresh signature
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
        && agentmux_common::jekt_sign::verify_reagent_jekt(
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
    let Ok(pinned_key_b64) = state.wstore.lan_peer_pubkey_pin_get_or_set(&claimed, &observed_key_b64) else {
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
fn resolve_delivery_tier(auth_via: super::ReactiveAuthVia, claimed: Option<&str>) -> String {
    if auth_via == super::ReactiveAuthVia::LanKey {
        "lan".to_string()
    } else {
        claimed.unwrap_or("host").to_string()
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
/// Takes `wstore: &Arc<Store>` directly (not `&AppState`) so
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
pub(crate) fn resolve_transcript_request_tier_fields(wstore: &std::sync::Arc<crate::backend::storage::store::Store>, req: &mut InjectionRequest) {
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
    let visibility = wstore
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
            let granted = wstore
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
mod transcript_request_tier_resolution_tests {
    use super::*;
    use crate::backend::storage::store::AgentDefinition;
    use crate::server::tests::test_state;

    fn insert_agent_def(state: &AppState, slug: &str, conversation_visibility: &str) {
        let mut def = AgentDefinition {
            id: uuid::Uuid::new_v4().to_string(),
            slug: slug.to_string(),
            name: slug.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            auto_continue_enabled: 0,
            memory_id: String::new(),
            conversation_visibility: conversation_visibility.to_string(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();
    }

    fn transcript_request_message() -> String {
        r#"{"type":"transcript_request","request_id":"r1","max_lines":50}"#.to_string()
    }

    fn base_req(target: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: target.to_string(),
            message: transcript_request_message(),
            source_agent: Some("requester".to_string()),
            delivery_tier: Some("lan".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn ordinary_message_never_sets_either_field() {
        let state = test_state();
        let mut req = InjectionRequest {
            target_agent: "agent1".to_string(),
            message: "just chatting".to_string(),
            ..Default::default()
        };
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(!req.is_transcript_request);
        assert!(!req.transcript_request_escalate_forced);
    }

    #[tokio::test]
    async fn private_visibility_does_not_force_escalate() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "private");
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(req.is_transcript_request);
        assert!(!req.transcript_request_escalate_forced);
    }

    #[tokio::test]
    async fn ask_visibility_forces_escalate() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "ask");
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(req.is_transcript_request);
        assert!(req.transcript_request_escalate_forced);
    }

    #[tokio::test]
    async fn trusted_peers_without_a_grant_forces_escalate() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "trusted_peers");
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(
            req.transcript_request_escalate_forced,
            "an un-granted requester must still force escalation under trusted_peers mode"
        );
    }

    #[tokio::test]
    async fn trusted_peers_with_a_matching_grant_does_not_force_escalate() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "trusted_peers");
        state.wstore.conversation_trust_grant_add("agent1", "requester", "lan").unwrap();
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(
            !req.transcript_request_escalate_forced,
            "an allow-listed requester on the SAME tier must not force escalation"
        );
    }

    #[tokio::test]
    async fn trusted_peers_grant_on_a_different_tier_still_forces_escalate() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "trusted_peers");
        // Granted for WAN, but this request arrives on LAN (base_req's default).
        state.wstore.conversation_trust_grant_add("agent1", "requester", "wan").unwrap();
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(
            req.transcript_request_escalate_forced,
            "a grant for one tier's identity guarantee must never be assumed to cover a different tier"
        );
    }

    // Phase C (WAN): this function is tier-generic — these two tests pin
    // that WAN delivery gets the identical treatment LAN already has,
    // since `sync_agent_reactive` (the WAN delivery path,
    // `muxbus/cloud_subscriber.rs`) calls this exact function directly.
    #[tokio::test]
    async fn wan_tier_transcript_request_forces_sensitive_same_as_lan() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "private");
        let mut req = InjectionRequest {
            target_agent: "agent1".to_string(),
            message: transcript_request_message(),
            source_agent: Some("requester".to_string()),
            delivery_tier: Some("wan".to_string()),
            ..Default::default()
        };
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(req.is_transcript_request, "rule 1 must fire on WAN exactly like every other tier");
    }

    #[tokio::test]
    async fn wan_tier_trusted_peers_grant_on_the_matching_tier_relaxes_escalation() {
        let state = test_state();
        insert_agent_def(&state, "agent1", "trusted_peers");
        state.wstore.conversation_trust_grant_add("agent1", "requester", "wan").unwrap();
        let mut req = InjectionRequest {
            target_agent: "agent1".to_string(),
            message: transcript_request_message(),
            source_agent: Some("requester".to_string()),
            delivery_tier: Some("wan".to_string()),
            ..Default::default()
        };
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(
            !req.transcript_request_escalate_forced,
            "a WAN grant checked against an actual WAN request must relax escalation, same as LAN's matching-tier case"
        );
    }

    #[tokio::test]
    async fn matches_on_slug_not_display_name() {
        let state = test_state();
        // Insert with a slug matching the target, but a DIFFERENT display name —
        // the lookup must key off slug (the stable AGENTMUX_AGENT_ID-derived
        // identifier), same cross-namespace hazard the Supervisor-nudge
        // opt-in check just above this function already guards against.
        let state_clone = &state;
        {
            let mut def = AgentDefinition {
                id: uuid::Uuid::new_v4().to_string(),
                slug: "agent1".to_string(),
                name: "Totally Different Display Name".to_string(),
                icon: String::new(),
                provider: "claude".to_string(),
                description: String::new(),
                working_directory: String::new(),
                shell: String::new(),
                provider_flags: String::new(),
                auto_start: 0,
                restart_on_crash: 0,
                idle_timeout_minutes: 0,
                created_at: 0,
                agent_type: "standalone".to_string(),
                environment: String::new(),
                agent_bus_id: String::new(),
                is_seeded: 0,
                accounts: String::new(),
                parent_id: String::new(),
                branch_label: String::new(),
                updated_at: 0,
                user_hidden: 0,
                container_image: String::new(),
                container_volumes: "[]".to_string(),
                container_name: String::new(),
                use_ambient_login: 0,
                model_vendor_base_url: String::new(),
                auto_continue_enabled: 0,
                memory_id: String::new(),
                conversation_visibility: "ask".to_string(),
            };
            state_clone.wstore.agent_def_insert(&mut def).unwrap();
        }
        let mut req = base_req("agent1");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(req.transcript_request_escalate_forced, "lookup must match by slug \"agent1\", not the unrelated display name");
    }

    #[tokio::test]
    async fn unknown_target_agent_fails_closed_to_private_defaults() {
        let state = test_state();
        // No AgentDefinition inserted at all for this target.
        let mut req = base_req("no-such-agent");
        resolve_transcript_request_tier_fields(&state.wstore, &mut req);
        assert!(req.is_transcript_request, "rule 1 (forced sensitive) applies regardless of whether the target is known");
        assert!(!req.transcript_request_escalate_forced, "an unknown agent defaults to the safe 'private' behavior for the escalate-forcing question");
    }
}

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Extension(auth_via): Extension<super::ReactiveAuthVia>,
    Json(mut req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );

    req.delivery_tier = Some(resolve_delivery_tier(auth_via, req.delivery_tier.as_deref()));

    verify_jekt_signature(&state, &mut req);
    verify_reagent_signature(&mut req, now_unix_secs());
    verify_lan_signature(&state, &mut req).await;
    resolve_transcript_request_tier_fields(&state.wstore, &mut req);

    // 1. Try local ReactiveHandler first (fast path — same instance).
    let resp = state.reactive_handler.inject_message(req.clone());
    if resp.success {
        echo_jekt_to_sender(
            &state,
            req.source_agent.as_deref(),
            &req.target_agent,
            &req.message,
            &resp.request_id,
            resp.effective_tier.as_deref(),
            resp.requires_stop,
            req.delivery_tier.as_deref().unwrap_or("host"),
            req.sig_verified,
            // Same `req` this call's own `effective_tier`/`requires_stop`
            // were computed from (via `inject_message` just above) — not
            // hardcoded, so the echoed marker's TRUST/SIG stays consistent
            // with its own ESCALATE= (reagentx P1 on PR #2623).
            req.reagent_verified,
            req.lan_verified,
            req.priority.as_deref().unwrap_or("normal"),
        );
        return Json(serde_json::to_value(&resp).unwrap_or_default());
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
        return Json(serde_json::to_value(&resp).unwrap_or_default());
    }

    // Every forward below sends this hop-incremented request, not the
    // original `req` — a peer that also fails to find the agent locally
    // and forwards onward needs to see the accumulated hop count too.
    let mut forwarded_req = req.clone();
    forwarded_req.forward_hops = req.forward_hops.saturating_add(1);

    if is_not_found {
        // Tier 2: same-host, different sidecar (file registry → HTTP loopback)
        let data_dir = base::get_wave_data_dir();
        if let Some(entry) = agent_registry::lookup(&data_dir, &req.target_agent) {
            // Guard against self-forwarding loops.
            if entry.local_url != state.local_web_url {
                let forward_url = format!("{}/agentmux/reactive/inject", entry.local_url);
                tracing::debug!(
                    target = %req.target_agent,
                    url = %forward_url,
                    "cross-instance inject forward"
                );
                let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
                if !entry.auth_key.is_empty() {
                    fwd = fwd.header("X-AuthKey", &entry.auth_key);
                }
                match fwd.send().await {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                echo_jekt_to_sender(
                                    &state,
                                    req.source_agent.as_deref(),
                                    &req.target_agent,
                                    &req.message,
                                    body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                    body.get("effective_tier").and_then(|v| v.as_str()),
                                    body.get("requires_stop").and_then(|v| v.as_bool()),
                                    "host",
                                    req.sig_verified,
                                    req.reagent_verified,
                                    req.lan_verified,
                                    req.priority.as_deref().unwrap_or("normal"),
                                );
                                return Json(body);
                            }
                            // success:false — could mean this entry is stale
                            // (e.g. agent unregistered without a clean
                            // shutdown) OR that the owning process is alive
                            // but hasn't (yet) registered this specific agent
                            // — e.g. right after that channel's srv came up.
                            // should_evict_on_forward_failure combines
                            // PID-liveness with the entry's age: a dead
                            // process always evicts; a live process only
                            // protects a FRESH entry (the actual startup
                            // race), not an old one — an old entry whose
                            // process happens to still be alive for OTHER
                            // agents is presumed to be its own genuinely-dead
                            // agent (reagent P1 round 2 on #2640: PID alone
                            // over-protects, since it identifies the whole
                            // srv process, not this one agent). See
                            // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                            // Falls through to Tier 2b/3 either way (reagent
                            // P1 on #2350 — this previously returned
                            // unconditionally whenever the body parsed,
                            // regardless of success, so a stale same-channel
                            // entry never fell through to any later tier).
                            if agent_registry::should_evict_on_forward_failure(&entry) {
                                tracing::warn!(
                                    target = %req.target_agent,
                                    pid = entry.pid,
                                    "cross-instance forward: success=false, entry presumed dead — evicting and falling through"
                                );
                                agent_registry::remove(&data_dir, &req.target_agent);
                            } else {
                                tracing::warn!(
                                    target = %req.target_agent,
                                    pid = entry.pid,
                                    "cross-instance forward: success=false but entry is fresh and owning process is alive — NOT evicting, falling through"
                                );
                            }
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            status = %r.status(),
                            url = %forward_url,
                            "cross-instance forward: non-success status"
                        );
                    }
                    Err(e) => {
                        // Connection-level failure (e.g. target port not
                        // listening yet) is at least as plausible a transient
                        // trigger as a parsed success:false body — same
                        // should_evict_on_forward_failure guard as that
                        // branch above (reagent P1 on this PR, both rounds:
                        // this Err(e) arm originally evicted unconditionally,
                        // then a PID-only guard over-protected an old entry
                        // whose process stayed alive for other agents). See
                        // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                        if agent_registry::should_evict_on_forward_failure(&entry) {
                            tracing::warn!(
                                target = %req.target_agent,
                                error = %e,
                                url = %forward_url,
                                pid = entry.pid,
                                "cross-instance forward failed, entry presumed dead — removing registry entry"
                            );
                            agent_registry::remove(&data_dir, &req.target_agent);
                        } else {
                            tracing::warn!(
                                target = %req.target_agent,
                                error = %e,
                                url = %forward_url,
                                pid = entry.pid,
                                "cross-instance forward failed but entry is fresh and owning process is alive — NOT evicting"
                            );
                        }
                    }
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

                let forward_url = format!("{}/agentmux/reactive/inject", entry.local_url);
                tracing::debug!(
                    target = %req.target_agent,
                    channel = %entry.channel,
                    url = %forward_url,
                    "cross-channel inject forward"
                );
                let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
                if !entry.auth_key.is_empty() {
                    fwd = fwd.header("X-AuthKey", &entry.auth_key);
                }
                match fwd.send().await {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                echo_jekt_to_sender(
                                    &state,
                                    req.source_agent.as_deref(),
                                    &req.target_agent,
                                    &req.message,
                                    body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                    body.get("effective_tier").and_then(|v| v.as_str()),
                                    body.get("requires_stop").and_then(|v| v.as_bool()),
                                    "host",
                                    req.sig_verified,
                                    req.reagent_verified,
                                    req.lan_verified,
                                    req.priority.as_deref().unwrap_or("normal"),
                                );
                                return Json(body);
                            }
                            // success:false — same ambiguity as Tier 2a above,
                            // same should_evict_on_forward_failure policy
                            // (PID-liveness alone over-protects an old,
                            // genuinely-dead individual agent whose srv
                            // process another agent keeps alive — reagent P1
                            // round 2 on #2640). See
                            // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                            if agent_registry::should_evict_on_forward_failure(&entry) {
                                tracing::warn!(
                                    target = %req.target_agent,
                                    channel = %entry.channel,
                                    pid = entry.pid,
                                    "cross-channel forward: success=false, entry presumed dead — evicting and trying next candidate"
                                );
                                agent_registry::remove_shared(&shared_dir, &req.target_agent, &entry.channel);
                            } else {
                                tracing::warn!(
                                    target = %req.target_agent,
                                    channel = %entry.channel,
                                    pid = entry.pid,
                                    "cross-channel forward: success=false but entry is fresh and owning process is alive — NOT evicting, trying next candidate"
                                );
                            }
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            channel = %entry.channel,
                            status = %r.status(),
                            url = %forward_url,
                            "cross-channel forward: non-success status"
                        );
                    }
                    Err(e) => {
                        // Same should_evict_on_forward_failure policy as the
                        // success:false branch above — a connection failure
                        // can be the exact same startup-race transient
                        // (target channel's srv not listening yet), not
                        // proof the specific agent is dead. reagent P1 on
                        // this PR, both rounds. See
                        // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                        if agent_registry::should_evict_on_forward_failure(&entry) {
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                error = %e,
                                url = %forward_url,
                                pid = entry.pid,
                                "cross-channel forward failed, entry presumed dead — evicting entry"
                            );
                            agent_registry::remove_shared(&shared_dir, &req.target_agent, &entry.channel);
                        } else {
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                error = %e,
                                url = %forward_url,
                                pid = entry.pid,
                                "cross-channel forward failed but entry is fresh and owning process is alive — NOT evicting"
                            );
                        }
                    }
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
            let forward_url = format!("{}/agentmux/reactive/inject", peer_url);
            tracing::debug!(
                target = %req.target_agent,
                url = %forward_url,
                "LAN peer inject forward"
            );
            let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
            if !peer_auth_key.is_empty() {
                fwd = fwd.header("X-AuthKey", &peer_auth_key);
            }
            match fwd.send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        // /reactive/inject always returns HTTP 200; check body.success
                        // to detect "agent not found on that peer" (e.g. after migration).
                        if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
                            tracing::warn!(
                                target = %req.target_agent,
                                url = %forward_url,
                                "LAN peer inject: success=false — evicting stale cache entry"
                            );
                            state.lan_discovery.evict_agent(&req.target_agent);
                        } else {
                            echo_jekt_to_sender(
                                &state,
                                req.source_agent.as_deref(),
                                &req.target_agent,
                                &req.message,
                                body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                body.get("effective_tier").and_then(|v| v.as_str()),
                                body.get("requires_stop").and_then(|v| v.as_bool()),
                                "lan",
                                None,
                                None, // reagent signing is WAN-only, never applies on the LAN forward path
                                req.lan_verified,
                                req.priority.as_deref().unwrap_or("normal"),
                            );
                            return Json(body);
                        }
                    }
                }
                Ok(r) => {
                    tracing::warn!(
                        target = %req.target_agent,
                        status = %r.status(),
                        url = %forward_url,
                        "LAN peer forward: non-success HTTP status"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target = %req.target_agent,
                        error = %e,
                        url = %forward_url,
                        "LAN peer forward failed — evicting cache entry"
                    );
                    state.lan_discovery.evict_agent(&req.target_agent);
                }
            }
        }
    }

    // 4. Return original error (muxbus-client will fall back to cloud relay).
    Json(serde_json::to_value(&resp).unwrap_or_default())
}

pub(super) async fn handle_reactive_agents(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agents = state.reactive_handler.list_agents();
    Json(serde_json::to_value(&agents).unwrap_or(json!([])))
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
            if let Ok(Some(pubkey)) = state.wstore.agent_lan_public_key_load(id) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("lan_public_key".to_string(), json!(pubkey));
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
    match state
        .reactive_handler
        .register_agent(&req.agent_id, &req.block_id, req.tab_id.as_deref())
    {
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
            let data_dir = base::get_wave_data_dir();
            agent_registry::write(&data_dir, &req.agent_id, &state.local_web_url, &req.block_id);

            // And to the host-global shared registry (Tier 2b) so instances
            // running in OTHER channels on this host can reach this agent
            // too — closes issue #1916 (Tier 2 previously only ever reached
            // the caller's own channel).
            agent_registry::write_shared_from_env(&req.agent_id, &state.local_web_url, &req.block_id);

            // Auto-watch this agent's Claude Code config dir for subagent JSONL files.
            // Pass block_id so subagent events are stamped with the owning pane,
            // letting the frontend route ⚡ panels to that pane only. See
            // `resolve_claude_config_dir`'s doc comment for why this must read
            // the block's own `cmd:env`, not just guess a path convention.
            let block = state.wstore.get::<crate::backend::obj::Block>(&req.block_id).ok().flatten();
            let empty_meta = crate::backend::obj::MetaMapType::new();
            // Identity-bound agents' real CLAUDE_CONFIG_DIR is never the
            // stale `cmd:env` snapshot below — see
            // `resolve_claude_config_dir`'s doc comment and
            // SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md.
            let bound_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                &state.wstore,
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
            state.broker.publish(crate::backend::wps::WaveEvent {
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
    match state.wstore.agent_jekt_key_ensure(&req.agent_id) {
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
}

pub(super) async fn handle_reactive_unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Json<serde_json::Value> {
    // Capture block_id before unregistering so we can emit the Swarm refresh event.
    let block_id = state.reactive_handler.get_agent(&req.agent_id)
        .map(|r| r.block_id.clone());

    state.reactive_handler.unregister_agent(&req.agent_id);
    // Also remove from cross-instance file registry.
    let data_dir = base::get_wave_data_dir();
    agent_registry::remove(&data_dir, &req.agent_id);
    // And from the host-global shared registry (Tier 2b).
    agent_registry::remove_shared_from_env(&req.agent_id);
    // Drop the subagent filesystem watcher (handle + channel + task) — the
    // symmetric teardown for the watch_agent() call in the register handler.
    // Passes block_id (captured above) so a shared-agent-id watcher with
    // another still-open dependent block survives this one's teardown.
    state.subagent_watcher.unwatch_agent(&req.agent_id, block_id.as_deref());
    // Notify cloud subscriber so it stops subscribing for this agent
    if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        sub.remove_agent(&req.agent_id);
    }

    // Symmetric refresh: tell the Swarm view this pane is gone.
    if let Some(bid) = block_id {
        state.broker.publish(crate::backend::wps::WaveEvent {
            event: "agent:reactive-unregistered".to_string(),
            scopes: vec![format!("block:{}", bid)],
            sender: String::new(),
            persist: 0,
            data: Some(json!({ "block_id": bid })),
        });
    }

    Json(json!({"success": true}))
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

    let Some(reg) = state.reactive_handler.get_agent(&params.agent) else {
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
    };
    let block_id = reg.block_id.clone();

    let (raw_bytes, _total_line_count) = match crate::backend::session_archive::read_session_output(
        &state.wstore,
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
        .header("X-AuthKey", &entry.auth_key)
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
mod transcript_cross_channel_tests {
    use super::*;
    use crate::server::tests::test_state;

    /// The regression this test guards: before Phase A
    /// (`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`),
    /// an unknown agent name always 404d straight out of the host-tier
    /// lookup. The refactor routes that miss through
    /// `handle_reactive_transcript_cross_channel` instead — this proves the
    /// common case (nothing found on any tier, which is what a fresh
    /// `test_state()` with no shared registry entries looks like) still
    /// ends in the same 404, not a panic or a wrongly-200'd empty body.
    #[tokio::test]
    async fn unknown_agent_still_404s_when_not_on_any_tier() {
        let state = test_state();
        let resp = handle_reactive_transcript(
            State(state),
            Query(TranscriptQuery {
                agent: "no-such-agent-anywhere".to_string(),
                max_lines: 100,
                forwarded: false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn empty_agent_param_is_bad_request_before_any_tier_lookup() {
        let state = test_state();
        let resp = handle_reactive_transcript(
            State(state),
            Query(TranscriptQuery {
                agent: String::new(),
                max_lines: 100,
                forwarded: false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn host_tier_hit_is_labeled_with_its_tier() {
        let state = test_state();
        let unique = uuid::Uuid::new_v4();
        let agent_id = format!("transcript-tier-test-{unique}");
        let block_id = format!("transcript-tier-block-{unique}");
        state
            .reactive_handler
            .register_agent(&agent_id, &block_id, None)
            .unwrap();
        state
            .filestore
            .make_file(
                &block_id,
                "output",
                crate::backend::storage::filestore::FileMeta::default(),
                crate::backend::storage::filestore::FileOpts::default(),
            )
            .expect("make_file");
        state
            .filestore
            .append_data(&block_id, "output", b"hello\n")
            .expect("append_data");
        let mut block = crate::backend::obj::Block {
            oid: block_id.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: Default::default(),
            subblockids: None,
        };
        state.wstore.insert(&mut block).expect("wstore insert");

        let resp = handle_reactive_transcript(
            State(state),
            Query(TranscriptQuery {
                agent: agent_id,
                max_lines: 100,
                forwarded: false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["tier"], "host");
    }

    /// The P1 regression this guards (reagent + codex on PR #2715): a
    /// forwarded request (`forwarded: true`) must 404 immediately on a
    /// host-tier miss, never attempt a second cross-channel lookup/forward
    /// — that's what caps a stale/cyclic shared-registry entry at exactly
    /// one hop instead of looping (self-forward) or chaining indefinitely
    /// (multi-instance cycle).
    #[tokio::test]
    async fn forwarded_request_404s_without_a_second_hop() {
        let state = test_state();
        let resp = handle_reactive_transcript(
            State(state),
            Query(TranscriptQuery {
                agent: "no-such-agent-anywhere".to_string(),
                max_lines: 100,
                forwarded: true,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

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
    Json(req): Json<SupervisorDecisionRequest>,
) -> Response {
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
    // check belongs at the HTTP boundary where `state.wstore` is available,
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
    // name/slug cross-namespace hazard `agents.rs`'s
    // `instance_get_by_name_and_by_slug_never_cross_the_others_namespace`
    // regression-tests for the read path.)
    if matches!(action, SupervisorAction::Nudge) {
        let opted_in = state
            .wstore
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
#[derive(serde::Serialize)]
pub(super) struct RemoteRegistrationEntry {
    channel: String,
    pid: u32,
    updated_at: u64,
}

/// Summary of the most recent `identity-mismatch` audit entry for this
/// agent_id, if any (see #2695's `Handler::inject_message_inner` check) —
/// narrowed from `AuditLogEntry` to what the Stash badge needs.
#[derive(serde::Serialize)]
pub(super) struct MismatchAuditSummary {
    timestamp: u64,
    block_id: String,
    error_message: Option<String>,
}

#[derive(serde::Serialize)]
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

#[derive(serde::Deserialize)]
pub(super) struct ReactiveRegistrationsParams {
    agent_id: String,
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
    engine.register_handler(
        "reactive.registrations",
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                let params: ReactiveRegistrationsParams = serde_json::from_value(data)
                    .map_err(|e| format!("reactive.registrations: {e}"))?;

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
                // (`entry.local_url == state.local_web_url`, ~line 454/580).
                let remote = crate::registry::resolve_shared_reactive_dir()
                    .map(|shared_dir| {
                        agent_registry::lookup_all_shared(&shared_dir, &params.agent_id)
                            .into_iter()
                            .filter(|e| !is_self_registration(&e.local_url, &state.local_web_url))
                            // A crashed sibling instance's entry otherwise
                            // lingers until the next startup-only
                            // cleanup_stale_shared sweep (bootstrap.rs) —
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
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );
}

/// `verify_jekt_signature` unit tests (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
/// §2.2, reagentx review on PR #2565). Deliberately test the extracted
/// function directly rather than the full `handle_inject`/websocket
/// handlers it's now called from: `server::tests::test_state()`'s
/// `reactive_handler` is a *global* singleton shared across every test in
/// the binary (`backend_reactive::get_global_handler()`), so exercising it
/// end-to-end here risks cross-test interference on shared agent
/// registrations. `verify_jekt_signature` itself only touches `state.wstore`
/// (key lookup), not the handler, so it's safe to test in isolation with no
/// such risk — and it's the one piece of logic actually being fixed here;
/// the two call sites (messagebus.rs, websocket.rs) are a one-line "call
/// this before inject_message" wiring, visible directly in their diffs.
#[cfg(test)]
mod verify_jekt_signature_tests {
    use super::*;
    use crate::server::tests::test_state;

    fn base_req(source_agent: &str, target_agent: &str, message: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: target_agent.to_string(),
            message: message.to_string(),
            source_agent: Some(source_agent.to_string()),
            delivery_tier: Some("host".to_string()),
            ..Default::default()
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[tokio::test]
    async fn a_correctly_signed_message_verifies_true() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key,
            req.request_id.as_deref().unwrap(),
            "agentx",
            "agenty",
            req.ts_secs.unwrap(),
            "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(true));
    }

    /// The core P0 fix this whole file's `verify_jekt_signature` extraction
    /// exists for: a claimed sender with a real key on file but NO
    /// signature attached (exactly what `messagebus.rs::handle_inject` and
    /// websocket.rs's `bus:inject` used to send, pre-fix) must render as a
    /// real, escalating "unverified" — not silently pass through unchecked.
    #[tokio::test]
    async fn a_claimed_sender_with_a_key_but_no_signature_is_unverified() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        // req.jekt_sig deliberately left None — the exact bypass shape.

        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified,
            Some(false),
            "a signable identity with no signature must be a real 'unverified,' not skipped"
        );
    }

    #[tokio::test]
    async fn no_key_on_file_leaves_sig_verified_unset() {
        let state = test_state();
        // No agent_jekt_key_ensure call — "slack", or any non-agent caller.
        let mut req = base_req("slack", "agenty", "hello");
        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified, None,
            "no key on file means nothing to check — must not be escalated"
        );
    }

    // Was named "network_tier_is_never_checked_regardless_of_signature" and
    // asserted the OPPOSITE of what's below — reagentx P0 (round 2) on the
    // LAN signing PR: that WAS the bypass. Gating this check on
    // delivery_tier meant a request could claim "wan"/"lan" for a
    // source_agent this instance actually has a local key for, skip host
    // verification entirely, and land unescalated. Flipped, not deleted, to
    // document the fix rather than silently change it — see
    // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §3's second
    // revision.
    #[tokio::test]
    async fn a_locally_known_sender_is_still_checked_even_under_a_claimed_network_tier() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();
        let mut req = base_req("agentx", "agenty", "hello");
        req.delivery_tier = Some("wan".to_string());
        // req.jekt_sig deliberately left None — claiming "wan" must not be a
        // way to dodge this check for an agent this instance actually knows.
        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified,
            Some(false),
            "a locally-known agent's identity, unsigned, must still be flagged regardless of \
             what delivery_tier the request claims — that claim is not a trust boundary"
        );
    }

    #[tokio::test]
    async fn a_genuinely_unknown_remote_sender_is_unaffected_by_a_claimed_network_tier() {
        let state = test_state();
        // No agent_jekt_key_ensure call — this instance never spawned "korp,"
        // exactly the shape of a real remote LAN/WAN agent.
        let mut req = base_req("korp", "agenty", "hello");
        req.delivery_tier = Some("lan".to_string());
        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified, None,
            "no local key for the claimed sender means nothing to check — running this \
             unconditionally must not manufacture a finding for genuinely remote traffic"
        );
    }

    /// Anti-replay (reagentx P1 on PR #2565): a signature that was valid
    /// once must stop verifying once its `ts_secs` falls outside the
    /// freshness window — otherwise a captured signed jekt replays forever.
    #[tokio::test]
    async fn a_stale_timestamp_fails_verification_even_with_a_correct_signature() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let stale_ts = now() - JEKT_SIG_MAX_AGE_SECS - 60; // well outside the window
        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(stale_ts);
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key, "msg-1", "agentx", "agenty", stale_ts, "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified,
            Some(false),
            "a mathematically correct signature must still fail outside the freshness window"
        );
    }

    #[tokio::test]
    async fn a_timestamp_just_inside_the_window_still_verifies() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let recent_ts = now() - (JEKT_SIG_MAX_AGE_SECS - 10);
        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(recent_ts);
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key, "msg-1", "agentx", "agenty", recent_ts, "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(true));
    }

    #[tokio::test]
    async fn a_wrong_signature_is_unverified() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        req.jekt_sig = Some("forged-not-a-real-signature".to_string());

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(false));
    }
}

/// `verify_reagent_signature` unit tests (reagentx P1 on PR #41 —
/// `InjectionRequest` declared `reagent_sig`/`reagent_key_id` as
/// deserializable input fields but nothing on the HTTP
/// `/agentmux/reactive/inject` path ever verified them, so a reagent-signed
/// notification delivered through `@agentmuxai/muxbus-client`'s
/// `pollAndDeliverInjections` arrived unsigned in effect).
#[cfg(test)]
mod verify_reagent_signature_tests {
    use super::*;

    // Reuses the exact fixture from agentmux-common/src/jekt_sign.rs's own
    // `a_correctly_signed_reagent_message_verifies` test: a signature
    // produced offline against the "reagent-v1-dev" pinned public key's
    // matching private half, over signed_material("msg-1",
    // "github-consumer", "agentx", 1000, "hello"). The private key isn't in
    // this repo (agentmux-cloud's Secrets Manager only) so a fresh signature
    // can't be minted at test time — `now` is passed explicitly instead of
    // wall-clock so this fixed ts_secs=1000 can be held inside the
    // freshness window on demand.
    const FIXTURE_SIG_B64: &str =
        "QehidZjJa2jYLPIPYSsVxUlm86W5Fdbr9PV3P4HJyZwJ68/HZR9EaAL0MpcVtTuZJW2+MMGebc0RH9HITNJGCw==";
    const FIXTURE_TS_SECS: i64 = 1_000;

    fn wan_req() -> InjectionRequest {
        InjectionRequest {
            target_agent: "agentx".to_string(),
            message: "hello".to_string(),
            source_agent: Some("github-consumer".to_string()),
            delivery_tier: Some("wan".to_string()),
            reagent_sig: Some(FIXTURE_SIG_B64.to_string()),
            reagent_key_id: Some("reagent-v1-dev".to_string()),
            reagent_msg_id: Some("msg-1".to_string()),
            reagent_ts_secs: Some(FIXTURE_TS_SECS),
            ..Default::default()
        }
    }

    #[test]
    fn a_correctly_signed_and_fresh_reagent_message_verifies() {
        let mut req = wan_req();
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(true));
    }

    #[test]
    fn a_correct_signature_outside_the_freshness_window_fails() {
        let mut req = wan_req();
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS + REAGENT_SIG_MAX_AGE_SECS + 60);
        assert_eq!(
            req.reagent_verified,
            Some(false),
            "a mathematically correct signature must still fail outside the freshness window"
        );
    }

    #[test]
    fn host_tier_is_never_checked_regardless_of_signature() {
        let mut req = wan_req();
        req.delivery_tier = Some("host".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
    }

    #[test]
    fn lan_tier_is_never_checked_regardless_of_signature() {
        let mut req = wan_req();
        req.delivery_tier = Some("lan".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
    }

    #[test]
    fn a_wrong_signature_is_unverified() {
        let mut req = wan_req();
        req.reagent_sig = Some("forged-not-a-real-signature".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(false));
    }

    #[test]
    fn an_unknown_key_id_is_unverified() {
        let mut req = wan_req();
        req.reagent_key_id = Some("reagent-v2-does-not-exist".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(false));
    }

    // A partial set of the four fields is "not signed," not "signed but
    // broken" — matches cloud_subscriber.rs's identical policy (see
    // reagent_key_id's doc comment in types.rs).
    #[test]
    fn a_partial_signature_set_is_treated_as_unsigned_not_invalid() {
        let mut req = wan_req();
        req.reagent_key_id = None;
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None);
    }

    #[test]
    fn no_reagent_fields_at_all_is_left_unset() {
        let mut req = wan_req();
        req.reagent_sig = None;
        req.reagent_key_id = None;
        req.reagent_msg_id = None;
        req.reagent_ts_secs = None;
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None);
    }
}

#[cfg(test)]
mod verify_lan_signature_tests {
    use super::*;
    use crate::server::tests::test_state;

    // test_state() has no real LAN peers discovered, so
    // find_agent_lan_pubkey always returns None here — these tests exercise
    // the paths reachable without one (tier scoping, no-signature-attempted,
    // no-pubkey-found). The actual verify_lan_jekt crypto — correct sig
    // verifies, wrong sig fails, tampered content/sender fails — is
    // exhaustively covered in agentmux-common/src/jekt_sign.rs; the tier
    // escalation this feeds into (is_lan_sig_invalid forcing sensitive,
    // TRUST=lan-verified rendering) is covered end-to-end via
    // Handler::inject_message in backend/reactive/tests.rs, driven directly
    // off req.lan_verified rather than through this HTTP-round-trip lookup.

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn lan_req(source_agent: &str, target_agent: &str, message: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: target_agent.to_string(),
            message: message.to_string(),
            source_agent: Some(source_agent.to_string()),
            delivery_tier: Some("lan".to_string()),
            request_id: Some("req-lan-1".to_string()),
            ts_secs: Some(now()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn lan_signature_verification_is_skipped_off_the_lan_tier() {
        let state = test_state();
        let mut req = lan_req("agentx", "agenty", "hello");
        req.delivery_tier = Some("host".to_string());
        req.lan_sig = Some("anything".to_string());
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(req.lan_verified, None, "lan signing only applies to the LAN tier");
    }

    #[tokio::test]
    async fn no_lan_sig_attempted_leaves_lan_verified_unset() {
        let state = test_state();
        let mut req = lan_req("agentx", "agenty", "hello");
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(req.lan_verified, None, "nothing to check when no signature was attempted");
    }

    // reagentx P0 follow-up regression test: a rate-limited pubkey lookup
    // must NOT be treated the same as "no key found." Without the fix, an
    // attacker could exhaust the limiter with junk lookups, then slip a
    // forged signature for a real agent's identity through as
    // unverified/benign instead of forced-sensitive.
    #[tokio::test]
    async fn rate_limited_pubkey_lookup_forces_failed_not_unset() {
        let state = test_state();
        // Burn through the fan-out rate limiter with distinct agent_ids —
        // each is a genuine cache miss (test_state() has zero real LAN
        // peers, so every one of these negatively caches after consuming
        // one token). LAN_PUBKEY_LOOKUP_RATE_LIMIT is 10/sec.
        for i in 0..10 {
            let _ = state
                .lan_discovery
                .find_agent_lan_pubkey(&format!("burn-{i}"), &state.http_client)
                .await;
        }
        // The 11th distinct lookup this second must be rate-limited.
        let mut req = lan_req("korp", "agenty", "hello");
        req.lan_sig = Some("forged-or-real-doesnt-matter".to_string());
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(
            req.lan_verified,
            Some(false),
            "a rate-limited lookup for a claimed sender with a real signature attempt must be \
             treated as a verification FAILURE, never silently left unset like a genuinely \
             unknown/unsigned sender"
        );
    }

    #[tokio::test]
    async fn a_claimed_sender_with_no_discoverable_pubkey_leaves_lan_verified_unset() {
        // A lan_sig IS present, but with zero LAN peers discovered
        // (test_state()'s default), find_agent_lan_pubkey can't find
        // anyone's public key — "nothing to check against" must not be
        // conflated with "the signature is invalid."
        let state = test_state();
        let mut req = lan_req("agentx", "agenty", "hello");
        req.lan_sig = Some("some-signature".to_string());
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(
            req.lan_verified, None,
            "an unfindable public key must not be conflated with a failed verification"
        );
    }
}

#[cfg(test)]
mod resolve_delivery_tier_tests {
    use super::*;

    #[test]
    fn lan_key_forces_lan_regardless_of_claim() {
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, Some("host")), "lan");
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, Some("wan")), "lan");
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, None), "lan");
    }

    #[test]
    fn full_auth_key_trusts_the_body_claim_including_lan() {
        // reagentx P0 regression: same-host Tier 2a/2b forwarding
        // re-authenticates an already-lan-tagged jekt with a sibling
        // instance's own full auth_key. If this ever downgrades "lan" to
        // "host" again, an already-detected LAN signature failure's
        // forced-sensitive escalation silently disappears on the second
        // hop (lan_verified resets to None, and verify_lan_signature only
        // runs when delivery_tier == "lan").
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("lan")), "lan");
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("wan")), "wan");
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("host")), "host");
    }

    #[test]
    fn full_auth_key_defaults_to_host_when_body_omits_the_field() {
        assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, None), "host");
    }
}

#[cfg(test)]
mod is_self_registration_tests {
    use super::is_self_registration;

    /// reagentx P1 on #2698: without this check, `reactive.registrations`'s
    /// `remote` list always included this instance's own shared-registry
    /// entry (every agent unconditionally writes itself into that same
    /// registry), so the "registered elsewhere too" badge fired on every
    /// healthy agent — defeating the whole point of the feature.
    #[test]
    fn same_local_url_is_self() {
        assert!(is_self_registration(
            "http://127.0.0.1:12345",
            "http://127.0.0.1:12345"
        ));
    }

    #[test]
    fn different_local_url_is_not_self() {
        assert!(!is_self_registration(
            "http://127.0.0.1:12345",
            "http://127.0.0.1:54321"
        ));
    }
}
