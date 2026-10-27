// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Audit lines for messages agents send each other through the CLI's own
//! messaging (`backend::cli_native`), kept like the jekt lines: in the ring
//! behind `GET /agentmux/reactive/audit` and, for good, in the audit file. As
//! with jekts, a line holds the text's hash and length, not the text.

use super::handler::{Handler, ReactiveHandler};
use super::types::{AuditLogEntry, CliNativeAudit};
use super::{audit_hash, now_unix_millis};
use crate::backend::cli_native::reader::Event;

/// The longest preview kept.
const PREVIEW_CHARS: usize = 200;

/// The first line of `text`, cut to [`PREVIEW_CHARS`].
fn preview(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    match first.char_indices().nth(PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first.to_string(),
    }
}

fn entry(ev: &Event) -> AuditLogEntry {
    AuditLogEntry {
        timestamp: now_unix_millis(),
        source_agent: Some(ev.source.clone()),
        target_agent: ev.target.clone(),
        block_id: ev.block_id.clone(),
        message_hash: audit_hash(&ev.text),
        message_length: ev.text.len(),
        success: true,
        error_message: None,
        request_id: ev.msg_id.clone(),
        outcome: Some(ev.state.to_string()),
        reason: ev.reason.clone(),
        event_kind: "cli_native".to_string(),
        evicted_block: None,
        evicted_agent: None,
        audit_source_uid: String::new(),
        lan_claimed_uid: String::new(),
        wan: None,
        cli_native: Some(CliNativeAudit {
            seen_by: ev.seen_by.to_string(),
            identity: ev.identity.to_string(),
            claimed_name: ev.claimed_name.clone(),
            cli_to_name: ev.cli_to_name.clone(),
            sender_pid: ev.sender_pid,
            from_mode: ev.from_mode.clone(),
            preview: preview(&ev.text),
        }),
    }
}

impl Handler {
    pub(super) fn log_cli_native(&mut self, ev: &Event) {
        super::audit_store::push(&mut self.audit_log, super::AUDIT_LOG_MAX, entry(ev));
    }
}

impl ReactiveHandler {
    /// Audit one state of a CLI-native message.
    pub fn audit_cli_native(&self, ev: &Event) {
        self.inner.lock().unwrap().log_cli_native(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(text: &str) -> Event {
        Event {
            block_id: "b1".into(),
            msg_id: "m1".into(),
            state: "read",
            seen_by: "receiver",
            source: "alpha".into(),
            identity: "agent",
            target: "beta".into(),
            claimed_name: Some("alpha-7".into()),
            cli_to_name: None,
            sender_pid: Some(101),
            from_mode: Some("bypass".into()),
            text: text.into(),
            reason: None,
        }
    }

    #[test]
    fn the_line_holds_a_hash_a_length_and_a_preview_never_the_text() {
        let e = entry(&ev("\n  first line here\nsecond line with a secret"));
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("second line"), "{json}");
        assert_eq!(e.event_kind, "cli_native");
        assert_eq!((e.request_id.as_str(), e.outcome.as_deref(), e.message_length), ("m1", Some("read"), 44));
        let c = e.cli_native.unwrap();
        assert_eq!((c.preview.as_str(), c.identity.as_str(), c.sender_pid), ("first line here", "agent", Some(101)));
    }

    #[test]
    fn a_long_first_line_is_cut() {
        let p = preview(&"é".repeat(500));
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn the_handlers_ring_and_route_see_the_line() {
        let handler = ReactiveHandler::new();
        handler.audit_cli_native(&ev("hello"));
        let log = handler.get_audit_log(1);
        assert_eq!((log[0].event_kind.as_str(), log[0].source_agent.as_deref(), log[0].target_agent.as_str()), ("cli_native", Some("alpha"), "beta"));
    }
}
