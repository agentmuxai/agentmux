// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::sanitize::{format_injected_message, sanitize_message, validate_agent_id};
use super::types::*;
use super::{now_unix_millis, sha256_hex, AUDIT_LOG_MAX, RATE_LIMIT_MAX};

// ---- Rate Limiter ----

pub(super) struct RateLimiter {
    tokens: u32,
    max_tokens: u32,
    last_refill: Instant,
}

impl RateLimiter {
    pub(super) fn new(max_tokens: u32) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill: Instant::now(),
        }
    }

    pub(super) fn check(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        if elapsed >= Duration::from_secs(1) {
            self.tokens = self.max_tokens;
            self.last_refill = now;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

// ---- Handler ----

/// Core reactive messaging handler.
///
/// Manages agent registrations, rate limiting, message injection,
/// and audit logging.
pub struct Handler {
    agent_to_block: HashMap<String, String>,
    block_to_agent: HashMap<String, String>,
    agent_info: HashMap<String, AgentRegistration>,
    input_sender: Option<InputSender>,
    audit_log: Vec<AuditLogEntry>,
    rate_limiter: RateLimiter,
    include_source_in_message: bool,
}

impl Handler {
    /// Create a new handler without an input sender.
    /// Call `set_input_sender` before injecting messages.
    pub fn new() -> Self {
        Self {
            agent_to_block: HashMap::new(),
            block_to_agent: HashMap::new(),
            agent_info: HashMap::new(),
            input_sender: None,
            audit_log: Vec::with_capacity(AUDIT_LOG_MAX),
            rate_limiter: RateLimiter::new(RATE_LIMIT_MAX),
            include_source_in_message: false,
        }
    }

    /// Set the input sender function for message injection.
    pub fn set_input_sender(&mut self, sender: InputSender) {
        self.input_sender = Some(sender);
    }

    /// Set whether to include source agent prefix in injected messages.
    pub fn set_include_source(&mut self, include: bool) {
        self.include_source_in_message = include;
    }

    /// Register an agent with a block.
    pub fn register_agent(
        &mut self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
    ) -> Result<(), String> {
        if !validate_agent_id(agent_id) {
            return Err(format!("invalid agent ID: {}", agent_id));
        }

        // Remove existing registration for this agent
        if let Some(old_block) = self.agent_to_block.remove(agent_id) {
            self.block_to_agent.remove(&old_block);
        }

        // Remove existing registration for this block
        if let Some(old_agent) = self.block_to_agent.remove(block_id) {
            self.agent_to_block.remove(&old_agent);
            self.agent_info.remove(&old_agent);
        }

        let now = now_unix_millis();
        self.agent_to_block
            .insert(agent_id.to_string(), block_id.to_string());
        self.block_to_agent
            .insert(block_id.to_string(), agent_id.to_string());
        self.agent_info.insert(
            agent_id.to_string(),
            AgentRegistration {
                agent_id: agent_id.to_string(),
                block_id: block_id.to_string(),
                tab_id: tab_id.map(|s| s.to_string()),
                registered_at: now,
                last_seen: now,
            },
        );

        Ok(())
    }

    /// Unregister an agent.
    pub fn unregister_agent(&mut self, agent_id: &str) {
        if let Some(block_id) = self.agent_to_block.remove(agent_id) {
            self.block_to_agent.remove(&block_id);
        }
        self.agent_info.remove(agent_id);
    }

    /// Unregister by block ID.
    pub fn unregister_block(&mut self, block_id: &str) {
        if let Some(agent_id) = self.block_to_agent.remove(block_id) {
            self.agent_to_block.remove(&agent_id);
            self.agent_info.remove(&agent_id);
        }
    }

    /// Update the last_seen timestamp for an agent.
    pub fn update_last_seen(&mut self, agent_id: &str) {
        if let Some(info) = self.agent_info.get_mut(agent_id) {
            info.last_seen = now_unix_millis();
        }
    }

    /// Get agent registration by agent ID.
    pub fn get_agent(&self, agent_id: &str) -> Option<&AgentRegistration> {
        self.agent_info.get(agent_id)
    }

    /// Get agent registration by block ID.
    pub fn get_agent_by_block(&self, block_id: &str) -> Option<&AgentRegistration> {
        self.block_to_agent
            .get(block_id)
            .and_then(|agent_id| self.agent_info.get(agent_id))
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentRegistration> {
        self.agent_info.values().cloned().collect()
    }

    /// Inject a message into an agent's terminal.
    ///
    /// Sends `message\r` as a single payload (required for text display),
    /// then spawns 3 delayed `\r` sends at 200ms intervals as separate
    /// PTY writes to ensure submission. See `specs/jekt-inject-timing.md`.
    pub fn inject_message(&mut self, mut req: InjectionRequest) -> InjectionResponse {
        let now = now_unix_millis();

        // Generate request ID if missing
        if req.request_id.is_none() || req.request_id.as_deref() == Some("") {
            req.request_id = Some(uuid::Uuid::new_v4().to_string());
        }
        let request_id = req.request_id.clone().unwrap_or_default();

        // Rate limit check
        if !self.rate_limiter.check() {
            return InjectionResponse {
                success: false,
                request_id,
                block_id: None,
                error: Some("rate limit exceeded".to_string()),
                timestamp: now,
            };
        }

        // Validate agent ID
        if !validate_agent_id(&req.target_agent) {
            return InjectionResponse {
                success: false,
                request_id,
                block_id: None,
                error: Some(format!("invalid agent ID: {}", req.target_agent)),
                timestamp: now,
            };
        }

        // Sanitize message
        let sanitized = sanitize_message(&req.message);

        // Look up block ID
        let block_id = match self.agent_to_block.get(&req.target_agent) {
            Some(id) => id.clone(),
            None => {
                let err = format!("agent not found: {}", req.target_agent);
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    "",
                    &sanitized,
                    false,
                    Some(&err),
                    &request_id,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: None,
                    error: Some(err),
                    timestamp: now,
                };
            }
        };

        // Format message with source prefix if configured
        let final_msg = format_injected_message(
            &sanitized,
            req.source_agent.as_deref(),
            self.include_source_in_message,
        );

        // Send message via input sender
        let sender = match &self.input_sender {
            Some(s) => s.clone(),
            None => {
                let err = "input sender not configured".to_string();
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    &block_id,
                    &sanitized,
                    false,
                    Some(&err),
                    &request_id,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: Some(block_id),
                    error: Some(err),
                    timestamp: now,
                };
            }
        };

        // Jekt inject sequence (see specs/jekt-inject-timing.md):
        // 1. \r to clear any partial input on the line
        // 2. message\r as single payload (proven to display text — v0.31.122/125)
        // 3. Three delayed \r at 200ms intervals as separate PTY writes to submit
        let _ = sender(&block_id, b"\r");
        let payload = format!("{}\r", final_msg);
        tracing::info!(
            target_agent = %req.target_agent,
            block_id = %block_id,
            msg_len = payload.len(),
            "inject: sending payload to PTY"
        );
        if let Err(e) = sender(&block_id, payload.as_bytes()) {
            tracing::error!(
                target_agent = %req.target_agent,
                block_id = %block_id,
                error = %e,
                "inject: sender failed"
            );
            self.log_audit(
                req.source_agent.as_deref(),
                &req.target_agent,
                &block_id,
                &sanitized,
                false,
                Some(&e),
                &request_id,
            );
            return InjectionResponse {
                success: false,
                request_id,
                block_id: Some(block_id),
                error: Some(e),
                timestamp: now,
            };
        }

        // Spawn 3 delayed \r sends as separate PTY events to ensure submission.
        let sender_enter = sender.clone();
        let block_id_enter = block_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
        });

        // Success
        self.log_audit(
            req.source_agent.as_deref(),
            &req.target_agent,
            &block_id,
            &sanitized,
            true,
            None,
            &request_id,
        );

        InjectionResponse {
            success: true,
            request_id,
            block_id: Some(block_id),
            error: None,
            timestamp: now,
        }
    }

    /// Get audit log entries, most recent first.
    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditLogEntry> {
        let start = if self.audit_log.len() > limit {
            self.audit_log.len() - limit
        } else {
            0
        };
        let mut entries: Vec<_> = self.audit_log[start..].to_vec();
        entries.reverse();
        entries
    }

    /// Add an entry to the audit ring buffer.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn log_audit(
        &mut self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        message: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
    ) {
        let entry = AuditLogEntry {
            timestamp: now_unix_millis(),
            source_agent: source_agent.map(|s| s.to_string()),
            target_agent: target_agent.to_string(),
            block_id: block_id.to_string(),
            message_hash: sha256_hex(message),
            message_length: message.len(),
            success,
            error_message: error_message.map(|s| s.to_string()),
            request_id: request_id.to_string(),
        };

        if self.audit_log.len() >= AUDIT_LOG_MAX {
            self.audit_log.remove(0);
        }
        self.audit_log.push(entry);
    }
}

impl Default for Handler {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Thread-safe wrapper ----

/// Thread-safe wrapper around Handler.
pub struct ReactiveHandler {
    inner: Mutex<Handler>,
}

impl ReactiveHandler {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Handler::new()),
        }
    }

    pub fn set_input_sender(&self, sender: InputSender) {
        self.inner.lock().unwrap().set_input_sender(sender);
    }

    pub fn set_include_source(&self, include: bool) {
        self.inner.lock().unwrap().set_include_source(include);
    }

    pub fn register_agent(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .register_agent(agent_id, block_id, tab_id)
    }

    pub fn unregister_agent(&self, agent_id: &str) {
        self.inner.lock().unwrap().unregister_agent(agent_id);
    }

    pub fn unregister_block(&self, block_id: &str) {
        self.inner.lock().unwrap().unregister_block(block_id);
    }

    pub fn update_last_seen(&self, agent_id: &str) {
        self.inner.lock().unwrap().update_last_seen(agent_id);
    }

    pub fn get_agent(&self, agent_id: &str) -> Option<AgentRegistration> {
        self.inner.lock().unwrap().get_agent(agent_id).cloned()
    }

    pub fn get_agent_by_block(&self, block_id: &str) -> Option<AgentRegistration> {
        self.inner
            .lock()
            .unwrap()
            .get_agent_by_block(block_id)
            .cloned()
    }

    pub fn list_agents(&self) -> Vec<AgentRegistration> {
        self.inner.lock().unwrap().list_agents()
    }

    pub fn inject_message(&self, req: InjectionRequest) -> InjectionResponse {
        self.inner.lock().unwrap().inject_message(req)
    }

    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditLogEntry> {
        self.inner.lock().unwrap().get_audit_log(limit)
    }
}

impl Default for ReactiveHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Global reactive handler singleton.
static GLOBAL_HANDLER: OnceLock<ReactiveHandler> = OnceLock::new();

/// Get or initialize the global reactive handler.
pub fn get_global_handler() -> &'static ReactiveHandler {
    GLOBAL_HANDLER.get_or_init(ReactiveHandler::new)
}
