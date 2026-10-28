//! Identity M4d-6 (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.10): the LAN v2 signature, checked once the v1 LAN signature verified.
//! LAN peers are unauthenticated, so a UID it proves is recorded only as
//! claimed by that LAN path: never verified, never matched to a local row.
//! A sender that presents a UID and a v2 signature has made a claim, though: a
//! claim that does not hold up counts as a failed signature, like a name key
//! that does not match its pin.
//!
//! [`sign_at_origin`] is the sending half for an agent whose own process
//! signed nothing (`SPEC_LAN_JEKT_SIGNED_BY_SRV_2026_10_25.md`).

use super::{AppState, InjectionRequest};

/// [`sign_at_origin_why`], and one INFO line saying what became of the signature
/// of a request about to go to the LAN peer at `peer_url`.
pub(super) fn sign_and_log(
    mstore: &crate::backend::storage::store::Store,
    caller_uid: &str,
    req: InjectionRequest,
    peer_url: &str,
) -> InjectionRequest {
    let (req, signature) = sign_at_origin_why(mstore, caller_uid, req);
    tracing::info!(target = %req.target_agent, source = req.source_agent.as_deref().unwrap_or(""), peer = %peer_url, v2 = req.lan_sig_v2.is_some(), "LAN forward: {signature}");
    req
}

/// One INFO line for a request that arrived over the LAN: what was made of its signature.
pub(super) fn log_receipt(req: &InjectionRequest) {
    if let Some(note) = receipt_note(req) {
        tracing::info!(target = %req.target_agent, source = req.source_agent.as_deref().unwrap_or(""), v2 = req.lan_sig_v2.is_some(), "LAN receipt: {note}");
    }
}

/// In a word, what the receiving side made of a LAN request's signature, for
/// the log line of every LAN receipt: a peer that sees `network-claimed` on a
/// message can then be told whether nothing was signed or nothing could be
/// checked. `None` for a request that did not arrive over the LAN.
pub(super) fn receipt_note(req: &InjectionRequest) -> Option<&'static str> {
    if req.delivery_tier.as_deref() != Some("lan") {
        return None;
    }
    Some(match req.lan_verified {
        Some(true) => "signature verified",
        Some(false) => "signature failed verification",
        None if req.lan_sig.is_none() => "unverified: no signature came with it",
        None if req.source_agent.as_deref().is_none_or(str::is_empty) => "unverified: signed, but names no sender",
        None => "unverified: signed, but no key for the sender was found among the LAN peers",
    })
}

/// Record `source_uid` as claimed by this LAN path when the answering peer
/// gave that UID's key for `claimed`, the `(name, uid)` pin matches, and the
/// v2 signature verifies under it. Only once the v1 check passed, so the
/// name pin already matched: the UID pin is set only beside it.
///
/// A request that carries both a UID and a v2 signature but fails any of those
/// checks (the peer gave another UID for the name, the pin holds a different
/// key, the signature does not verify, the pin store cannot be read) is no
/// longer verified: `lan_verified` becomes `Some(false)`. A request with no v2
/// at all is left as v1 decided it, and so is one whose peer gave no UID key
/// for a name that never gave one: there is nothing to contradict. Once a UID
/// is pinned under the name, the peer owes its key again: a cache that no
/// longer holds it is asked afresh (`name_key` must come back unchanged), and
/// an answer without it fails the jekt rather than fall back to v1.
pub(super) async fn claim_uid(state: &AppState, req: &mut InjectionRequest, claimed: &str, name_key: &[u8]) {
    if req.lan_verified != Some(true) {
        return;
    }
    let (Some(sig), Some(uid)) = (req.lan_sig_v2.clone(), req.source_uid.clone().filter(|u| !u.is_empty())) else {
        return;
    };
    let record = crate::backend::agent_resolve::record_uid_fallback;
    let Some((answered_uid, key)) = uid_key_owed(state, claimed, name_key).await else {
        if state.mstore.lan_peer_uid_pin_exists(claimed).unwrap_or(true) {
            record("m4d.lan_v2_pinned_peer_gave_no_uid_key");
            req.lan_verified = Some(false);
        }
        return;
    };
    if answered_uid != uid {
        record("m4d.lan_v2_other_uid");
        req.lan_verified = Some(false);
        return;
    }
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let key_b64 = BASE64.encode(&key);
    match state.mstore.lan_peer_uid_pin_get_or_set(claimed, &uid, &key_b64) {
        Ok(pinned) if pinned == key_b64 => {}
        Ok(_) => {
            record("m4d.lan_v2_pin_mismatch");
            req.lan_verified = Some(false);
            return;
        }
        Err(_) => {
            req.lan_verified = Some(false); // fail closed, as for the name pin
            return;
        }
    }
    let verified = agentmux_common::jekt_sign::verify_lan_jekt_v2(
        &key,
        req.request_id.as_deref().unwrap_or(""),
        claimed,
        &uid,
        &req.target_agent,
        req.ts_secs.unwrap_or(0),
        &req.message,
        &sig,
    );
    if verified {
        req.lan_claimed_uid = uid;
    } else {
        record("m4d.lan_v2_bad_sig");
        req.lan_verified = Some(false);
    }
}

