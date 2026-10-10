//! Identity M4d-6 (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.10): cross-channel jekts verified by the sender's UID, not its name.

use super::{agent_registry, AppState, InjectionRequest, CHANNEL_SIG_MAX_AGE_SECS};

/// Settle a cross-channel jekt's checks by its v2 signature, when it proves
/// the sender is the agent this instance knows by the claimed name (or one it
/// knows no agent by). `true` when settled, either way; `false` leaves the
/// name's checks to the caller.
pub(super) fn settle_by_uid(
    state: &AppState,
    req: &mut InjectionRequest,
    claimed: &str,
    shared_dir: &std::path::Path,
    now_secs: i64,
) -> bool {
    if verify_channel_v2(req, shared_dir, now_secs) != Some(true) {
        return false;
    }
    let uid = req.source_uid.clone().unwrap_or_default();
    match state.mstore.agent_ids_for_slug_folded(claimed).as_deref() {
        Ok([local]) if *local == uid => {
            req.channel_verified = Some(true);
            req.sig_verified = None;
            if req.audit_source_uid.is_empty() {
                req.audit_source_uid = uid;
            }
            true
        }
        Ok([]) if !matches!(state.mstore.agent_jekt_key_load(claimed), Ok(Some(_))) => {
            req.channel_verified = Some(true);
            if req.audit_source_uid.is_empty() {
                req.audit_source_uid = uid;
            }
            true
        }
        // Proven to be another agent than the one this instance knows by the
        // name (or the name is ambiguous here): v1, which accepts any key
        // published under the name, must not pass it as that name.
        Ok(_) => {
            crate::backend::agent_resolve::record_uid_fallback("m4d.channel_v2_other_agent");
            req.channel_verified = Some(false);
            true
        }
        // The store couldn't say: the name's own checks decide.
        Err(_) => false,
    }
}

/// Identity M4d-6: the v2 cross-channel signature against the keys published
/// for its `source_uid` (M4d-5). `None` when there is nothing to check (no
/// v2 sent, no key published for the UID); `Some(false)` when one was
/// published and the signature, or its freshness, fails.
fn verify_channel_v2(
    req: &InjectionRequest,
    shared_dir: &std::path::Path,
    now_secs: i64,
) -> Option<bool> {
    let sig = req.channel_sig_v2.as_deref()?;
    let uid = req.source_uid.as_deref().filter(|u| !u.is_empty())?;
    let source = req.source_agent.as_deref()?;
    let keys: Vec<Vec<u8>> = agent_registry::lookup_shared_by_uid(shared_dir, uid)
        .into_iter()
        .filter_map(|e| agentmux_common::jekt_sign::decode_key(&e.uid_public_key))
        .collect();
    if keys.is_empty() {
        return None;
    }
    let ts = req.ts_secs.unwrap_or(0);
    let fresh = ts > 0 && (now_secs - ts).abs() <= CHANNEL_SIG_MAX_AGE_SECS;
    let msgid = req.request_id.as_deref().unwrap_or("");
    let channel = req.source_channel.as_deref().unwrap_or("");
    Some(
        fresh
            && keys.iter().any(|key| {
                agentmux_common::jekt_sign::verify_channel_jekt_v2(
                    key,
                    msgid,
                    source,
                    uid,
                    channel,
                    &req.target_agent,
                    ts,
                    &req.message,
                    sig,
                )
            }),
    )
}
