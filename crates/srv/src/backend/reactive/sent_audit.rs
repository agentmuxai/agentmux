// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sender's half of a message's trail: the `sent` line written where a message is handed to this srv. The receiving
//! srv writes the line it writes today under the same message id (`msg_id`), and `audit.query` joins the two into one
//! row. A message another srv handed over (a peer, another channel, the cloud) has no `sent` line here: its sender wrote
//! it.

use super::types::{AuditLogEntry, InjectionRequest};
use super::{audit_hash, now_unix_millis};

/// `event_kind` of the line, and its `outcome`.
pub const SENT: &str = "sent";

/// The line for `req`, a message handed over by a caller on this computer. `None` unless it has an id and came
/// straight from a local caller (the `host` tier, not forwarded): the id is what the receiving end's line is joined by.
fn line(req: &InjectionRequest) -> Option<AuditLogEntry> {
    let id = req.request_id.clone().filter(|id| !id.is_empty())?;
    if req.delivery_tier.as_deref().unwrap_or("host") != "host" || req.forward_hops != 0 {
        return None;
    }
    Some(AuditLogEntry {
        timestamp: now_unix_millis(),
        source_agent: req.source_agent.clone(),
        target_agent: req.target_agent.clone(),
        block_id: String::new(),
        message_hash: audit_hash(&req.message),
        message_length: req.message.len(),
        request_id: id.clone(),
        outcome: Some(SENT.to_string()),
        event_kind: SENT.to_string(),
        tier: Some("host".to_string()),
        msg_id: Some(id),
        ..Default::default()
    })
}

/// Writes the `sent` line of `req` to the audit file, when it has one, after giving a request with no id the one it will
/// be delivered under. Called from `admit` once the message is accepted: after the replay check (which treats a signed
/// message with no id as one it cannot count, so it must see the request as it came) and the refusals of files and of an
/// empty reply, and before a hold, so a held message is joined to its later delivery. Not kept in the in-memory ring: that serves the deliveries, and a message would take two places in it.
pub fn record(req: &mut InjectionRequest) {
    req.request_id = Some(req.request_id.take().filter(|id| !id.is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
    if let Some(entry) = line(req) {
        super::audit_store::record(&entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(tier: Option<&str>, hops: u8, id: Option<&str>) -> InjectionRequest {
        InjectionRequest {
            target_agent: "opaz".into(),
            message: "hello".into(),
            source_agent: Some("loap".into()),
            delivery_tier: tier.map(str::to_string),
            forward_hops: hops,
            request_id: id.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_message_a_local_caller_handed_over_has_a_sent_line_under_its_id() {
        let e = line(&request(Some("host"), 0, Some("m-1"))).unwrap();
        assert_eq!((e.event_kind.as_str(), e.outcome.as_deref(), e.msg_id.as_deref(), e.request_id.as_str()), ("sent", Some("sent"), Some("m-1"), "m-1"));
        assert_eq!((e.source_agent.as_deref(), e.target_agent.as_str(), e.tier.as_deref(), e.message_length), (Some("loap"), "opaz", Some("host"), 5));
        assert!(line(&request(None, 0, Some("m-1"))).is_some(), "no tier means a local caller");
    }

    #[test]
    fn a_message_another_srv_handed_over_or_with_no_id_has_none() {
        for (tier, hops, id) in [(Some("channel"), 0, Some("m")), (Some("lan"), 0, Some("m")), (Some("wan"), 0, Some("m")), (Some("host"), 1, Some("m")), (Some("host"), 0, None), (Some("host"), 0, Some(""))] {
            assert!(line(&request(tier, hops, id)).is_none(), "{tier:?} {hops} {id:?}");
        }
    }
}
