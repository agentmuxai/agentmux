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

// ---- Supervisor nudge-ceiling tracking ----

/// Per-target-agent consecutive-nudge tracking for the Warden Supervisor
/// guardrail. Keyed on the target agent's lowercased name in
/// `Handler::nudge_counters`.
struct NudgeCounterState {
    /// The target's `AgentRegistration::registration_nonce` as of the last
    /// nudge — a respawn (new nonce) resets the counter, since "consecutive"
    /// only makes sense within one continuous run. Real nonces are ≥ 1
    /// (persistent-controller spawns only — `persistent::next_registration_
    /// nonce`); PTY/shell and HTTP-register paths always register with 0
    /// ("not recorded"), so nonce alone can't detect a respawn for them —
    /// `block_id` below covers that case instead.
    registration_nonce: u64,
    /// The target's block id as of the last nudge. A PTY/shell-registered
    /// agent that's closed and relaunched gets a fresh block id (a new
    /// pane), which nonce (always 0 for these paths) can't see — comparing
    /// block_id catches that respawn case reagent flagged as a P1 gap
    /// (nonce-only reset never fires for PTY agents). Doesn't help detect a
    /// respawn that reuses the same pane/block id (e.g. a one-shot
    /// SubprocessController that re-registers every turn in-place) — that
    /// case still relies on `NUDGE_COOLDOWN_RESET_MS` alone, same as before.
    block_id: String,
    count: u32,
    last_nudge_at_ms: u64,
}

/// Max consecutive auto-continue nudges a Supervisor may send to the same
/// target agent (within one registration / cooldown window) before
/// `record_supervisor_decision` refuses and forces a decline instead. Bounds
/// a runaway auto-continue loop — see
/// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
pub(super) const MAX_CONSECUTIVE_AUTO_CONTINUES: u32 = 5;

/// Gap since the last nudge after which the consecutive counter resets, on
/// the theory that a long-idle target has effectively started a new work
/// session even without a fresh `registration_nonce`.
pub(super) const NUDGE_COOLDOWN_RESET_MS: u64 = 30 * 60 * 1000;