/// The UID key the peer gave beside `claimed`'s name key: the cached answer,
/// or, when a UID is already pinned under the name and the cache has none, one
/// fresh ask.
async fn uid_key_owed(state: &AppState, claimed: &str, name_key: &[u8]) -> Option<(String, Vec<u8>)> {
    if let Some(answer) = state.lan_discovery.cached_lan_uid_key(claimed) {
        return Some(answer);
    }
    if !state.mstore.lan_peer_uid_pin_exists(claimed).unwrap_or(true) {
        return None;
    }
    state.lan_discovery.refetch_lan_uid_key(claimed, name_key, &state.http_client).await
}

/// The LAN forward of a jekt from an agent that signed nothing itself. A Codex
/// agent's jekts arrived at every LAN peer with no signature (so
/// `TRUST=network-claimed`, however good the receiver's checks) though srv held
/// the agent's key and its token named it. As for WAN
/// (`SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md` §3.1), srv signs, here at the
/// origin, for the agent the request's own token names, and only then:
///
/// 1. `caller_uid` is non-empty: this srv saw the agent's token, so this is
///    the origin, not a hop (a hop carries no `Caller`).
/// 2. The request is host-tier, as every origin request is.
/// 3. Nothing is signed yet: an agent's own `lan_sig` is never replaced.
/// 4. The token's row owns `source_agent` alone (`actor::wan_signer`).
///
/// It signs with that name's LAN key (v1), and with the UID-keyed key (v2) when
/// the sender carried no UID of its own, the same two signatures the MCP makes.
/// The receiver checks exactly what it checked before; nothing there changes.
/// Any unmet condition, or a missing key, returns the request as it came.
#[cfg(test)]
pub(super) fn sign_at_origin(
    mstore: &crate::backend::storage::store::Store,
    caller_uid: &str,
    req: InjectionRequest,
) -> InjectionRequest {
    sign_at_origin_why(mstore, caller_uid, req).0
}

