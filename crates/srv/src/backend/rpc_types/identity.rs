// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity / account / agent-identity-link command payloads (v6 + v7).
//! See docs/specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
//! Strings use snake_case for cross-language parity with mstore.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandListIdentityAccountsData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGetIdentityAccountData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandDeleteIdentityAccountData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandLinkAgentIdentityData {
    pub agent_id: String,
    pub account_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandListAgentIdentitiesData {
    pub agent_id: String,
}


/// Result of `unlinkagentidentity`. Was an inline `json!({ "unlinked": .. })`.
/// False means there was no link to remove — the unlink is idempotent, so this
/// is "was something actually removed", not an error flag.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct UnlinkAgentIdentityResult {
    pub unlinked: bool,
}

/// Result of `account.oauth.cancel`. Was an inline
/// `json!({ "cancelled": .. })`. False means there was no in-flight flow.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AccountOAuthCancelResult {
    pub cancelled: bool,
}

/// Request for `listallagentidentities`. The handler ignores its payload, but
/// this must be a struct rather than `()`: the stub calls it with `{}`, and
/// serde deserializes `()` only from JSON `null`, so a unit Req would reject
/// every real call at runtime while passing every CI gate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandListAllAgentIdentitiesData {}
