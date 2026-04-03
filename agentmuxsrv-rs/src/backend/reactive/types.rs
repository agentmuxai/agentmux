// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;

use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agentmux_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agentmux_token: Option<String>,
    #[serde(default)]
    pub poll_interval_secs: u64,
}

/// AgentMux config file format (agentmux.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMuxConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Pending injection from AgentMux cloud.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingResponse {
    pub injections: Vec<PendingInjection>,
}

/// Acknowledgment request for delivered injections.
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
