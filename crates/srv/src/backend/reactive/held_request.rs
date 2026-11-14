// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A held jekt row turned back into the request it was accepted as, for the replay (`server/jekt_held.rs`, which
//! re-exports it). It reads only the row and the ack policy, so the reactive tests reach it from the core.

use crate::backend::reactive::types::{InjectionRequest, JektTier};
use crate::backend::storage::jekt_held::HeldJekt;

/// The request a held row is delivered as: addressed by the target's UID,
/// with the verdicts it was accepted with — never re-verified (a signature
/// is valid for five minutes) and never raised — and its original send time.
pub(crate) fn request_from_held(row: &HeldJekt) -> InjectionRequest {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let mut req = InjectionRequest {
        target_agent: row.target_uid.clone(),
        // The row keeps the name the sender addressed; the marker says it
        // rather than the UID the replay routes by (#31433).
        marker_target: (!row.target_agent.is_empty() && row.target_agent != row.target_uid)
            .then(|| crate::backend::reactive::sanitize::route_name(&row.target_agent))
            .flatten(),
        message: row.message.clone(),
        source_agent: non_empty(&row.source_agent),
        request_id: Some(row.request_id.clone()),
        priority: non_empty(&row.priority),
        jekt_tier: non_empty(&row.jekt_tier)
            .and_then(|t| serde_json::from_value::<JektTier>(serde_json::Value::String(t)).ok()),
        // Never raised: a message from another machine keeps the tier and proof it was accepted with.
        delivery_tier: Some(if matches!(row.delivery_tier.as_str(), "lan" | "wan") { row.delivery_tier.clone() } else { "host".to_string() }),
        ..Default::default()
    };
    if row.delivery_tier == "wan" {
        let wan: serde_json::Value = serde_json::from_str(&row.wan_json).unwrap_or_default();
        req.wan_instance = serde_json::from_value(wan["instance"].clone()).ok();
        req.wan_verified = (wan["verified"] == true && req.wan_instance.is_some()).then_some(true);
    }
    req.sig_verified = row.sig_verified;
    // Not restored: reagent verification is WAN-only and no held row carries a
    // real reagent verdict (a verified WAN peer is held on its `wan_verified`
    // verdict, restored above), and a replayed `Some(true)` without its key id
    // would read as a failed verification.
    req.reagent_verified = None;
    req.lan_verified = row.lan_verified;
    req.channel_verified = row.channel_verified;
    req.is_transcript_request = row.is_transcript_request;
    req.transcript_request_escalate_forced = row.transcript_request_escalate_forced;
    req.audit_source_uid = row.audit_source_uid.clone();
    req.held_sent_at_ms = Some(row.sent_at_ms);
    // The sender's expiry rides again, so a delivery that ends up waiting
    // once more (the controller's startup queue, a re-hold) still drops the
    // message at its own deadline rather than delivering it stale.
    req.expires_at_ms = row.sender_expires_at_ms;
    // The row has no `reply` column; srv remembers an hour of `reply: none` ids, which covers most holds.
    req.reply = crate::backend::ack_policy::asked_for_no_reply(&row.request_id).then(|| "none".to_string());
    req.files = (!row.files_json.is_empty())
        .then(|| serde_json::from_str(&row.files_json).ok())
        .flatten();
    req
}
