// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `claim_and_deliver`: the claim, what is checked about each claimed message
//! (signatures), the request it is delivered as, and its files.

use super::*;

/// What was checked about one claimed message: kept for the delivery and for noting it delivered.
pub(super) struct Checked {
    reagent_verified: Option<bool>,
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
    let reagent_verified = reagent_verdict(inj, agent_id);

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
    Checked { reagent_verified, wan_row, wan_store, wan_verdict }
}

/// The verdict on a reagent-signed message; `None` when it carries no complete signature.
fn reagent_verdict(inj: &PendingInj, agent_id: &str) -> Option<bool> {
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
    // same-machine call — matches the review notifications' own
    // delivery window, so a message
    // that's still legitimately within its own staleness budget never
    // spuriously fails signature freshness on top of that.
    match (&inj.reagent_sig, &inj.reagent_key_id, &inj.reagent_msg_id, inj.reagent_ts_secs) {
        (Some(sig), Some(key_id), Some(msg_id), Some(ts_secs)) => {
            Some(
                reagent_sig_is_fresh(ts_secs, now_unix_secs())
                    && agentmux_common::jekt_sign::verify_trusted_reagent_jekt(
                        key_id,
                        msg_id,
                        inj.source_agent.as_deref().unwrap_or(""),
                        agent_id,
                        ts_secs,
                        &inj.message,
                        sig,
                    ),
            )
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

/// Note a signature as seen, once its message is delivered.
pub(super) fn record_delivered(checked: &Checked) {
    if let Some(wan) = &checked.wan_store {
        crate::muxbus::wan_verify::record_delivered(wan, &checked.wan_verdict, &checked.wan_row, now_unix_secs());
    }
}
