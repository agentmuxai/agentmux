// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `claim_and_deliver`: the claim, what is checked about each claimed message
//! (signatures), the request it is delivered as, and its files.

use super::*;

/// What was checked about one claimed message: kept for the delivery and for noting it delivered.
pub(super) struct Checked {
    reagent_verified: Option<bool>,
    /// The signature is genuine and failed only the age rules: the marker says `SIG=expired`.
    reagent_expired: bool,
    /// (signed message id, agent, delivery id) of a reagent-signed message that verified: recorded once it is delivered.
    reagent_delivery: Option<(String, String, String)>,
    /// The delivery a verified signed message already went to this agent as, when this is a second copy of it.
    repeat_of: Option<String>,
    wan_row: crate::muxbus::wan_verify::CarriedRow,
    wan_store: Option<Arc<crate::backend::storage::wan_identity::WanIdentityStore>>,
    wan_verdict: crate::muxbus::wan_verify::WanVerdict,
}

/// The files could not be fetched and the claim was given back to the relay.
pub(super) struct GivenBack;

/// Claim `all_ids`. `Err` is the outcome that ends the call: nothing was claimed. The token is the
/// one the claim was made with, which a rejected agent credential changes to the shared one.
#[allow(clippy::too_many_arguments)]
pub(super) async fn claim(
    base: &str,
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    mstore: &Arc<Store>,
    all_ids: &[String],
    mut agent_token: String,
    mut using_per_agent: bool,
) -> Result<(AckResp, String), AgentSyncOutcome> {
    let ack_url = format!("{}/reactive/ack", base);
    let claimed: AckResp = loop {
        // For telling a 409 from before a Take over apart (note_not_holder_since).
        let claim_resp_started = std::time::Instant::now();
        let claim_resp = match http
            .post(&ack_url)
            .header("Authorization", format!("Bearer {}", agent_token))
            .header("X-Agent-ID", agent_id)
            .header("X-Agent-Instance", crate::muxbus::wan_lease::instance_id())
            .json(&serde_json::json!({ "injection_ids": all_ids }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "cloud_subscriber: claim request failed");
                delivery_status::pull_failed(agent_id, "couldn't claim messages from the relay");
                return Err(AgentSyncOutcome::Ok); // nothing claimed — retried on the next wake/poll
            }
        };

        if is_credential_rejected(claim_resp.status()) {
            if using_per_agent {
                // Same reasoning as the /reactive/pending branch above:
                // invalidate and retry THIS claim immediately with the
                // shared token, rather than reconnecting the whole session
                // or leaving this agent's already-fetched pending
                // injections unclaimed until an unrelated broadcast fires.
                invalidate_rejected_credential(agent_id, claim_resp.status(), mstore);
                agent_token = shared_token_now(mstore, token).await;
                using_per_agent = false;
                continue;
            }
            return Err(shared_token_rejection_outcome(claim_resp.status(), agent_id));
        }
        if claim_resp.status() == reqwest::StatusCode::CONFLICT {
            // The relay's lease fence on acks (the lease changed hands between
            // this pull and this claim): same handling as a 409 on the pull —
            // claim nothing, and fence the local holder (ReAgent P1 on #3746).
            let body = claim_resp.json::<serde_json::Value>().await.unwrap_or_default();
            if let Some(where_) = crate::muxbus::wan_lease::note_not_holder_since(agent_id, &body, claim_resp_started) {
                crate::backend::agent_admission::fence_agent_named(agent_id, &where_);
            }
            return Err(AgentSyncOutcome::Ok);
        }
        if !claim_resp.status().is_success() {
            tracing::warn!(
                status = %claim_resp.status(),
                agent_id = %agent_id,
                "cloud_subscriber: claim request non-2xx"
            );
            delivery_status::pull_failed(agent_id, format!("the relay answered {} to a message claim", claim_resp.status().as_u16()));
            return Err(AgentSyncOutcome::Ok); // nothing claimed — retried on the next wake/poll
        }

        break match claim_resp.json().await {
            Ok(b) => b,
            Err(e) => {
                // The server processed the claim (status was 2xx) before this
                // body failed to parse — some subset of `all_ids` may now be
                // flipped to "delivered" server-side with no local delivery
                // and no way for us to release them, since we don't know
                // which ids succeeded or their delivered_at stamp (required
                // by /reactive/release). We deliberately do NOT guess (e.g.
                // via /reactive/status) and release whatever looks delivered:
                // some of `all_ids` may have been legitimately claimed and
                // delivered by a *different* concurrent poller (another
                // channel/seat racing for the same agent_id), and blindly
                // releasing those would reintroduce the exact duplicate-
                // delivery bug this change exists to fix. Logging every
                // affected id loudly is the safe tradeoff: rare silent
                // message loss here, never a resurrected duplicate.
                tracing::error!(
                    agent_id = %agent_id,
                    injection_ids = ?all_ids,
                    error = %e,
                    "cloud_subscriber: parse claim response failed — these injections may be claimed server-side with no local delivery and cannot be safely auto-recovered"
                );
                return Err(AgentSyncOutcome::Ok);
            }
        };
    };
    Ok((claimed, agent_token))
}

