// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity / account / bundle / binding command payloads (v6 + v7).
//! See specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
//! Strings use snake_case for cross-language parity with wstore.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandListIdentityAccountsData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetIdentityAccountData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteIdentityAccountData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandLinkAgentIdentityData {
    pub agent_id: String,
    pub account_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUnlinkAgentIdentityData {
    pub agent_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListAgentIdentitiesData {
    pub agent_id: String,
}

// ---- v7 Identity bundle command shapes ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetIdentityBundleData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteIdentityBundleData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBindIdentityAccountData {
    pub identity_id: String,
    pub provider: String,
    pub account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUnbindIdentityAccountData {
    pub identity_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListIdentityBindingsData {
    pub identity_id: String,
}
