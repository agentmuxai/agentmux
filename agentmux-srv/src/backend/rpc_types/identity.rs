// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity / account / agent-identity-link command payloads (v6 + v7).
//! See specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
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
    /// Skip the `agentcredentials:revoked:<agent_id>` broadcast (see the
    /// handler's own comment — spec-mandated for a genuine unbind, since a
    /// live process still holds the unlinked account's tokens until
    /// restarted). Set when this unlink is really an ALIAS MIGRATION — the
    /// same credential is staying bound to the agent under its canonical
    /// provider id, just cleaning up the now-redundant legacy-alias row
    /// (reagent P2 on PR #2414: the generic revoked event made a successful
    /// re-login show "Credentials revoked" immediately afterward). Default
    /// false — every existing caller is a real unbind and keeps disclosing.
    #[serde(default)]
    pub silent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListAgentIdentitiesData {
    pub agent_id: String,
}

