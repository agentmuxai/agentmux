// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::sanitize::{format_injected_message, is_sensitive_message, sanitize_message, validate_agent_id, wrap_jekt_message};
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
    /// Controller-aware delivery for non-PTY agents (persistent stream-json / ACP).
    /// When set, it is tried before the PTY keystroke path so messages reach (and
    /// steer) agents that have no terminal. See `set_message_sender`.
    message_sender: Option<MessageSender>,
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
            message_sender: None,
            audit_log: Vec::with_capacity(AUDIT_LOG_MAX),
            rate_limiter: RateLimiter::new(RATE_LIMIT_MAX),
            include_source_in_message: false,
        }
    }

    /// Set the input sender function for message injection.
    pub fn set_input_sender(&mut self, sender: InputSender) {
        self.input_sender = Some(sender);
    }

    /// Set the controller-aware message sender. When present, `inject_message`
    /// tries it first: persistent stream-json and ACP agents receive a structured
    /// message on their live channel (mid-turn steering); PTY-based agents report
    /// back so injection falls through to the keystroke path.
    pub fn set_message_sender(&mut self, sender: MessageSender) {
        self.message_sender = Some(sender);
    }

    /// Set whether to include source agent prefix in injected messages.
    #[allow(dead_code)]
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
        self.register_agent_generated(agent_id, block_id, tab_id, 0)
    }

    /// [`register_agent`], recording the registering persistent-controller
    /// spawn's generation so its own exit-handler can later
    /// compare-and-remove ([`unregister_block_if_generation`]) instead of
    /// blindly wiping a fallback respawn's fresh registration (issue #2363).
    pub fn register_agent_generated(
        &mut self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        spawn_generation: u64,
    ) -> Result<(), String> {
        if !validate_agent_id(agent_id) {
            return Err(format!("invalid agent ID: {}", agent_id));
        }

        let agent_key = agent_id.to_lowercase();

        // Remove existing registration for this agent
        if let Some(old_block) = self.agent_to_block.remove(&agent_key) {
            self.block_to_agent.remove(&old_block);
        }

        // Remove existing registration for this block
        if let Some(old_agent) = self.block_to_agent.remove(block_id) {
            self.agent_to_block.remove(&old_agent);
            self.agent_info.remove(&old_agent);
        }

        let now = now_unix_millis();
        self.agent_to_block
            .insert(agent_key.clone(), block_id.to_string());
        self.block_to_agent
            .insert(block_id.to_string(), agent_key.clone());
        self.agent_info.insert(
            agent_key.clone(),
            AgentRegistration {
                agent_id: agent_id.to_string(),
                block_id: block_id.to_string(),
                tab_id: tab_id.map(|s| s.to_string()),
                registered_at: now,
                last_seen: now,
                spawn_generation,
            },
        );

        Ok(())
    }

    /// Unregister an agent.
    pub fn unregister_agent(&mut self, agent_id: &str) {
        let agent_key = agent_id.to_lowercase();
        if let Some(block_id) = self.agent_to_block.remove(&agent_key) {
            self.block_to_agent.remove(&block_id);
        }
        self.agent_info.remove(&agent_key);
    }

    /// Unregister by block ID.
    pub fn unregister_block(&mut self, block_id: &str) {
        if let Some(agent_id) = self.block_to_agent.remove(block_id) {
            self.agent_to_block.remove(&agent_id);
            self.agent_info.remove(&agent_id);
        }
    }

    /// Unregister by block ID **only if** the current registration was
    /// written by the spawn with `expected_generation` — a
    /// compare-and-remove for persistent-controller exit-handlers (issue
    /// #2363: the handler's `is_current_generation` gate is read once,
    /// while a fallback respawn re-registers on a parallel task; an
    /// unconditional [`unregister_block`] here could wipe the NEW spawn's
    /// registration, leaving the live agent invisible to Tier-1 delivery
    /// with nothing left to re-register it). Runs atomically under the
    /// handler's own lock (via the outer wrapper). A registration with no
    /// recorded generation (0 — HTTP/PTY paths) is never removed by this
    /// variant: leaving a stale entry to the TTL sweep is strictly safer
    /// than deleting a live one.
    ///
    /// Returns true if the registration was ours and was removed.
    pub fn unregister_block_if_generation(&mut self, block_id: &str, expected_generation: u64) -> bool {
        let Some(agent_id) = self.block_to_agent.get(block_id) else {
            return false;
        };
        let matches = self
            .agent_info
            .get(agent_id)
            .is_some_and(|info| expected_generation != 0 && info.spawn_generation == expected_generation);
        if !matches {
            tracing::info!(
                block_id = %block_id,
                expected_generation = expected_generation,
                "reactive: registration generation changed since this spawn registered — skipping unregister"
            );
            return false;
        }
        self.unregister_block(block_id);
        true
    }

    /// Update the last_seen timestamp for an agent.
    #[allow(dead_code)]
    pub fn update_last_seen(&mut self, agent_id: &str) {
        if let Some(info) = self.agent_info.get_mut(&agent_id.to_lowercase()) {
            info.last_seen = now_unix_millis();
        }
    }

    /// Get agent registration by agent ID.
    pub fn get_agent(&self, agent_id: &str) -> Option<&AgentRegistration> {
        self.agent_info.get(&agent_id.to_lowercase())
    }

    /// Get agent registration by block ID.
    #[allow(dead_code)]
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
                effective_tier: None,
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
                effective_tier: None,
            };
        }

        // Sanitize message
        let sanitized = sanitize_message(&req.message);

        // Look up block ID
        let block_id = match self.agent_to_block.get(&req.target_agent.to_lowercase()) {
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
                    effective_tier: None,
                };
            }
        };

        // Determine effective jekt tier.
        // Escalation rules (spec §5.2):
        //   1. WAN or LAN delivery → always SENSITIVE, regardless of declared tier
        //      or keyword content (network-tier senders are not verified).
        //   2. Host delivery + declared SENSITIVE → SENSITIVE.
        //   3. Host delivery + keyword match → SENSITIVE.
        //   4. Otherwise → use declared tier (default: coord).
        let declared_tier = req.jekt_tier.as_ref();
        let delivery_tier = req.delivery_tier.as_deref().unwrap_or("host");
        let is_network_tier = delivery_tier == "wan" || delivery_tier == "lan";
        let is_sensitive = is_network_tier
            || matches!(declared_tier, Some(super::types::JektTier::Sensitive))
            || is_sensitive_message(&sanitized);
        let effective_tier = if is_sensitive { "sensitive" } else {
            declared_tier.map_or("coord", |t| match t {
                super::types::JektTier::Info => "info",
                super::types::JektTier::Coord => "coord",
                super::types::JektTier::Sensitive => "sensitive",
            })
        };
        let priority = req.priority.as_deref().unwrap_or("normal");

        // Wrap in JEKT marker block (structured tag + human-readable header).
        let wrapped = wrap_jekt_message(
            &sanitized,
            req.source_agent.as_deref(),
            &req.target_agent,
            effective_tier,
            delivery_tier,
            &request_id,
            priority,
        );

        // Legacy source-prefix format preserved for PTY controllers that don't
        // parse the JEKT block — the wrap_jekt_message output already includes
        // the source in the human-readable header so format_injected_message
        // is called with include_source=false here.
        let final_msg = format_injected_message(
            &wrapped,
            req.source_agent.as_deref(),
            false,
        );

        // Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3).
        // Persistent (stream-json) and ACP agents have no PTY — their inbox is a
        // structured channel (live stdin NDJSON / `session/prompt`). Delivering there
        // also lands the message mid-turn (steering) instead of waiting for idle.
        // PTY-based shell/term agents report back so we fall through to keystrokes.
        if let Some(ref deliver) = self.message_sender {
            match deliver(&block_id, &final_msg) {
                Ok(true) => {
                    tracing::info!(
                        target_agent = %req.target_agent,
                        block_id = %block_id,
                        "inject: structured delivery to non-PTY controller (mid-turn steer)"
                    );
                    self.log_audit(
                        req.source_agent.as_deref(),
                        &req.target_agent,
                        &block_id,
                        &sanitized,
                        true,
                        None,
                        &request_id,
                    );
                    return InjectionResponse {
                        success: true,
                        request_id,
                        block_id: Some(block_id),
                        error: None,
                        timestamp: now,
                        effective_tier: Some(effective_tier.to_string()),
                    };
                }
                Ok(false) => {
                    // PTY-based controller — fall through to keystroke injection.
                }
                Err(e) => {
                    // Structured controller but delivery failed (e.g. persistent
                    // process not running). Do NOT fall back to PTY keystrokes — the
                    // persistent controller rejects raw input. Surface the error.
                    tracing::warn!(
                        target_agent = %req.target_agent,
                        block_id = %block_id,
                        error = %e,
                        "inject: structured delivery failed"
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
                        effective_tier: Some(effective_tier.to_string()),
                    };
                }
            }
        }

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
                    effective_tier: Some(effective_tier.to_string()),
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
                effective_tier: Some(effective_tier.to_string()),
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
            effective_tier: Some(effective_tier.to_string()),
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

    pub fn set_message_sender(&self, sender: MessageSender) {
        self.inner.lock().unwrap().set_message_sender(sender);
    }

    #[allow(dead_code)]
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

    /// See the inner [`Handler::register_agent_generated`].
    pub fn register_agent_generated(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        spawn_generation: u64,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .register_agent_generated(agent_id, block_id, tab_id, spawn_generation)
    }

    pub fn unregister_agent(&self, agent_id: &str) {
        self.inner.lock().unwrap().unregister_agent(agent_id);
    }

    pub fn unregister_block(&self, block_id: &str) {
        self.inner.lock().unwrap().unregister_block(block_id);
    }

    /// See the inner [`Handler::unregister_block_if_generation`] — atomic
    /// compare-and-remove under the handler lock (issue #2363).
    pub fn unregister_block_if_generation(&self, block_id: &str, expected_generation: u64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .unregister_block_if_generation(block_id, expected_generation)
    }

    /// Return the logical agent_id currently mapped to this block, if any.
    pub fn agent_id_for_block(&self, block_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .block_to_agent
            .get(block_id)
            .cloned()
    }

    #[allow(dead_code)]
    pub fn update_last_seen(&self, agent_id: &str) {
        self.inner.lock().unwrap().update_last_seen(agent_id);
    }

    pub fn get_agent(&self, agent_id: &str) -> Option<AgentRegistration> {
        self.inner.lock().unwrap().get_agent(agent_id).cloned()
    }

    #[allow(dead_code)]
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
