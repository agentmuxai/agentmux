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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Sender-facing echo of the delivered `[JEKT:...]` block — `Some` only
    /// on success, so `agentmux-mcp`'s `SendMessage` can return it as the
    /// tool result instead of a bare confirmation string. See
    /// SPEC_JEKT_OUTGOING_ECHO_2026_07_10.md §2.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<String>,
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