/// Drop the per-agent credential a claim was rejected with: a 403 (binding mismatch) records its
/// failure, any other rejection clears only the cached token.
fn invalidate_rejected_credential(agent_id: &str, status: reqwest::StatusCode, mstore: &Arc<Store>) {
    if status == reqwest::StatusCode::FORBIDDEN {
        crate::muxbus::agent_credentials::invalidate_binding_mismatched_credential(agent_id, mstore);
    } else {
        crate::muxbus::agent_credentials::invalidate_cached_token(agent_id, mstore);
    }
    tracing::warn!(
        agent_id = %agent_id,
        status = %status,
        "cloud_subscriber: per-agent credential rejected on claim — invalidated, retrying with shared token",
    );
}

/// Check one claimed message: a reagent signature, and the WAN signature of a same-account agent.
pub(super) async fn verify(
    inj: &PendingInj,
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    mstore: &Arc<Store>,
) -> Checked {
    let reagent_verified = reagent_verdict(inj, agent_id, now_unix_secs());
    let reagent_expired = reagent_expired(inj, agent_id, now_unix_secs());
    // A verified signed message that already reached this agent under another delivery id is a second copy (the
    // sender retried a call the relay had in fact taken, or a captured notice was sent again): not delivered again.
    let reagent_delivery = match (reagent_verified, &inj.reagent_msg_id) {
        (Some(true), Some(msg_id)) => Some((msg_id.clone(), agent_id.to_string(), inj.id.clone())),
        _ => None,
    };
    let repeat_of = reagent_delivery
        .as_ref()
        .and_then(|(msg_id, agent, inj_id)| reagent_age::delivered_under_another(msg_id, agent, inj_id, now_unix_secs()));

    // W3-S (§2.3): a same-account agent's WAN signature, verified here —
    // before transcript-request resolution, and against the POLLED
    // agent, never a row field. Every "couldn't check" is `None` and the
    // message is delivered as today; only an active failure is
    // `Some(false)`.
    let wan_row = crate::muxbus::wan_verify::CarriedRow {
        wan_sig: inj.wan_sig.clone(),
        wan_msg_id: inj.wan_msg_id.clone(),
        wan_ts_secs: inj.wan_ts_secs,
        wan_source_agent: inj.wan_source_agent.clone(),
        wan_target_agent: inj.wan_target_agent.clone(),
        wan_source_host: inj.wan_source_host.clone(),
        wan_source_channel: inj.wan_source_channel.clone(),
        wan_key_fp: inj.wan_key_fp.clone(),
        sender_same_account: inj.sender_same_account,
    };
    let wan_store = crate::backend::storage::wan_identity::global();
    let wan_verdict = match (&wan_store, &inj.wan_sig) {
        (Some(wan), Some(_)) => match wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()) {
            Ok(own) => {
                let base_url = crate::muxbus::relay::rest_base_url();
                let dir_token = shared_token_now(mstore, token).await;
                let dir = crate::muxbus::wan_verify::Directory {
                    base_url: &base_url,
                    http,
                    token: &dir_token,
                    budget: &crate::muxbus::wan_verify::GLOBAL_BUDGET,
                };
                crate::muxbus::wan_verify::verify(
                    wan,
                    &own.instance_id,
                    agent_id,
                    inj.source_agent.as_deref().unwrap_or(""),
                    &wan_row,
                    &inj.message,
                    now_unix_secs(),
                    &dir,
                )
                .await
            }
            Err(e) => crate::muxbus::wan_verify::WanVerdict::unchecked("wan_no_local_instance", e.to_string()),
        },
        (None, Some(_)) => crate::muxbus::wan_verify::WanVerdict::unchecked("wan_no_local_store", "no wan.db attached"),
        (_, None) => crate::muxbus::wan_verify::WanVerdict::default(),
    };
    log_wan_outcome(inj, &wan_verdict);
    Checked { reagent_verified, reagent_expired, reagent_delivery, repeat_of, wan_row, wan_store, wan_verdict }
}

