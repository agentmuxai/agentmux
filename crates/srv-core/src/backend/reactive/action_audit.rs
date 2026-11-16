// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The audit line for a recorded RPC call (`backend::rpc::action_record`): kept
//! like the jekt lines, in the ring behind `GET /agentmux/reactive/audit` and in
//! the audit file. `event_kind` is `action`; `outcome` is the command;
//! `source_agent` is the actor (`agent:<id>`, `operator` or `unproven`);
//! `target_agent` and `block_id` are what it acted on; `reason` names the
//! identifier it acted on; `success` and `error_message` say how it went.

use super::handler::ReactiveHandler;
use super::types::AuditLogEntry;
use super::{audit_hash, now_unix_millis, AUDIT_LOG_MAX};
use crate::backend::rpc::action_record::ActionRecord;

fn entry(rec: &ActionRecord, target_agent: String) -> AuditLogEntry {
    action_entry(
        Some(rec.actor.label()),
        target_agent,
        rec.target.block_id.clone().unwrap_or_default(),
        &rec.command,
        rec.error.is_none(),
        rec.error.clone(),
        &rec.request_id,
        rec.target.subject.clone(),
    )
}

/// One action row: `outcome` is the command, `reason` what it acted on or the caller's note, and no message
/// length, so the Audit view shows no byte count or token estimate for it.
fn action_entry(
    source_agent: Option<String>,
    target_agent: String,
    block_id: String,
    command: &str,
    success: bool,
    error: Option<String>,
    request_id: &str,
    reason: Option<String>,
) -> AuditLogEntry {
    AuditLogEntry {
        timestamp: now_unix_millis(),
        source_agent,
        target_agent,
        block_id,
        message_hash: audit_hash(command),
        message_length: 0,
        success,
        error_message: error,
        request_id: request_id.to_string(),
        outcome: Some(command.to_string()),
        reason,
        event_kind: "action".to_string(),
        evicted_block: None,
        evicted_agent: None,
        audit_source_uid: String::new(),
        lan_claimed_uid: String::new(),
        wan: None,
        cli_native: None,
        turn: None,
        host: None,
        channel: None,
        version: None,
        tier: None,
        trust: None,
        msg_id: None,
        prompt: None,
        answer: None,
        level: None,
        standing: None,
    }
}

/// The most action entries the in-memory ring holds, so the rest stays for other events.
const ACTIONS_IN_RING_MAX: usize = AUDIT_LOG_MAX / 2;

impl ReactiveHandler {
    /// Record one call. A call that names a pane is recorded against the agent
    /// running in it. Runs on the reply path, so a poisoned lock is recovered rather than
    /// allowed to stop the reply.
    pub fn audit_action(&self, rec: &ActionRecord) {
        let mut handler = self.inner.lock();
        // A pane's own registration says whom it acts on; an agent field in the request is only a
        // claim (a command may ignore it), so it is used when no pane is named or registered.
        let target = rec
            .target
            .block_id
            .as_deref()
            .and_then(|b| handler.names_for_block(b).into_iter().next())
            .or_else(|| rec.target.agent.clone())
            .unwrap_or_default();
        // Calls come in bursts; they may not push the deliveries and registrations out of the
        // ring that `GET /agentmux/reactive/audit` serves. The audit file keeps every one.
        let ring = &mut handler.audit_log;
        if ring.iter().filter(|e| e.event_kind == "action").count() >= ACTIONS_IN_RING_MAX {
            if let Some(oldest) = ring.iter().position(|e| e.event_kind == "action") {
                ring.remove(oldest);
            }
        }
        super::audit_store::push(ring, AUDIT_LOG_MAX, entry(rec, target));
    }