/// [`sign_at_origin`], and in a word what became of the signature: the agent
/// signed it itself, srv signed it, or why neither (for the log line of a LAN
/// forward, so a peer that sees `network-claimed` can be explained).
pub(super) fn sign_at_origin_why(
    mstore: &crate::backend::storage::store::Store,
    caller_uid: &str,
    mut req: InjectionRequest,
) -> (InjectionRequest, &'static str) {
    if req.lan_sig.is_some() {
        return (req, "signed by the agent's own process");
    }
    if req.delivery_tier.as_deref() != Some("host") {
        return (req, "unsigned: a forwarded request, not the origin (not host-tier)");
    }
    let Some(signer) = crate::server::actor::wan_signer(mstore, caller_uid, &req).map(str::to_string) else {
        return (req, "unsigned: no agent token names the sender (or its row doesn't own the name alone)");
    };
    let Some(key) = mstore.agent_lan_key_ensure(&signer).ok().and_then(|k| agentmux_common::jekt_sign::decode_key(&k.private_key)) else {
        return (req, "unsigned: no LAN key for the sender");
    };
    let (msgid, ts) = msgid_and_ts(&req);
    let Some(sig) = agentmux_common::jekt_sign::sign_lan_jekt(&key, &msgid, &signer, &req.target_agent, ts, &req.message) else {
        return (req, "unsigned: signing failed");
    };
    req.request_id = Some(msgid.clone());
    req.ts_secs = Some(ts);
    req.lan_sig = Some(sig);
    if req.lan_sig_v2.is_none() && req.source_uid.as_deref().is_none_or(str::is_empty) {
        let uid_key = mstore.agent_uid_keys_ensure(caller_uid).ok().flatten()
            .and_then(|(keys, _)| agentmux_common::jekt_sign::decode_key(&keys.lan.private_key));
        if let Some(v2) = uid_key.and_then(|k| agentmux_common::jekt_sign::sign_lan_jekt_v2(&k, &msgid, &signer, caller_uid, &req.target_agent, ts, &req.message)) {
            req.source_uid = Some(caller_uid.to_string());
            req.lan_sig_v2 = Some(v2);
        }
    }
    tracing::info!(agent_id = %signer, "LAN jekt signed at the origin: the sender's process signed nothing");
    (req, "signed by srv at the origin")
}

/// The message id and time a signature covers. The MCP sets both; a sender that did not (a shell's
/// `muxsh agent send`) gets them here, so the signature has something to cover.
fn msgid_and_ts(req: &InjectionRequest) -> (String, i64) {
    let msgid = req.request_id.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}-{}-s{n}", agentmux_common::time::now_ms_u64(), std::process::id())
    });
    (msgid, req.ts_secs.filter(|t| *t > 0).unwrap_or_else(agentmux_common::time::now_secs))
}

/// The same-machine, other-instance counterpart of [`sign_at_origin`]: the cross-channel signatures
/// (`channel_sig`, and `channel_sig_v2` when the sender carried no UID of its own) for an agent whose own
/// process signed nothing, so the receiving instance still finds a published key's signature on it. The
/// conditions are `sign_at_origin`'s, and a signature already there is never replaced.
pub(super) fn channel_sign_at_origin(
    mstore: &crate::backend::storage::store::Store,
    caller_uid: &str,
    mut req: InjectionRequest,
) -> InjectionRequest {
    if req.channel_sig.is_some() || req.delivery_tier.as_deref() != Some("host") {
        return req;
    }
    let Some(signer) = crate::server::actor::wan_signer(mstore, caller_uid, &req).map(str::to_string) else {
        return req;
    };
    let Some(key) = mstore.agent_lan_key_ensure(&signer).ok().and_then(|k| agentmux_common::jekt_sign::decode_key(&k.private_key)) else {
        return req;
    };
    let (msgid, ts) = msgid_and_ts(&req);
    let channel = crate::backend::reactive::registry::local_channel_id();
    let Some(sig) = agentmux_common::jekt_sign::sign_channel_jekt(&key, &msgid, &signer, &channel, &req.target_agent, ts, &req.message) else {
        return req;
    };
    req.request_id = Some(msgid.clone());
    req.ts_secs = Some(ts);
    req.channel_sig = Some(sig);
    if req.channel_sig_v2.is_none() && req.source_uid.as_deref().is_none_or(str::is_empty) {
        let uid_key = mstore.agent_uid_keys_ensure(caller_uid).ok().flatten()
            .and_then(|(keys, _)| agentmux_common::jekt_sign::decode_key(&keys.lan.private_key));
        if let Some(v2) = uid_key.and_then(|k| agentmux_common::jekt_sign::sign_channel_jekt_v2(&k, &msgid, &signer, caller_uid, &channel, &req.target_agent, ts, &req.message)) {
            req.source_uid = Some(caller_uid.to_string());
            req.channel_sig_v2 = Some(v2);
        }
    }
    req.source_channel = Some(channel);
    tracing::info!(agent_id = %signer, "cross-channel jekt signed at the origin: the sender's process signed nothing");
    req
}

#[cfg(test)]
#[path = "../tests/reactive/lan_v2_tests.rs"]
mod tests;