/// The verdict on a reagent-signed message; `None` when it carries no complete signature.
pub(super) fn reagent_verdict(inj: &PendingInj, agent_id: &str, now: i64) -> Option<bool> {
    // Verify a reagent-signed WAN message (see PendingInj's doc comment)
    // before delivery. Only attempted when every one of the four
    // signing fields is present — a partial set (e.g. a sig but no
    // key_id) is treated the same as "not signed," not "signed but
    // broken," since a legitimate sender always sends all four
    // together. This verdict never changes TRUST; it renders as `SIG=`.
    // `Some(false)` — a signature present but failed, or valid only
    // under an untrusted key — forces TIER=sensitive (see
    // InjectionRequest::reagent_verified,
    // SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md, and
    // https://docs.agentmux.ai/internals/jekt-protocol/).
    // `verify_trusted_reagent_jekt` yields `Some(true)` only
    // under the trusted production key; a signature under any other
    // registered key is `Some(false)`. `req.reagent_key_id` in `request` must still be carried
    // through from `inj` so the marker's `SIG=` label and the SIG=verified
    // rule-1b exception render correctly (reagentx P0 on PR #2576 — it
    // wasn't, so relaxation silently never activated for
    // the desktop app's WS delivery path despite `reagent_verified`
    // being computed correctly).
    //
    // Anti-replay (reagentx P1 on PR #2570): a captured, validly-signed
    // (reagent_sig, reagent_key_id, reagent_msg_id, reagent_ts_secs,
    // message) tuple could otherwise be resubmitted under a brand-new
    // injection `id` indefinitely and still render SIG=verified — same
    // class of gap host-tier's `verify_jekt_signature` closed with
    // `JEKT_SIG_MAX_AGE_SECS`. `REAGENT_SIG_MAX_AGE_SECS` is wider
    // (600s vs. 300s) because this is real network delivery, not a
    // same-machine call. A notice the relay held for a host that was off
    // is older than that when it arrives: it is judged by `reagent_age`
    // (its age when the relay accepted it, and a bound at delivery), not
    // by delivery time alone, which read a genuine late notice as a
    // forgery. A second copy of one already delivered is dropped by the
    // caller (`Checked::repeat_of`), not rendered as a forgery.
    reagent_checks(inj, agent_id, now).map(|(age_ok, signature_ok)| age_ok && signature_ok)
}

/// Whether a reagent-signed message's signature is genuine (under the trusted key) and fails only the age rules:
/// it renders `SIG=expired`, not `SIG=invalid`. It is still `reagent_verified == Some(false)`, so it is still
/// `TIER=sensitive` and still needs a stop.
pub(super) fn reagent_expired(inj: &PendingInj, agent_id: &str, now: i64) -> bool {
    reagent_checks(inj, agent_id, now) == Some((false, true))
}

