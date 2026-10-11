// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claim → verify → deliver for one agent's pulled injections: the half of a
//! pull that comes after the relay answered, shared by the single pull
//! (`sync_agent_reactive`) and the batch pull (`sync_agents_batched`).

use super::*;

/// One agent's answer to a pending pull, single or batched.
#[derive(Deserialize)]
pub(super) struct PendingResp {
    pub(super) injections: Vec<PendingInj>,
    // What the relay dropped undelivered since the last pull. Absent
    // from an older relay: nothing to report.
    #[serde(default)]
    pub(super) expired: Vec<delivery_status::notes::ExpiredItem>,
}
#[derive(Deserialize)]
pub(super) struct PendingInj {
    id: String,
    source_agent: Option<String>,
    message: String,
    priority: Option<String>,
    // When the relay received it (RFC 3339), for marking a late delivery.
    #[serde(default)]
    created_at: Option<String>,
    // Present only for messages signed by an AgentMux-operated WAN
    // sender (currently just the GitHub review-notification consumer,
    // "reagent") — see agentmux_common::jekt_sign::verify_reagent_jekt.
    // All four travel together; verification is attempted only when
    // every one of them deserializes present (see below).
    #[serde(default)]
    reagent_sig: Option<String>,
    #[serde(default)]
    reagent_key_id: Option<String>,
    #[serde(default)]
    reagent_msg_id: Option<String>,
    #[serde(default)]
    reagent_ts_secs: Option<i64>,
    // W3-S carried tuple + the cloud's same-account answer
    // (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.1). Absent from an
    // older cloud and on unsigned rows; verified by `wan_verify` below.
    #[serde(default)]
    wan_sig: Option<String>,
    #[serde(default)]
    wan_msg_id: Option<String>,
    #[serde(default)]
    wan_ts_secs: Option<i64>,
    #[serde(default)]
    wan_source_agent: Option<String>,
    #[serde(default)]
    wan_target_agent: Option<String>,
    #[serde(default)]
    wan_source_host: Option<String>,
    #[serde(default)]
    wan_source_channel: Option<String>,
    #[serde(default)]
    wan_key_fp: Option<String>,
    #[serde(default)]
    sender_same_account: Option<bool>,
}
#[derive(Deserialize)]
pub(super) struct AckResp {
    acknowledged: Vec<String>,
    delivered_at: String,
}