    /// Record a fleet action (an AWS credential grant, a seat change, a rename, a pane close) written by
    /// [`ReactiveHandler::log_fleet_action_audit`]: an action row, like a recorded RPC call, with the caller's own
    /// `source_agent` (a plain agent id, or `you` for the person) and the target it already resolved.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn audit_fleet_action(
        &self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        action: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        reason: Option<&str>,
    ) {
        let e = action_entry(
            source_agent.map(str::to_string),
            target_agent.to_string(),
            block_id.to_string(),
            action,
            success,
            error_message.map(str::to_string),
            request_id,
            reason.map(str::to_string),
        );
        let mut handler = self.inner.lock();
        let ring = &mut handler.audit_log;
        if ring.iter().filter(|e| e.event_kind == "action").count() >= ACTIONS_IN_RING_MAX {
            if let Some(oldest) = ring.iter().position(|e| e.event_kind == "action") {
                ring.remove(oldest);
            }
        }
        super::audit_store::push(ring, AUDIT_LOG_MAX, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc::action_record::Target;
    use crate::backend::rpc::actor::Actor;

    fn rec(i: usize) -> ActionRecord {
        ActionRecord { actor: Actor::Operator, command: format!("c{i}"), target: Target::default(), request_id: i.to_string(), error: None }
    }

    #[test]
    fn a_burst_of_calls_does_not_push_the_other_events_out_of_the_ring() {
        let handler = ReactiveHandler::new();
        {
            let mut inner = handler.inner.lock();
            for i in 0..AUDIT_LOG_MAX {
                let mut e = entry(&rec(i), String::new());
                e.event_kind = "delivery".to_string();
                super::super::audit_store::push(&mut inner.audit_log, AUDIT_LOG_MAX, e);
            }
        }
        for i in 0..300 {
            handler.audit_action(&rec(i));
        }
        let log = handler.get_audit_log(AUDIT_LOG_MAX);
        let actions = log.iter().filter(|e| e.event_kind == "action").count();
        assert_eq!(log.len(), AUDIT_LOG_MAX);
        assert_eq!(actions, ACTIONS_IN_RING_MAX);
        assert_eq!(log[0].outcome.as_deref(), Some("c299"), "the newest call is kept");
    }

    #[test]
    fn a_fleet_action_is_one_action_row_with_the_callers_note_and_no_message_length() {
        let handler = ReactiveHandler::new();
        handler.log_fleet_action_audit(Some("korp"), "opaz", "blk-9", "agent.rename", true, None, "req-1", Some("renamed Pulse #2"));
        handler.log_fleet_action_audit(Some("you"), "opaz", "blk-9", "agent.spend", false, Some("refused"), "req-2", None);
        let log = handler.get_audit_log(2);
        let (refused, done) = (&log[0], &log[1]);
        assert!(log.iter().all(|e| e.event_kind == "action" && e.message_length == 0));
        assert_eq!((done.source_agent.as_deref(), done.target_agent.as_str(), done.block_id.as_str()), (Some("korp"), "opaz", "blk-9"));
        assert_eq!((done.outcome.as_deref(), done.reason.as_deref(), done.success, done.request_id.as_str()), (Some("agent.rename"), Some("renamed Pulse #2"), true, "req-1"));
        assert_eq!((refused.success, refused.error_message.as_deref(), refused.source_agent.as_deref()), (false, Some("refused"), Some("you")));
    }

    #[test]
    fn the_panes_registration_names_the_target_not_an_agent_field_in_the_request() {
        let handler = ReactiveHandler::new();
        handler.inner.lock().register_agent("victim", "blk-1", None).unwrap();
        let mut r = rec(1);
        r.target = Target { agent: Some("someone-else".into()), block_id: Some("blk-1".into()), subject: None };
        handler.audit_action(&r);
        let mut r = rec(2);
        r.target = Target { agent: Some("named".into()), block_id: Some("unregistered".into()), subject: None };
        handler.audit_action(&r);
        let log = handler.get_audit_log(2);
        assert_eq!(log[1].target_agent, "victim");
        assert_eq!(log[0].target_agent, "named", "an unregistered pane falls back to the named agent");
    }
}
