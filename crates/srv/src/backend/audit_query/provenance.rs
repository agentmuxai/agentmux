// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a jekt row came from and how it travelled (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION §3.6), and the join of a
//! message's two halves: the sender's `sent` line and the receiver's line, which carry one `msg_id`.

use std::collections::HashMap;

use crate::backend::reactive::sent_audit::SENT;
use crate::backend::reactive::types::AuditLogEntry;
use crate::backend::rpc_types::{AuditEntry, AuditProvenance};

use super::KIND_JEKT;

/// The provenance of a line, or none for a line that says nothing of it and proves nothing. A line from before the
/// fields existed still has its tier where the line itself proves it: a WAN verdict is the cloud, a LAN-claimed UID is
/// the LAN. That is marked `tier_inferred`; any other old line has no tier.
pub(super) fn of(e: &AuditLogEntry) -> Option<AuditProvenance> {
    let (tier, inferred) = match e.tier.as_deref() {
        Some(t) => (Some(named(t)), false),
        None if e.wan.is_some() => (Some("cloud".to_string()), true),
        None if !e.lan_claimed_uid.is_empty() => (Some("lan".to_string()), true),
        None => (None, false),
    };
    let provenance = AuditProvenance {
        host: e.host.clone(),
        channel: e.channel.clone(),
        version: e.version.clone(),
        tier,
        tier_inferred: inferred.then_some(true),
        trust: e.trust.clone(),
        msg_id: e.msg_id.clone(),
        sent_ms: None,
        sent_only: (e.event_kind == SENT).then_some(true),
    };
    (provenance != AuditProvenance::default()).then_some(provenance)
}

/// A tier as the Audit names it: the handler's `wan` is the cloud.
fn named(tier: &str) -> String {
    if tier == "wan" { "cloud" } else { tier }.to_string()
}

/// The message id of a jekt entry, if it has one.
fn msg_id(e: &AuditEntry) -> Option<&str> {
    e.provenance.as_ref().and_then(|p| p.msg_id.as_deref()).filter(|_| e.kind == KIND_JEKT)
}

fn is_sent_only(e: &AuditEntry) -> bool {
    e.provenance.as_ref().is_some_and(|p| p.sent_only == Some(true))
}

fn is_joined(e: &AuditEntry) -> bool {
    e.provenance.as_ref().is_some_and(|p| p.sent_ms.is_some())
}

/// The rows of a page and where each message id was first seen in it, so the halves of one message come out as one row.
#[derive(Default)]
pub(super) struct Joiner {
    seen: HashMap<String, usize>,
}

impl Joiner {
    /// Whether `next` is the sender's half of a message the page already holds the other half of: it goes in the page
    /// even when the page is full, so a page boundary does not split a message.
    pub(super) fn completes(&self, next: &AuditEntry) -> bool {
        is_sent_only(next) && msg_id(next).is_some_and(|id| self.seen.contains_key(id))
    }