/// What the age rules and the signature each say of a reagent-signed message, as (age ok, signature verifies
/// under the trusted key); `None` when it carries no complete signature.
fn reagent_checks(inj: &PendingInj, agent_id: &str, now: i64) -> Option<(bool, bool)> {
    match (&inj.reagent_sig, &inj.reagent_key_id, &inj.reagent_msg_id, inj.reagent_ts_secs) {
        (Some(sig), Some(key_id), Some(msg_id), Some(ts_secs)) => {
            // The relay's own record of when it accepted the message: a notice that waited for a host that
            // was off is judged by its age when it was accepted, within a bound at delivery, and once.
            let accepted_secs = inj.created_at.as_deref().and_then(delivery_status::notes::parse_created_at).map(|ms| ms / 1000);
            Some((
                reagent_age::age_ok(ts_secs, accepted_secs, now),
                agentmux_common::jekt_sign::verify_trusted_reagent_jekt(
                    key_id,
                    msg_id,
                    inj.source_agent.as_deref().unwrap_or(""),
                    agent_id,
                    ts_secs,
                    &inj.message,
                    sig,
                ),
            ))
        }
        _ => None,
    }
}

/// One line to the log for the WAN signature outcome of `inj`.
fn log_wan_outcome(inj: &PendingInj, wan_verdict: &crate::muxbus::wan_verify::WanVerdict) {
    // Every "couldn't check" renders exactly like an unsigned message
    // (`TRUST=network-claimed`), so say which one happened at a level
    // that is actually recorded. One line per WAN jekt.
    if inj.wan_sig.is_some() {
        let (verified, reason, detail) =
            (wan_verdict.verified, wan_verdict.reason.unwrap_or(""), wan_verdict.detail.as_deref().unwrap_or(""));
        if verified == Some(false) {
            tracing::warn!(injection_id = %inj.id, source = ?inj.source_agent, reason, "wan verify: signature FAILED");
        } else if !detail.is_empty() {
            // A check that couldn't run for a known cause (directory
            // refused or unreachable, no local store): the one line that
            // reports it, at `warn` — `wan_verify::get_json` no longer
            // logs each attempt.
            tracing::warn!(
                injection_id = %inj.id,
                source = ?inj.source_agent,
                verified = ?verified,
                reason,
                detail,
                same_account = ?inj.sender_same_account,
                "wan verify: outcome"
            );
        } else {
            tracing::info!(
                injection_id = %inj.id,
                source = ?inj.source_agent,
                verified = ?verified,
                reason,
                same_account = ?inj.sender_same_account,
                "wan verify: outcome"
            );
        }
    } else if inj.reagent_sig.is_none() {
        tracing::info!(injection_id = %inj.id, source = ?inj.source_agent, "wan verify: arrived without a WAN signature");
    }
}

/// The request a claimed message is delivered as, with the verdicts, its transcript-request fields and a
/// note when it was late.
pub(super) fn request(inj: &PendingInj, target: &str, checked: &Checked, mstore: &Arc<Store>) -> InjectionRequest {
    let mut req = InjectionRequest {
        reagent_verified: checked.reagent_verified,
        reagent_expired: checked.reagent_expired,
        wan_verified: checked.wan_verdict.verified,
        wan_instance: checked.wan_verdict.instance.clone(),
        wan_reason: checked.wan_verdict.reason.map(str::to_string),
        wan_detail: checked.wan_verdict.detail.clone(),
        ..wan_request(inj, target)
    };
    // Phase C (muxspect Phase B/C conversation visibility,
    // SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md): this WAN
    // delivery path calls `Handler::inject_message` directly and never
    // goes through `server/reactive.rs::handle_reactive_inject`/HTTP at
    // all — without also resolving these two fields HERE, a
    // WAN-delivered `transcript_request` would silently skip both the
    // forced-TIER=sensitive rule and the escalate-forcing rule
    // entirely, not just get a weaker version of them.
    crate::server::reactive::resolve_transcript_request_tier_fields(mstore, &mut req);
    // Marked only after the verdicts and the transcript-request fields have read the message as sent.
    let created_ms = inj.created_at.as_deref().and_then(delivery_status::notes::parse_created_at);
    let late = delivery_status::notes::delayed_prefix(created_ms, now_ms());
    if !late.is_empty() {
        req.message = format!("{late}{}", req.message);
    }
    req
}

