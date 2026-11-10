// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The typed audit fields of a sensitive-op decision
//! (`REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md` §4.6): the dialog's text, the exact answer, its
//! risk level, and whether a remembered grant allowed it rather than a fresh countdown. Split out
//! of `handler.rs` (over the file-size baseline) the same way `action_audit.rs` and
//! `cli_native_audit.rs` already are: its own file, built on `Handler`'s fields the way those are.

use super::handler::{Handler, ReactiveHandler};
use super::types::AuditLogEntry;
use super::{audit_hash, now_unix_millis, AUDIT_LOG_MAX};

impl Handler {
    /// [`log_audit`](Handler::log_audit) plus the typed fields a sensitive-op decision has: the
    /// dialog's text, the exact answer, its risk level, and whether a remembered grant allowed it
    /// rather than a fresh countdown.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn log_audit_sensitive(
        &mut self,
        source_agent: Option<&str>,
        target_agent: &str,
        action: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        reason: Option<&str>,
        prompt: Option<&str>,
        answer: Option<&str>,
        level: Option<&str>,
        standing: bool,
    ) {
        self.log_audit_full(
            source_agent, target_agent, "", action, success, error_message, request_id, None, reason, prompt, answer, level,
            Some(standing),
        );
    }

    /// [`log_audit`](Handler::log_audit) and [`log_audit_sensitive`](Handler::log_audit_sensitive)'s
    /// shared constructor: `prompt`, `answer`, `level` and `standing` are `None` from the former.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn log_audit_full(
        &mut self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        message: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        outcome: Option<&str>,
        reason: Option<&str>,
        prompt: Option<&str>,
        answer: Option<&str>,
        level: Option<&str>,
        standing: Option<bool>,
    ) {
        let entry = AuditLogEntry {
            timestamp: now_unix_millis(),
            source_agent: source_agent.map(|s| s.to_string()),
            target_agent: target_agent.to_string(),
            block_id: block_id.to_string(),
            message_hash: audit_hash(message),
            message_length: message.len(),
            success,
            error_message: error_message.map(|s| s.to_string()),
            request_id: request_id.to_string(),
            outcome: outcome.map(|s| s.to_string()),
            reason: reason.map(|s| s.to_string()),
            event_kind: "delivery".to_string(),
            evicted_block: None,
            evicted_agent: None,
            audit_source_uid: self.delivery_source_uid.clone(),
            lan_claimed_uid: self.delivery_lan_claimed_uid.clone(),
            wan: self.delivery_wan.clone(),
            cli_native: None,
            turn: self.delivery_turn.take().map(str::to_string),
            tier: self.delivery_how.as_ref().map(|(tier, _)| tier.clone()),
            trust: self.delivery_how.as_ref().map(|(_, trust)| trust.clone()),
            msg_id: Some(request_id.to_string()).filter(|id| !id.is_empty()),
            prompt: prompt.map(|s| s.to_string()),
            answer: answer.map(|s| s.to_string()),
            level: level.map(|s| s.to_string()),
            standing,
            ..Default::default()
        };

        super::audit_store::push(&mut self.audit_log, AUDIT_LOG_MAX, entry);
    }
}

impl ReactiveHandler {
    /// [`log_fleet_action_audit`](ReactiveHandler::log_fleet_action_audit) for a sensitive-op
    /// decision (`sagas::sensitive_window`, `server::browser_permission`): the same, plus the
    /// dialog's text, the exact answer, its risk level, and whether a remembered grant allowed it
    /// rather than a fresh countdown.
    #[allow(clippy::too_many_arguments)]
    pub fn log_sensitive_op_audit(
        &self,
        source_agent: Option<&str>,
        target_agent: &str,
        action: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        reason: Option<&str>,
        prompt: Option<&str>,
        answer: Option<&str>,
        level: Option<&str>,
        standing: bool,
    ) {
        self.inner.lock().unwrap().log_audit_sensitive(
            source_agent, target_agent, action, success, error_message, request_id, reason, prompt, answer, level, standing,
        );
    }
}