    /// Adds `entry` to `entries`, joined into the row of the same message when one of the two is the sender's `sent`
    /// line and the other is not yet joined. The joined row is the receiver's, with the time the sender handed the
    /// message over; a `sent` line alone stays a row of its own, marked as such. Rows of one message that are both the
    /// receiver's (a hold and its delivery) stay separate rows, and the `sent` line joins the newest.
    pub(super) fn push(&mut self, entries: &mut Vec<AuditEntry>, entry: AuditEntry) {
        let Some(id) = msg_id(&entry).map(str::to_string) else {
            entries.push(entry);
            return;
        };
        let Some(&at) = self.seen.get(&id) else {
            self.seen.insert(id, entries.len());
            entries.push(entry);
            return;
        };
        if is_sent_only(&entry) == is_sent_only(&entries[at]) || is_joined(&entries[at]) {
            entries.push(entry);
            return;
        }
        let (mut receiver, sent_ms) = if is_sent_only(&entry) { (entries[at].clone(), entry.ts_ms) } else { (entry, entries[at].ts_ms) };
        if let Some(p) = receiver.provenance.as_mut() {
            p.sent_ms = Some(sent_ms);
        }
        entries[at] = receiver;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::AuditLinks;

    fn row(id: &str, ts: u64, msg: Option<&str>, sent: bool) -> AuditEntry {
        AuditEntry {
            id: id.into(),
            ts_ms: ts,
            kind: KIND_JEKT.into(),
            agent_id: None,
            from: None,
            to: None,
            summary: String::new(),
            outcome: None,
            reason: None,
            cost: None,
            links: AuditLinks::default(),
            provenance: msg.map(|m| AuditProvenance { msg_id: Some(m.into()), sent_only: sent.then_some(true), ..Default::default() }),
        }
    }

    fn page(rows: Vec<AuditEntry>) -> Vec<AuditEntry> {
        let (mut joiner, mut out) = (Joiner::default(), Vec::new());
        for r in rows {
            joiner.push(&mut out, r);
        }
        out
    }

    #[test]
    fn the_two_halves_of_a_message_are_one_row_whichever_comes_first() {
        // Newest first: the receiver's line then the sender's, or (a peer's clock ahead) the other way round.
        for rows in [vec![row("d", 20, Some("m"), false), row("s", 10, Some("m"), true)], vec![row("s", 30, Some("m"), true), row("d", 20, Some("m"), false)]] {
            let sent_at = rows.iter().find(|r| r.id == "s").unwrap().ts_ms;
            let out = page(rows);
            assert_eq!(out.len(), 1);
            assert_eq!((out[0].id.as_str(), out[0].provenance.as_ref().unwrap().sent_ms), ("d", Some(sent_at)));
            assert!(out[0].provenance.as_ref().unwrap().sent_only.is_none());
        }
    }

    #[test]
    fn a_sent_line_with_no_other_end_stays_its_own_row_and_other_messages_are_not_touched() {
        let out = page(vec![row("a", 40, Some("x"), true), row("b", 30, Some("y"), false), row("c", 20, None, false)]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].provenance.as_ref().unwrap().sent_only, Some(true));
        assert!(out.iter().all(|e| e.provenance.as_ref().is_none_or(|p| p.sent_ms.is_none())));
    }

    #[test]
    fn a_hold_and_its_delivery_stay_two_rows_and_the_sent_line_joins_the_newest() {
        let out = page(vec![row("delivered", 30, Some("m"), false), row("held", 20, Some("m"), false), row("sent", 10, Some("m"), true)]);
        assert_eq!(out.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["delivered", "held"]);
        assert_eq!(out[0].provenance.as_ref().unwrap().sent_ms, Some(10));
        assert_eq!(out[1].provenance.as_ref().unwrap().sent_ms, None);
    }

    #[test]
    fn a_sent_line_completes_a_page_that_holds_its_other_half() {
        let (mut joiner, mut out) = (Joiner::default(), Vec::new());
        joiner.push(&mut out, row("d", 20, Some("m"), false));
        assert!(joiner.completes(&row("s", 10, Some("m"), true)));
        assert!(!joiner.completes(&row("s2", 10, Some("other"), true)));
        assert!(!joiner.completes(&row("d2", 10, Some("m"), false)));
    }

    #[test]
    fn old_lines_get_a_tier_only_where_the_line_proves_it() {
        let text = r#"{"timestamp":1,"target_agent":"a","block_id":"b","message_hash":"h","message_length":1,"success":true,"request_id":"r","channel":"dev"}"#;
        let mut e: AuditLogEntry = serde_json::from_str(text).unwrap();
        let p = of(&e).unwrap();
        assert_eq!((p.channel.as_deref(), p.tier.as_deref(), p.tier_inferred), (Some("dev"), None, None));
        e.lan_claimed_uid = "u".into();
        let p = of(&e).unwrap();
        assert_eq!((p.tier.as_deref(), p.tier_inferred), (Some("lan"), Some(true)));
        e.tier = Some("wan".into());
        let p = of(&e).unwrap();
        assert_eq!((p.tier.as_deref(), p.tier_inferred), (Some("cloud"), None));
    }
}