/// Fetch the files sent with `inj` into `req`. A relay that cannot be reached gives the claim back.
pub(super) async fn attach_files(
    base: &str,
    http: &reqwest::Client,
    agent_token: &str,
    agent_id: &str,
    inj: &PendingInj,
    req: &mut InjectionRequest,
    delivered_at: &str,
) -> Result<(), GivenBack> {
    // Files sent with it: fetched now that the claim is ours. A relay that
    // can't be reached gives the claim back, for a retry with its files.
    if let Some(files) = inj.files.clone() {
        if let Err(e) =
            crate::server::jekt_cloud::fetch(base, http, agent_token, agent_id, req, files, inj.wan_msg_id.as_deref()).await
        {
            tracing::warn!(injection_id = %inj.id, error = %e, "cloud_subscriber: files not fetched; releasing for a retry");
            release(base, http, agent_token, agent_id, &inj.id, delivered_at).await;
            return Err(GivenBack);
        }
        crate::server::jekt_files::prepare(req).await;
    }
    Ok(())
}

/// A verified signed notice this agent already got under another delivery id: the sender signs once and may send
/// twice (a retry of a call the relay had taken), and a replay looks the same. Neither is shown: one warning names
/// both ids, and the caller delivers nothing. False for every other message.
pub(super) fn is_repeat(inj: &PendingInj, agent_id: &str, checked: &Checked) -> bool {
    let Some(delivered_as) = &checked.repeat_of else { return false };
    tracing::warn!(
        injection_id = %inj.id,
        delivered_as = %delivered_as,
        agent_id = %agent_id,
        "cloud_subscriber: a signed notice already delivered to this agent came again under another delivery id; not delivered again"
    );
    true
}

