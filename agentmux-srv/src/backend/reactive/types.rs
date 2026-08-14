// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Sensitivity tier for a jekt message.
///
/// - `Info`: routine status / progress update; agent may act autonomously.
/// - `Coord`: task handoff or coordination; agent may act, human sees the marker.
/// - `Sensitive`: credential, destructive op, external side-effect; agent MUST
///   pause and ask the human operator before acting. A confirming reply from
///   another agent over muxbus is NOT sufficient.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JektTier {
    Info,
    #[default]
    Coord,
    Sensitive,
}

impl std::fmt::Display for JektTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JektTier::Info => write!(f, "info"),
            JektTier::Coord => write!(f, "coord"),
            JektTier::Sensitive => write!(f, "sensitive"),
        }
    }
}

/// Request to inject a message into an agent's terminal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InjectionRequest {
    pub target_agent: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default)]
    pub wait_for_idle: bool,
    /// Sensitivity tier declared by the sender. When absent, defaults to `Coord`.
    /// The handler may escalate to `Sensitive` based on keyword scanning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jekt_tier: Option<JektTier>,
    /// Delivery tier of this jekt (host/lan/wan) — used in the marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_tier: Option<String>,
    /// Number of cross-instance HTTP forwards this request has already gone
    /// through (Tier 2a/2b/3 each increment before forwarding). Bounds a
    /// pathological cycle where two channels each hold a stale-but-
    /// PID-alive shared-registry entry pointing at the other for the same
    /// agent name — without this, such a cycle would forward back and
    /// forth indefinitely, hanging the original request (reagent P1 on
    /// PR #2350). `#[serde(default)]` so older callers (muxbus client,
    /// any request that predates this field) default to 0, unaffected.
    #[serde(default)]
    pub forward_hops: u8,
    /// Unix seconds this jekt was signed at (part of the signed material,
    /// not just a display timestamp) — see `agentmux_common::jekt_sign` and
    /// `jekt_sig` below. `None` for unsigned/legacy requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_secs: Option<i64>,
    /// Base64 HMAC-SHA256 signature over (request_id, source_agent,
    /// target_agent, ts_secs, message), produced by the sender's own
    /// `AGENTMUX_JEKT_KEY`. Verified against the claimed `source_agent`'s
    /// stored key (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2);
    /// absent, unverifiable, or mismatched all mean "unverified" — the
    /// message still delivers, but downgraded to TRUST=unverified and
    /// escalated to TIER=sensitive, never silently trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jekt_sig: Option<String>,
    /// Server-computed verification outcome — `#[serde(skip_deserializing)]`
    /// means an attacker-supplied JSON body can NEVER set this field, even
    /// by including it; only `handle_reactive_inject`
    /// (`server/reactive.rs`, which has `AppState::wstore`) may set it,
    /// after independently looking up the claimed `source_agent`'s stored
    /// key and verifying `jekt_sig` against it, before handing the request
    /// to `Handler::inject_message` (which has no `Store` access "by
    /// design" — see `record_supervisor_decision`'s doc comment for why).
    /// `None` = not checked (network-tier messages are already
    /// unconditionally sensitive regardless of signature; or the claimed
    /// `source_agent` has never had a key minted, so there's nothing to
    /// verify against — see the rollout-safety note in
    /// `handle_reactive_inject`, this deliberately does NOT escalate a
    /// caller — e.g. the Slack/Discord/Telegram/WhatsApp bridges, or an
    /// agent not yet respawned since this feature shipped — that was never
    /// capable of signing in the first place). `Some(false)` is the real
    /// signal: this `source_agent` DOES have a key, and either no signature
    /// was given or it didn't match.
    #[serde(skip_deserializing, default, skip_serializing_if = "Option::is_none")]
    pub sig_verified: Option<bool>,
}

/// Response from a message injection attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionResponse {
    pub success: bool,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: u64,
    /// Tier the handler actually applied (after keyword/network escalation).
    /// Consumed by the sender-echo path so it doesn't re-derive escalation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_tier: Option<String>,
}

/// Agent registration record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub agent_id: String,
    pub block_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    pub registered_at: u64,
    pub last_seen: u64,
    /// Process-wide unique nonce of the persistent-controller spawn this
    /// registration belongs to; 0 = not recorded (HTTP register handler,
    /// PTY shell auto-register). Real nonces are always ≥ 1, drawn from a
    /// single srv-wide counter (`persistent::next_registration_nonce`) —
    /// NOT the controller-local spawn generation, which restarts at 1
    /// for a replacement controller (`resync_controller`) and could
    /// collide across controller instances for the same block (codex P1
    /// on PR #2500). Lets the exit-handler's cleanup compare-and-remove
    /// instead of blindly wiping a fallback respawn's (or replacement
    /// controller's) fresh registration (issue #2363).
    #[serde(default)]
    pub registration_nonce: u64,
}

