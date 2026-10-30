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
    AuditLogEntry {
        timestamp: now_unix_millis(),
        source_agent: Some(rec.actor.label()),
        target_agent,
        block_id: rec.target.block_id.clone().unwrap_or_default(),
        message_hash: audit_hash(&rec.command),
        message_length: 0,
        success: rec.error.is_none(),
        error_message: rec.error.clone(),
        request_id: rec.request_id.clone(),
        outcome: Some(rec.command.clone()),
        reason: rec.target.subject.clone(),
        event_kind: "action".to_string(),
        evicted_block: None,
        evicted_agent: None,
        audit_source_uid: String::new(),
        lan_claimed_uid: String::new(),
        wan: None,
        cli_native: None,
        turn: None,
    }
}

/// The most action entries the in-memory ring holds, so the rest stays for other events.
const ACTIONS_IN_RING_MAX: usize = AUDIT_LOG_MAX / 2;

impl ReactiveHandler {
    /// Record one call. A call that names a pane is recorded against the agent
    /// running in it. Runs on the reply path, so a poisoned lock is recovered rather than
    /// allowed to stop the reply.
    pub fn audit_action(&self, rec: &ActionRecord) {
        let mut handler = self.inner.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut inner = handler.inner.lock().unwrap();
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
    fn the_panes_registration_names_the_target_not_an_agent_field_in_the_request() {
        let handler = ReactiveHandler::new();
        handler.inner.lock().unwrap().register_agent("victim", "blk-1", None).unwrap();
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