/// Claim `body`'s injections for `agent_id` and deliver the ones this call
/// won: the second half of [`sync_agent_reactive`], shared with
/// [`sync_agents_batched`]. `agent_token` is the credential the pull used
/// (`using_per_agent` when it is the agent's own), retried with the shared
/// token on a rejection exactly as the pull is.
#[allow(clippy::too_many_arguments)]
pub(super) async fn claim_and_deliver(
    base: &str,
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    mstore: &Arc<Store>,
    handler: &'static crate::backend::reactive::handler::ReactiveHandler,
    body: PendingResp,
    mut agent_token: String,
    mut using_per_agent: bool,
) -> AgentSyncOutcome {
    if body.injections.is_empty() {
        delivery_status::fetched(agent_id, 0, &body.expired);
        return AgentSyncOutcome::Ok;
    }

    // Claim BEFORE delivering, not after: this is what prevents
    // double-delivery when the same agent_id is concurrently registered by
    // another AgentMux channel on this host (an intentionally-supported
    // "two seats" workflow — see
    // docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md).
    // /reactive/ack now performs an atomic pending->delivered transition
    // server-side; only injection ids that come back in `acknowledged` were
    // actually won by this call and get delivered locally below. A
    // previous version of this code delivered first and only acked
    // successes afterward, which let two concurrent pollers both deliver
    // the same injection.
    let all_ids: Vec<String> = body.injections.iter().map(|inj| inj.id.clone()).collect();
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
                return AgentSyncOutcome::Ok; // nothing claimed — retried on the next wake/poll
            }
        };

        if is_credential_rejected(claim_resp.status()) {
            if using_per_agent {
                // Same reasoning as the /reactive/pending branch above:
                // invalidate and retry THIS claim immediately with the
                // shared token, rather than reconnecting the whole session
                // or leaving this agent's already-fetched pending
                // injections unclaimed until an unrelated broadcast fires.
                if claim_resp.status() == reqwest::StatusCode::FORBIDDEN {
                    crate::muxbus::agent_credentials::invalidate_binding_mismatched_credential(agent_id, mstore);
                } else {
                    crate::muxbus::agent_credentials::invalidate_cached_token(agent_id, mstore);
                }
                tracing::warn!(
                    agent_id = %agent_id,
                    status = %claim_resp.status(),
                    "cloud_subscriber: per-agent credential rejected on claim — invalidated, retrying with shared token",
                );
                agent_token = shared_token_now(mstore, token).await;
                using_per_agent = false;
                continue;
            }
            return shared_token_rejection_outcome(claim_resp.status(), agent_id);
        }
        if claim_resp.status() == reqwest::StatusCode::CONFLICT {
            // The relay's lease fence on acks (the lease changed hands between
            // this pull and this claim): same handling as a 409 on the pull —
            // claim nothing, and fence the local holder (ReAgent P1 on #3746).
            let body = claim_resp.json::<serde_json::Value>().await.unwrap_or_default();
            if let Some(where_) = crate::muxbus::wan_lease::note_not_holder_since(agent_id, &body, claim_resp_started) {
                crate::backend::agent_admission::fence_agent_named(agent_id, &where_);
            }
            return AgentSyncOutcome::Ok;
        }
        if !claim_resp.status().is_success() {
            tracing::warn!(
                status = %claim_resp.status(),
                agent_id = %agent_id,
                "cloud_subscriber: claim request non-2xx"
            );
            delivery_status::pull_failed(agent_id, format!("the relay answered {} to a message claim", claim_resp.status().as_u16()));
            return AgentSyncOutcome::Ok; // nothing claimed — retried on the next wake/poll
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
                return AgentSyncOutcome::Ok;
            }
        };
    };

    let delivered_at = claimed.delivered_at;
    let claimed_ids: std::collections::HashSet<String> = claimed.acknowledged.into_iter().collect();
    let mut delivered_count = 0usize;

    for inj in &body.injections {
        if !claimed_ids.contains(&inj.id) {
            // Another concurrent poller (this account's other channel/seat)
            // already won this claim — expected under the "two seats"
            // workflow, not an error.
            continue;
        }

        // Verify a reagent-signed WAN message (see PendingInj's doc comment)
        // before delivery. Only attempted when every one of the four
        // signing fields is present — a partial set (e.g. a sig but no
        // key_id) is treated the same as "not signed," not "signed but
        // broken," since a legitimate sender always sends all four
        // together. TRUST is unaffected either way (WAN stays
        // unconditionally TRUST=network-claimed). TIER is only FORCED to
        // sensitive when `reagent_verified` is `Some(false)` — a signature
        // present but which failed to cryptographically verify (see
        // InjectionRequest::reagent_verified,
        // SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md); as of that
        // narrowing. `verify_trusted_reagent_jekt` yields `Some(true)` only
        // under the trusted production key; a signature under any other
        // registered key is `Some(false)`. `req.reagent_key_id` below must still be carried
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
        let reagent_verified = match (&inj.reagent_sig, &inj.reagent_key_id, &inj.reagent_msg_id, inj.reagent_ts_secs) {
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
        };

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

        let mut req = InjectionRequest {
            target_agent: agent_id.to_string(),
            message: inj.message.clone(),
            source_agent: inj.source_agent.clone(),
            request_id: Some(inj.id.clone()),
            priority: inj.priority.clone(),
            jekt_tier: None,       // auto-detected from keywords
            delivery_tier: Some("wan".to_string()),
            forward_hops: 0,
            reagent_verified,
            reagent_key_id: inj.reagent_key_id.clone(),
            wan_verified: wan_verdict.verified,
            wan_instance: wan_verdict.instance.clone(),
            wan_reason: wan_verdict.reason.map(str::to_string),
            wan_detail: wan_verdict.detail.clone(),
            ..Default::default()
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
        // Marked only after every check above has read the message as sent.
        let created_ms = inj.created_at.as_deref().and_then(delivery_status::notes::parse_created_at);
        let late = delivery_status::notes::delayed_prefix(created_ms, now_ms());
        if !late.is_empty() {
            req.message = format!("{late}{}", req.message);
        }
        let delivery = handler.inject_message(req);
        tracing::debug!(
            injection_id = %inj.id,
            agent_id = %agent_id,
            success = delivery.success,
            "cloud_subscriber: delivered injection"
        );

        // W3-S §2.5: only now, after it landed, is this signature "seen" —
        // a release-and-redeliver after a failed delivery must not read as
        // a replay.
        if delivery.success {
            delivered_count += 1;
            if let Some(wan) = &wan_store {
                crate::muxbus::wan_verify::record_delivered(wan, &wan_verdict, &wan_row, now_unix_secs());
            }
        }

        if !delivery.success {
            // We hold the claim but local delivery failed (e.g. agent not
            // ready) — release it back to pending so it's retried, instead
            // of silently dropping it now that claiming already marked it
            // "delivered".
            let release_url = format!("{}/reactive/release", base);
            match http
                .post(&release_url)
                .header("Authorization", format!("Bearer {}", agent_token))
                .header("X-Agent-ID", agent_id)
                .json(&serde_json::json!({
                    "injection_id": inj.id,
                    "delivered_at": delivered_at,
                }))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    // Claim is stranded as "delivered" with no local
                    // delivery and no retry until this is visible — log
                    // loudly rather than discarding silently.
                    tracing::warn!(
                        injection_id = %inj.id,
                        agent_id = %agent_id,
                        status = %r.status(),
                        "cloud_subscriber: release request non-2xx — injection stranded as delivered, message lost"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        injection_id = %inj.id,
                        agent_id = %agent_id,
                        error = %e,
                        "cloud_subscriber: release request failed — injection stranded as delivered, message lost"
                    );
                }
            }
        }
    }

    delivery_status::fetched(agent_id, delivered_count, &body.expired);
    AgentSyncOutcome::Ok
}
