// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claim → verify → deliver for one agent's pulled injections: the half of a
//! pull that comes after the relay answered, shared by the single pull
//! (`sync_agent_reactive`) and the batch pull (`sync_agents_batched`).

use super::*;

#[path = "delivery_steps.rs"]
mod steps;

#[path = "reagent_age.rs"]
mod reagent_age;

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
    // The sender's delivery deadline (RFC 3339), when one is carried
    // (issue #3894): a short-lived notice is dropped, not delivered stale,
    // if srv only gets to it after this. Optional: a sender that doesn't set
    // it, or an older peer that doesn't send it, behaves exactly as before.
    #[serde(default)]
    expires_at: Option<String>,
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
    // Files sent with it, on the relay (server::jekt_cloud): the manifest as
    // the sender signed it. Absent on a text jekt and from an older relay.
    #[serde(default)]
    files: Option<Vec<agentmux_common::jekt_files::CloudFile>>,
    // The tier the sender declared (info|coord|sensitive), as the relay
    // passed it on. Absent when none was declared and from an older relay.
    #[serde(default)]
    jekt_tier: Option<String>,
}

/// The sender's declared tier as the relay carries it. The tier is not part
/// of the signed material, so only `sensitive` is taken: a carried tier can
/// mark a message sensitive, never lower it below the default `coord`. The
/// handler's trust rules apply on top as for any jekt.
fn declared_tier(raw: Option<&str>) -> Option<crate::backend::reactive::types::JektTier> {
    (raw == Some("sensitive")).then_some(crate::backend::reactive::types::JektTier::Sensitive)
}

/// What is noted when a relay message arrives: it is remembered (a bare "got it" back is refused), and,
/// as it shows the sender is reachable, a message queued to it is no longer watched for a reply.
fn note_arrival(mstore: &Store, req: &InjectionRequest) {
    crate::server::jekt_ack::remember_arrival(mstore, req);
    crate::muxbus::relay_watch::heard_from(req.source_agent.as_deref(), &req.target_agent);
}

/// The request a pulled injection is delivered as, before the signature
/// checks below add their verdicts.
fn wan_request(inj: &PendingInj, target: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: target.to_string(),
        message: inj.message.clone(),
        source_agent: inj.source_agent.clone(),
        request_id: Some(inj.id.clone()),
        priority: inj.priority.clone(),
        // Keyword scanning in the handler still applies on top.
        jekt_tier: declared_tier(inj.jekt_tier.as_deref()),
        // The sender's deadline, when the relay passed it on: what the hold
        // and the deferred queue then respect (issue #3894).
        expires_at_ms: inj.expires_at.as_deref().and_then(delivery_status::notes::parse_created_at),
        delivery_tier: Some("wan".to_string()),
        forward_hops: 0,
        reagent_key_id: inj.reagent_key_id.clone(),
        ..Default::default()
    }
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
    agent_token: String,
    using_per_agent: bool,
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
    let (claimed, agent_token) = match steps::claim(base, agent_id, token, http, mstore, &all_ids, agent_token, using_per_agent).await {
        Ok(claimed) => claimed,
        Err(outcome) => return outcome,
    };

    let delivered_at = claimed.delivered_at;
    let claimed_ids: std::collections::HashSet<String> = claimed.acknowledged.into_iter().collect();
    let mut delivered_count = 0usize;
    // A name the subscription holds through the pane's agent is delivered to the pane.
    let target = super::live_names::delivery_target(handler, agent_id).await;

    for inj in &body.injections {
        if !claimed_ids.contains(&inj.id) {
            // Another concurrent poller (this account's other channel/seat)
            // already won this claim — expected under the "two seats"
            // workflow, not an error.
            continue;
        }

        let checked = steps::verify(inj, agent_id, token, http, mstore).await;
        if steps::is_repeat(inj, agent_id, &checked) {
            continue;
        }
        let mut req = steps::request(inj, &target, &checked, mstore);
        if steps::attach_files(base, http, &agent_token, agent_id, inj, &mut req, &delivered_at).await.is_err() {
            continue;
        }
        // Remembered, so a bare "got it" back to this sender is refused (`backend::ack_policy`); the relay
        // doesn't carry `reply` yet, so a WAN message never arrives marked `REPLY=none`.
        note_arrival(mstore, &req);
        // Verified, and its agent can't work now: kept here until it can. Injected now it would be lost in
        // a failed turn, or given back to the relay to come again. Or a routine message to a session gone
        // cold, which waits for its next activity as a local or LAN one does (#2263).
        if crate::server::jekt_hold::hold_wan_for_unavailable(mstore, handler, &req) || crate::server::jekt_cold::hold_relayed(&req).await {
            delivered_count += 1;
            steps::record_delivered(&checked);
            continue;
        }
        let charged = req.files_delivery.is_some().then(|| req.clone());
        let delivery = handler.inject_message(req);
        if let Some(charged) = &charged {
            crate::server::jekt_files::settle(charged, delivery.success);
        }
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
            steps::record_delivered(&checked);
        }

        if !delivery.success {
            // We hold the claim but local delivery failed (e.g. agent not
            // ready) — release it back to pending so it's retried, instead
            // of silently dropping it now that claiming already marked it
            // "delivered".
            release(base, http, &agent_token, agent_id, &inj.id, &delivered_at).await;
        }
    }

    delivery_status::fetched(agent_id, delivered_count, &body.expired);
    AgentSyncOutcome::Ok
}

/// Give a claimed injection back to pending, so it's delivered on a later
/// pull. A release that fails strands it as delivered: logged loudly.
async fn release(base: &str, http: &reqwest::Client, agent_token: &str, agent_id: &str, injection_id: &str, delivered_at: &str) {
    let release_url = format!("{}/reactive/release", base);
    match http
        .post(&release_url)
        .header("Authorization", format!("Bearer {}", agent_token))
        .header("X-Agent-ID", agent_id)
        .json(&serde_json::json!({
            "injection_id": injection_id,
            "delivered_at": delivered_at,
        }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            // Claim is stranded as "delivered" with no local delivery and no
            // retry until this is visible — log loudly rather than discarding
            // silently.
            tracing::warn!(
                injection_id = %injection_id,
                agent_id = %agent_id,
                status = %r.status(),
                "cloud_subscriber: release request non-2xx — injection stranded as delivered, message lost"
            );
        }
        Err(e) => {
            tracing::warn!(
                injection_id = %injection_id,
                agent_id = %agent_id,
                error = %e,
                "cloud_subscriber: release request failed — injection stranded as delivered, message lost"
            );
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