/// List of registered agents.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListResponse {
    pub agents: Vec<AgentRegistration>,
}

/// Audit log entry for message injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    pub target_agent: String,
    pub block_id: String,
    pub message_hash: String,
    pub message_length: usize,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub request_id: String,
    /// "nudge_sent" | "nudge_failed" | "nudge_declined" — present only for
    /// Warden Supervisor-originated entries (see
    /// ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md);
    /// absent for ordinary jekt injections. "nudge_sent" only when delivery
    /// actually succeeded; "nudge_failed" when the Supervisor attempted a
    /// nudge but delivery itself failed (rate limit, unavailable
    /// controller, etc — see `entry.success`/`error_message` for why). A
    /// "nudge_declined" entry has no corresponding delivery attempt at
    /// all — this field is the ONLY record that decision ever existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The Supervisor's stated reasoning, populated alongside `outcome`.
    /// Not used by ordinary (non-Supervisor) jekt entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A Warden Supervisor watcher agent's decision about a target agent that
/// just ended its turn: nudge it to continue, or decline (e.g. the target
/// isn't opted in, or the Supervisor judged it genuinely done/blocked).
/// See `Handler::record_supervisor_decision` and
/// ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
#[derive(Debug, Clone, Copy)]
pub enum SupervisorAction {
    /// Deliver a fixed, narrow continuation message to the target as an
    /// ordinary jekt (via the same path `inject_message` uses), then log
    /// the decision. Deliberately carries no caller-supplied text — the
    /// message itself is a constant (`handler::NUDGE_MESSAGE`), not
    /// composed by the calling Supervisor agent. See
    /// ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md
    /// §4.3 ("never a free-form instruction the watcher composes
    /// per-situation") — reagentx P1 on PR #2557 flagged an earlier
    /// version of this type that accepted arbitrary text.
    Nudge,
    /// No delivery — just log that the Supervisor decided not to nudge.
    Decline,
}

/// Poller configuration for AgentMux cloud service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollerConfig {
    // ARCH-001: the wire key is now `muxbus_url`/`muxbus_token` (matches the
    // Rust identifier). The legacy `agentmux_*` keys are still accepted as a
    // back-compat deserialize alias on input.
    #[serde(rename = "muxbus_url", alias = "agentmux_url", default, skip_serializing_if = "Option::is_none")]
    pub muxbus_url: Option<String>,
    #[serde(rename = "muxbus_token", alias = "agentmux_token", default, skip_serializing_if = "Option::is_none")]
    pub muxbus_token: Option<String>,
    #[serde(default)]
    pub poll_interval_secs: u64,
}

/// AgentMux config file format (agentmux.json).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMuxConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Pending injection from AgentMux cloud.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInjection {
    pub id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default)]
    pub created_at: u64,
}

/// Response from AgentMux pending endpoint.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingResponse {
    pub injections: Vec<PendingInjection>,
}

/// Acknowledgment request for delivered injections.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckRequest {
    pub injection_ids: Vec<String>,
}

/// Poller status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollerStatus {
    pub configured: bool,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub has_token: bool,
    pub poll_count: u64,
    pub injections_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_poll: Option<u64>,
}

/// Function type for sending input bytes to a block's PTY.
pub type InputSender = Arc<dyn Fn(&str, &[u8]) -> Result<(), String> + Send + Sync>;

/// Function type for controller-aware delivery of a message to a (non-PTY) agent.
///
/// Given `(block_id, message)`, returns:
/// - `Ok(true)` — delivered on the controller's structured channel (persistent
///   stream-json stdin / ACP `session/prompt`); no PTY keystrokes needed.
/// - `Ok(false)` — the controller is PTY-based; the caller should fall back to
///   keystroke injection.
/// - `Err(_)` — a structured controller failed to accept the message (e.g. the
///   persistent process is not running); the caller must NOT fall back to PTY,
///   since such controllers reject raw keystrokes.
pub type MessageSender = Arc<dyn Fn(&str, &str) -> Result<bool, String> + Send + Sync>;