/// The one, fixed message a `SupervisorAction::Nudge` ever delivers.
/// Deliberately not a parameter — see the doc on `record_supervisor_decision`'s
/// `Nudge` arm for why a free-form message defeats the guardrail this
/// exists for.
pub(super) const NUDGE_MESSAGE: &str = "Continue the task you were already doing.";

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
    /// Warden Supervisor consecutive-nudge ceiling state, keyed on the
    /// target agent's lowercased name. In-memory only (same lifecycle as
    /// `audit_log`) — not persisted across a restart.
    nudge_counters: HashMap<String, NudgeCounterState>,
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
            nudge_counters: HashMap::new(),
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
        self.register_agent_with_nonce(agent_id, block_id, tab_id, 0)
    }

    /// [`register_agent`], recording the registering persistent-controller
    /// spawn's process-wide registration nonce
    /// (`AgentRegistration::registration_nonce`) so its own exit-handler
    /// can later compare-and-remove ([`unregister_block_if_nonce`])
    /// instead of blindly wiping a fallback respawn's fresh registration
    /// (issue #2363).
    pub fn register_agent_with_nonce(
        &mut self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
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
                registration_nonce,
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
    /// written by the spawn with `expected_nonce` — a
    /// compare-and-remove for persistent-controller exit-handlers (issue
    /// #2363: the handler's `is_current_generation` gate is read once,
    /// while a fallback respawn re-registers on a parallel task; an
    /// unconditional [`unregister_block`] here could wipe the NEW spawn's
    /// registration, leaving the live agent invisible to Tier-1 delivery
    /// with nothing left to re-register it). The nonce is process-wide
    /// unique, so the guard also holds across controller replacement
    /// (`resync_controller` — codex P1 on PR #2500), where a
    /// controller-local generation would restart at 1 and collide. Runs
    /// atomically under the handler's own lock (via the outer wrapper).
    /// A registration with no recorded nonce (0 — HTTP/PTY paths) is
    /// never removed by this variant: leaving a stale entry to the TTL
    /// sweep is strictly safer than deleting a live one.
    ///
    /// Returns true if the registration was ours and was removed.
    pub fn unregister_block_if_nonce(&mut self, block_id: &str, expected_nonce: u64) -> bool {
        let Some(agent_id) = self.block_to_agent.get(block_id) else {
            return false;
        };
        let matches = self
            .agent_info
            .get(agent_id)
            .is_some_and(|info| expected_nonce != 0 && info.registration_nonce == expected_nonce);
        if !matches {
            tracing::info!(
                block_id = %block_id,
                expected_nonce = expected_nonce,
                "reactive: registration changed hands since this spawn registered — skipping unregister"
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
    pub fn inject_message(&mut self, req: InjectionRequest) -> InjectionResponse {
        self.inject_message_inner(req, None, None, None)
    }

    /// Shared delivery path behind both `inject_message` (ordinary jekt,
    /// every `outcome`/`reason` param always `None`) and
    /// `record_supervisor_decision`'s `Nudge` arm (`outcome_on_success:
    /// Some("nudge_sent")`, `outcome_on_failure: Some("nudge_failed")`,
    /// `reason` the Supervisor's stated reasoning) — every audit-log write
    /// below carries these through instead of the two call sites
    /// duplicating sanitize/deliver logic. Two separate outcome params
    /// (rather than one applied uniformly) so a failed delivery is audited
    /// distinctly from a successful one (reagentx P2 on PR #2557 — the
    /// Supervisor UI's decision feed must not show "nudged" for a delivery
    /// that actually failed).
    fn inject_message_inner(
        &mut self,
        mut req: InjectionRequest,
        outcome_on_success: Option<&str>,
        outcome_on_failure: Option<&str>,
        reason: Option<&str>,
    ) -> InjectionResponse {
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
                requires_stop: None,
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
                requires_stop: None,
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
                    outcome_on_failure,
                    reason,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: None,
                    error: Some(err),
                    timestamp: now,
                    effective_tier: None,
                    requires_stop: None,
                };
            }
        };

        // Determine effective jekt tier.
        // Escalation rules (spec §5.2, extended by
        // SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, by
        // SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md §1 for the
        // network-tier exception below, and NARROWED by
        // SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md — repo-owner-
        // confirmed directly in a live conversation, not a jekt/muxbus claim):
        //   1. WAN or LAN delivery, sender's identity check ACTIVELY FAILED
        //      (a reagent_sig was present but didn't verify — SIG=invalid) →
        //      always SENSITIVE, regardless of declared tier or keyword
        //      content. A real red flag: someone tried to forge a signature.
        //      Mere ABSENCE of any signature attempt (no reagent_sig sent at
        //      all, or one verified only under the known-exposed dev key) is
        //      NOT this case — that's rule 5's default, same as any other
        //      self-declared sender. LAN never carries a signature attempt
        //      at all, so it's never eligible for this rule either — see
        //      rules 3/4 for what still catches a malicious LAN jekt.
        //   1b. WAN delivery, sender verified via reagent's pinned Ed25519
        //      key (`reagent_verified == Some(true)`) → NOT forced to
        //      SENSITIVE by delivery tier alone. As of the 2026-08-15
        //      narrowing this is NOT a distinct check anymore — it's simply
        //      rule 1 not matching (`Some(true)` isn't `Some(false)`), so a
        //      message verified under the trusted `reagent-v1` key and one
        //      verified only under the known-exposed `reagent-v1-dev`
        //      placeholder now get IDENTICAL tier treatment: neither is
        //      forced sensitive. `is_reagent_trusted_signing_key`
        //      (agentmux-common::jekt_sign) is NOT consulted here at all —
        //      unlike before this narrowing, key trust no longer gates
        //      TIER in any way; it still exists for other verification
        //      bookkeeping, just not this decision. Rules 3/4 below still
        //      apply on top: a verified reagent message that declares
        //      SENSITIVE or matches the keyword scan still escalates.
        //   2. Host delivery, sender identity checkable but signature missing
        //      or wrong → always SENSITIVE (host-tier senders can now be
        //      verified when the claimed source_agent has a signing key —
        //      see `sig_verified`'s doc comment on `InjectionRequest` for
        //      exactly when this applies vs. is skipped). Same "an active
        //      verification FAILURE is the red flag, not mere absence of
        //      one" logic as rule 1 — this rule was already scoped that way.
        //   3. Declared SENSITIVE (any tier) → SENSITIVE.
        //   4. Keyword match (any tier) → SENSITIVE.
        //   5. Otherwise → use declared tier (default: coord). This is now
        //      reachable by ordinary unverified LAN/WAN traffic with clean
        //      content — `TRUST` in the marker is UNCHANGED by this
        //      narrowing (still reads `network-claimed`, still exactly as
        //      forgeable as ever); only whether that lack of proof alone
        //      is sufficient grounds to interrupt the human has changed.
        let declared_tier = req.jekt_tier.as_ref();
        let delivery_tier = req.delivery_tier.as_deref().unwrap_or("host");
        let is_network_tier = delivery_tier == "wan" || delivery_tier == "lan";
        // A reagent_sig that was PRESENT but didn't verify — someone tried to
        // forge it. `reagent_verified` is WAN-only by construction
        // (`sync_agent_reactive`/`verify_reagent_signature` never compute it
        // off the WAN tier, so it's always `None` for LAN — reagent is a
        // WAN-only service sender, this never applied to LAN) — absence of a
        // signature attempt (`None`), or a signature that verified but only
        // under the known-exposed dev key, is NOT this case; both fall
        // through to rule 5 like any other self-declared sender.
        let is_network_tier_sig_invalid = is_network_tier && req.reagent_verified == Some(false);
        // A lan_sig that was PRESENT, whose claimed sender's public key WAS
        // found, but didn't cryptographically verify — a specific agent's
        // identity was actively forged, not merely unproven. Scoped to LAN
        // only, same reasoning as reagent's WAN scoping above — see
        // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.4/§2.5.
        let is_lan_sig_invalid = delivery_tier == "lan" && req.lan_verified == Some(false);
        let is_unverified_sender = req.sig_verified == Some(false);
        let is_sensitive = is_network_tier_sig_invalid
            || is_lan_sig_invalid
            || is_unverified_sender
            || matches!(declared_tier, Some(super::types::JektTier::Sensitive))
            || is_sensitive_message(&sanitized);
        let effective_tier = if is_sensitive { "sensitive" } else {
            declared_tier.map_or("coord", |t| match t {
                super::types::JektTier::Info => "info",
                super::types::JektTier::Coord => "coord",
                super::types::JektTier::Sensitive => "sensitive",
            })
        };
        // Whether TIER=sensitive should actually STOP the receiving agent and
        // require human confirmation, vs. just carry the tag for visual
        // indication (SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md,
        // repo-owner-confirmed directly in a live conversation, same channel
        // this policy's own STOP rule already treats as authoritative).
        //
        // Cryptographic proof of identity is exactly what the STOP rule was
        // protecting against the ABSENCE of (the spoofed-jekt-then-spoofed-
        // muxbus-confirmation incident this whole tiering system exists to
        // stop). Once a sender is actually verified — `sig_verified`,
        // `reagent_verified`, or `lan_verified` all being `Some(true)` — that
        // specific attack is no longer possible for this message, regardless
        // of WHY it was marked sensitive (self-declared, or a keyword match
        // on content a genuinely-signed sender is allowed to legitimately
        // discuss, e.g. a code review of credential-handling code).
        //
        // This can never accidentally cover an active-forgery case: the three
        // rules above that force `is_sensitive` from a signature actively
        // failing (`is_network_tier_sig_invalid`, `is_lan_sig_invalid`,
        // `is_unverified_sender`) are each keyed on the SAME field this
        // checks for `Some(true)` reading `Some(false)` instead — the two can
        // never both be true for the same field at once, so a message that
        // reaches STOP-required via one of those three rules is, by
        // construction, never simultaneously "verified" on that same tier.
        let is_cryptographically_verified = req.sig_verified == Some(true)
            || req.reagent_verified == Some(true)
            || req.lan_verified == Some(true);
        let requires_stop = is_sensitive && !is_cryptographically_verified;
        let priority = req.priority.as_deref().unwrap_or("normal");

        // Wrap in JEKT marker block (structured tag + human-readable header).
        // Note: `req.sig_verified` (three-state) is passed through as-is for
        // the marker's TRUST label — `is_unverified_sender` above only
        // captures the `Some(false)` case (the one that forces SENSITIVE);
        // the marker itself also needs to distinguish `None` ("never
        // checked," e.g. a Slack-bridge message) from `Some(true)`
        // ("actually verified") rather than collapsing both to the same
        // label — see `wrap_jekt_message`'s doc comment.
        let wrapped = wrap_jekt_message(
            &sanitized,
            req.source_agent.as_deref(),
            &req.target_agent,
            effective_tier,
            delivery_tier,
            req.sig_verified,
            req.reagent_verified,
            req.lan_verified,
            requires_stop,
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
                        outcome_on_success,
                        reason,
                    );
                    return InjectionResponse {
                        success: true,
                        request_id,
                        block_id: Some(block_id),
                        error: None,
                        timestamp: now,
                        effective_tier: Some(effective_tier.to_string()),
                        requires_stop: Some(requires_stop),
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
                        outcome_on_failure,
                        reason,
                    );
                    return InjectionResponse {
                        success: false,
                        request_id,
                        block_id: Some(block_id),
                        error: Some(e),
                        timestamp: now,
                        effective_tier: Some(effective_tier.to_string()),
                        requires_stop: Some(requires_stop),
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
                    outcome_on_failure,
                    reason,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: Some(block_id),
                    error: Some(err),
                    timestamp: now,
                    effective_tier: Some(effective_tier.to_string()),
                    requires_stop: Some(requires_stop),
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
                outcome_on_failure,
                reason,
            );
            return InjectionResponse {
                success: false,
                request_id,
                block_id: Some(block_id),
                error: Some(e),
                timestamp: now,
                effective_tier: Some(effective_tier.to_string()),
                requires_stop: Some(requires_stop),
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
            outcome_on_success,
            reason,
        );

        InjectionResponse {
            success: true,
            request_id,
            block_id: Some(block_id),
            error: None,
            timestamp: now,
            effective_tier: Some(effective_tier.to_string()),
            requires_stop: Some(requires_stop),
        }
    }

    /// Record a Warden Supervisor watcher agent's decision about
    /// `target_agent`. A `Nudge` is delivered through the same path
    /// `inject_message` uses (`inject_message_inner`) and audited with
    /// `outcome: "nudge_sent"`; a `Decline` sends nothing and is audited
    /// directly with `outcome: "nudge_declined"`.
    ///
    /// Enforces the consecutive-nudge ceiling
    /// (`MAX_CONSECUTIVE_AUTO_CONTINUES`): if this nudge would exceed it,
    /// the decision is forced to a decline (audited with
    /// `outcome: "nudge_declined"`, `reason: "consecutive-nudge ceiling
    /// reached"`) and `Err` is returned so the calling agent's MCP
    /// tool-call result surfaces the refusal directly — a signal for it to
    /// stop nudging and escalate to a human via an ordinary jekt instead of
    /// retrying. The counter resets when the target's `registration_nonce`
    /// or `block_id` changes (a respawn — "consecutive" only makes sense
    /// within one continuous run; see `NudgeCounterState`'s field docs for
    /// why both signals are needed) or after `NUDGE_COOLDOWN_RESET_MS` of
    /// inactivity.
    ///
    /// Does NOT check the target's `auto_continue_enabled` opt-in itself —
    /// `Handler` has no `Store` access by design (this module doesn't
    /// depend on `backend::storage`). That gate lives at the HTTP boundary,
    /// in `handle_reactive_supervisor_decision`
    /// (`server/reactive.rs`), which has `AppState::wstore`. Any other
    /// caller of this method directly is responsible for its own
    /// entitlement check first.
    pub fn record_supervisor_decision(
        &mut self,
        target_agent: &str,
        action: SupervisorAction,
        reason: &str,
        request_id: &str,
        source_agent: Option<&str>,
    ) -> Result<InjectionResponse, String> {
        let now = now_unix_millis();
        let target_key = target_agent.to_lowercase();
        let block_id = self
            .agent_to_block
            .get(&target_key)
            .cloned()
            .unwrap_or_default();

        match action {
            SupervisorAction::Decline => {
                self.log_audit(
                    source_agent,
                    target_agent,
                    &block_id,
                    "",
                    true,
                    None,
                    request_id,
                    Some("nudge_declined"),
                    Some(reason),
                );
                Ok(InjectionResponse {
                    success: true,
                    request_id: request_id.to_string(),
                    block_id: if block_id.is_empty() { None } else { Some(block_id) },
                    error: None,
                    timestamp: now,
                    effective_tier: None,
                    requires_stop: None,
                })
            }
            SupervisorAction::Nudge => {
                // Bound `nudge_counters`' growth (reagentx P2 on PR #2557):
                // an entry idle past the cooldown window is about to be
                // treated as stale on next use anyway, so dropping it here
                // is behavior-neutral — just reclaims memory instead of
                // accumulating one entry per distinct target agent forever.
                self.nudge_counters
                    .retain(|_, v| now.saturating_sub(v.last_nudge_at_ms) <= NUDGE_COOLDOWN_RESET_MS);

                let current_nonce = self
                    .agent_info
                    .get(&target_key)
                    .map(|info| info.registration_nonce)
                    .unwrap_or(0);

                // Scoped borrow: check/reset staleness and read the
                // pre-delivery count, then drop the borrow before calling
                // `inject_message_inner` (which needs `&mut self` too).
                let count_before = {
                    let entry = self.nudge_counters.entry(target_key.clone()).or_insert(NudgeCounterState {
                        registration_nonce: current_nonce,
                        block_id: block_id.clone(),
                        count: 0,
                        last_nudge_at_ms: 0,
                    });
                    // registration_nonce only distinguishes a respawn for
                    // persistent-controller agents (real nonces, ≥ 1);
                    // PTY/shell/HTTP-register paths always register with 0,
                    // so block_id is the fallback signal for those
                    // (reagentx P1 on PR #2557 — nonce alone never fired
                    // for them, leaving a respawned PTY agent stuck behind
                    // its prior run's exhausted ceiling for up to the full
                    // cooldown window).
                    let stale = entry.registration_nonce != current_nonce
                        || entry.block_id != block_id
                        || now.saturating_sub(entry.last_nudge_at_ms) > NUDGE_COOLDOWN_RESET_MS;
                    if stale {
                        entry.registration_nonce = current_nonce;
                        entry.block_id = block_id.clone();
                        entry.count = 0;
                    }
                    entry.count
                };

                if count_before >= MAX_CONSECUTIVE_AUTO_CONTINUES {
                    let ceiling_reason = "consecutive-nudge ceiling reached".to_string();
                    self.log_audit(
                        source_agent,
                        target_agent,
                        &block_id,
                        "",
                        true,
                        None,
                        request_id,
                        Some("nudge_declined"),
                        Some(&ceiling_reason),
                    );
                    return Err(ceiling_reason);
                }

                // Fixed, narrow continuation template — deliberately NOT
                // free-form text composed by the calling Supervisor agent.
                // ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_
                // 2026_08_12.md §4.3: "The nudge text should be a fixed,
                // narrow template... not a free-form instruction the
                // watcher composes per-situation — this is the direct
                // mitigation for consent-chain degradation." (reagentx P1
                // on PR #2557 — SupervisorNudge used to accept arbitrary
                // `message` text.) `reason` (the Supervisor's own
                // reasoning) still travels separately, into the audit log
                // only — never into what's delivered to the target.
                let req = InjectionRequest {
                    target_agent: target_agent.to_string(),
                    message: NUDGE_MESSAGE.to_string(),
                    source_agent: source_agent.map(|s| s.to_string()),
                    request_id: Some(request_id.to_string()),
                    priority: Some("normal".to_string()),
                    wait_for_idle: false,
                    jekt_tier: Some(super::types::JektTier::Coord),
                    delivery_tier: Some("host".to_string()),
                    forward_hops: 0,
                    ..Default::default()
                };
                let resp = self.inject_message_inner(
                    req,
                    Some("nudge_sent"),
                    Some("nudge_failed"),
                    Some(reason),
                );

                // Only a successful delivery consumes the ceiling
                // (reagentx P2 on PR #2557) — a rate-limited/unavailable-
                // controller failure shouldn't cost the Supervisor one of
                // its 5 consecutive attempts for this target.
                if resp.success {
                    if let Some(entry) = self.nudge_counters.get_mut(&target_key) {
                        entry.count += 1;
                        entry.last_nudge_at_ms = now;
                    }
                }

                Ok(resp)
            }
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

    /// Add an entry to the audit ring buffer. `outcome`/`reason` are `None`
    /// for every ordinary jekt injection (all current call sites) — only
    /// Warden Supervisor decisions (see `record_supervisor_decision`) ever
    /// set them.
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
        outcome: Option<&str>,
        reason: Option<&str>,
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
            outcome: outcome.map(|s| s.to_string()),
            reason: reason.map(|s| s.to_string()),
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

    /// See the inner [`Handler::register_agent_with_nonce`].
    pub fn register_agent_with_nonce(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .register_agent_with_nonce(agent_id, block_id, tab_id, registration_nonce)
    }

    pub fn unregister_agent(&self, agent_id: &str) {
        self.inner.lock().unwrap().unregister_agent(agent_id);
    }

    pub fn unregister_block(&self, block_id: &str) {
        self.inner.lock().unwrap().unregister_block(block_id);
    }

    /// See the inner [`Handler::unregister_block_if_nonce`] — atomic
    /// compare-and-remove under the handler lock (issue #2363).
    pub fn unregister_block_if_nonce(&self, block_id: &str, expected_nonce: u64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .unregister_block_if_nonce(block_id, expected_nonce)
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

    /// Records an action that isn't a jekt injection (currently: fleet
    /// bulk-stop — `agentmux-srv/src/server/app_api/fleet.rs`) into the SAME
    /// audit ring buffer, so it shows up in Warden's Audit tab exactly like
    /// an ordinary injection (`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`
    /// §6 — fleet actions get visibility there without Warden owning any
    /// new code). `action` fills the slot `log_audit` normally uses for the
    /// injected message text (e.g. `"fleet.bulk-stop"`); `target_agent` is
    /// the resolved agent name for `block_id` when known, else `block_id`
    /// itself (an unregistered/already-stopped block has no agent to name).
    #[allow(clippy::too_many_arguments)]
    pub fn log_fleet_action_audit(
        &self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        action: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
    ) {
        self.inner.lock().unwrap().log_audit(
            source_agent,
            target_agent,
            block_id,
            action,
            success,
            error_message,
            request_id,
            None,
            None,
        );
    }

    /// See the inner [`Handler::record_supervisor_decision`].
    pub fn record_supervisor_decision(
        &self,
        target_agent: &str,
        action: SupervisorAction,
        reason: &str,
        request_id: &str,
        source_agent: Option<&str>,
    ) -> Result<InjectionResponse, String> {
        self.inner
            .lock()
            .unwrap()
            .record_supervisor_decision(target_agent, action, reason, request_id, source_agent)
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
