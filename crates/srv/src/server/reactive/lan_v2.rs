//! Identity M4d-6 (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.10): the LAN v2 signature, checked once the v1 LAN signature verified.
//! LAN peers are unauthenticated, so a UID it proves is recorded only as
//! claimed by that LAN path: never verified, never matched to a local row.

use super::{AppState, InjectionRequest};

/// Record `source_uid` as claimed by this LAN path when the answering peer
/// gave that UID's key for `claimed`, the `(name, uid)` pin matches, and the
/// v2 signature verifies under it. Only once the v1 check passed, so the
/// name pin already matched: the UID pin is set only beside it.
pub(super) fn claim_uid(state: &AppState, req: &mut InjectionRequest, claimed: &str) {
    if req.lan_verified != Some(true) {
        return;
    }
    let (Some(sig), Some(uid)) = (req.lan_sig_v2.clone(), req.source_uid.clone().filter(|u| !u.is_empty())) else {
        return;
    };
    let Some((answered_uid, key)) = state.lan_discovery.cached_lan_uid_key(claimed) else {
        return;
    };
    let record = crate::backend::agent_resolve::record_uid_fallback;
    if answered_uid != uid {
        record("m4d.lan_v2_other_uid");
        return;
    }
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let key_b64 = BASE64.encode(&key);
    match state.mstore.lan_peer_uid_pin_get_or_set(claimed, &uid, &key_b64) {
        Ok(pinned) if pinned == key_b64 => {}
        Ok(_) => {
            record("m4d.lan_v2_pin_mismatch");
            return;
        }
        Err(_) => return,
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
    }
}

#[cfg(test)]
#[path = "../tests/reactive/lan_v2_tests.rs"]
mod tests;
