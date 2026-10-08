// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the cloud presence publisher (`presence.*`,
//! `muxbus::wan_presence`): whether this install's record reaches the
//! account's relay, and why not.

use serde::{Deserialize, Serialize};

/// Where the publisher stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum PresenceState {
    /// No MuxBus sign-in: nothing is sent.
    SignedOut,
    /// The last record was stored.
    Publishing,
    /// The relay was unreachable, failed or was busy: backing off.
    Retrying,
    /// The relay has no presence route yet.
    Unsupported,
    /// The relay refused the record.
    Rejected,
}

/// `presence.status`. Times are this computer's clock (unix ms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PresenceStatusResult {
    pub state: PresenceState,
    /// When the publisher entered `state`.
    #[ts(type = "number")]
    pub since_ms: u64,
    /// When a record was last stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_ok_ms: Option<u64>,
    /// Why the last try failed, in a few words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_error: Option<String>,
    /// When the next try is due; absent while signed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub next_try_ms: Option<u64>,
    /// The relay's version, from its health check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay_version: Option<String>,
    /// The relay's clock minus this computer's, from its last answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub offset_ms: Option<i64>,
    /// The record version the publisher sends: 2, or 1 for an older relay.
    #[ts(type = "number")]
    pub record_version: u32,
    /// Something worth showing beside the state: a clock that differs from
    /// the relay's, or an older relay that gets no agent states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// `presence.publish-now`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PresencePublishNowResult {
    /// False when the publisher isn't running on this install.
    pub started: bool,
}