/// Note a signature as seen, once its message is delivered.
pub(super) fn record_delivered(checked: &Checked) {
    if let Some((msg_id, agent, inj_id)) = &checked.reagent_delivery {
        reagent_age::record_delivered(msg_id, agent, inj_id, now_unix_secs());
    }
    if let Some(wan) = &checked.wan_store {
        crate::muxbus::wan_verify::record_delivered(wan, &checked.wan_verdict, &checked.wan_row, now_unix_secs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A signature made offline under the production `reagent-v1` key over ("msg-1", "github-consumer", "agentx",
    // 1000, "hello"): the same fixture as `server/tests/reactive/verify_reagent_signature_tests.rs`.
    const SIG: &str = "FCFjcvAzla329a39u8fFxOvRWaH1R2fUn8RsGtP9RaIbLbaS3aXgAQ7YB4ssWV5TDvAeGrwSkHfoeGi11iCPBg==";
    const SIGNED_AT: i64 = 1_000;

    fn notice(id: &str, created_at: Option<&str>, sig: &str) -> PendingInj {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "source_agent": "github-consumer",
            "message": "hello",
            "created_at": created_at,
            "reagent_sig": sig,
            "reagent_key_id": "reagent-v1",
            "reagent_msg_id": "msg-1",
            "reagent_ts_secs": SIGNED_AT,
        }))
        .unwrap()
    }

    // 1970-01-01T00:16:45Z, five seconds after the signature was made.
    const ACCEPTED: &str = "1970-01-01T00:16:45Z";

    #[test]
    fn a_notice_signed_and_accepted_together_and_delivered_twenty_minutes_later_verifies() {
        // Signed 16:46, accepted 16:46, delivered 17:06.
        assert_eq!(reagent_verdict(&notice("inj-1", Some(ACCEPTED), SIG), "agentx", SIGNED_AT + 20 * 60), Some(true));
    }

    #[test]
    fn a_relay_time_that_does_not_fit_falls_back_to_the_delivery_time_rule() {
        let late = SIGNED_AT + 20 * 60;
        // No time from the relay (an older relay).
        assert_eq!(reagent_verdict(&notice("inj-1", None, SIG), "agentx", late), Some(false));
        // Accepted in the future, or before it was signed (beyond skew).
        assert_eq!(reagent_verdict(&notice("inj-1", Some("1970-01-01T01:00:00Z"), SIG), "agentx", late), Some(false));
        assert_eq!(reagent_verdict(&notice("inj-1", Some("1970-01-01T00:00:00Z"), SIG), "agentx", late), Some(false));
        // Accepted more than ten minutes after it was signed.
        assert_eq!(reagent_verdict(&notice("inj-1", Some("1970-01-01T00:27:00Z"), SIG), "agentx", late), Some(false));
        // Fresh at delivery needs no time from the relay.
        assert_eq!(reagent_verdict(&notice("inj-1", None, SIG), "agentx", SIGNED_AT + 60), Some(true));
    }

    #[test]
    fn a_notice_past_the_longest_hold_is_not_trusted_whatever_the_relay_says() {
        let at_the_end = SIGNED_AT + reagent_age::SIG_MAX_HOLD_SECS;
        assert_eq!(reagent_verdict(&notice("inj-1", Some(ACCEPTED), SIG), "agentx", at_the_end), Some(true));
        assert_eq!(reagent_verdict(&notice("inj-1", Some(ACCEPTED), SIG), "agentx", at_the_end + 1), Some(false));
    }

    #[test]
    fn a_genuine_signature_that_fails_only_the_age_rules_is_expired_not_invalid() {
        let late = SIGNED_AT + 20 * 60;
        // No time from the relay: past the delivery-time window, signature genuine. Still `Some(false)`, so still
        // sensitive and stopped; only the label differs.
        let held = notice("inj-1", None, SIG);
        assert_eq!(reagent_verdict(&held, "agentx", late), Some(false));
        assert!(reagent_expired(&held, "agentx", late));
        // Past the longest hold, whatever the relay says.
        assert!(reagent_expired(&notice("inj-1", Some(ACCEPTED), SIG), "agentx", SIGNED_AT + reagent_age::SIG_MAX_HOLD_SECS + 1));
        // Not expired: it verifies, or its signature is not genuine, or it carries none.
        assert!(!reagent_expired(&notice("inj-1", Some(ACCEPTED), SIG), "agentx", late));
        assert!(!reagent_expired(&notice("inj-1", None, "AAAA"), "agentx", late));
        assert!(!reagent_expired(&notice("inj-1", None, SIG), "agenty", late));
        let mut unsigned = notice("inj-1", None, SIG);
        unsigned.reagent_sig = None;
        assert!(!reagent_expired(&unsigned, "agentx", late));
    }

    #[test]
    fn a_second_copy_of_a_delivered_signed_message_still_verifies_and_is_dropped_not_shown_as_forged() {
        let now = SIGNED_AT + 20 * 60;
        // Not delivered yet: neither copy is a repeat, so a first attempt that never reached the agent does not
        // use up its retry.
        assert_eq!(reagent_age::delivered_under_another("msg-1", "agentx", "inj-retry", now), None);
        // The first copy is delivered.
        assert_eq!(reagent_verdict(&notice("inj-first", Some(ACCEPTED), SIG), "agentx", now), Some(true));
        reagent_age::record_delivered("msg-1", "agentx", "inj-first", now);
        // The second, under another delivery id: still a genuine signature, and a repeat of the first.
        assert_eq!(reagent_verdict(&notice("inj-retry", Some(ACCEPTED), SIG), "agentx", now + 10), Some(true));
        assert_eq!(reagent_age::delivered_under_another("msg-1", "agentx", "inj-retry", now + 10).as_deref(), Some("inj-first"));
        // The same delivery claimed again (a release and re-claim) is not.
        assert_eq!(reagent_age::delivered_under_another("msg-1", "agentx", "inj-first", now + 10), None);
        // Another recipient of the same signed id is not.
        assert_eq!(reagent_age::delivered_under_another("msg-1", "agenty", "inj-retry", now + 10), None);
    }

    #[test]
    fn a_bad_or_missing_signature_is_judged_exactly_as_before() {
        let now = SIGNED_AT + 20 * 60;
        // Wrong signature: present and failing.
        assert_eq!(reagent_verdict(&notice("inj-1", Some(ACCEPTED), "AAAA"), "agentx", now), Some(false));
        // Another agent's target: the signature covers who it was for.
        assert_eq!(reagent_verdict(&notice("inj-1", Some(ACCEPTED), SIG), "agenty", now), Some(false));
        // No signature at all is no verdict.
        let mut unsigned = notice("inj-1", Some(ACCEPTED), SIG);
        unsigned.reagent_sig = None;
        assert_eq!(reagent_verdict(&unsigned, "agentx", now), None);
    }
}
