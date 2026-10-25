// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `pins.list`, `pins.forget` and `pins.adopt`: the keys and
//! installs this machine has pinned to the names of agents that speak to it
//! from elsewhere, for the operator's screen. Only the operator's own window
//! may call any of them.

use serde::{Deserialize, Serialize};

/// Data for `COMMAND_PINS_LIST`: nothing to ask.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandPinsListData {}

/// Data for `COMMAND_PINS_FORGET` and `COMMAND_PINS_ADOPT`: which pin.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandPinsChangeData {
    /// `lan` (the key pinned to a name on the local network) or `wan` (the
    /// install pinned to a name through the cloud).
    pub kind: String,
    pub agent_id: String,
    /// Required for `wan`: the channel the pin is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channel: Option<String>,
}

/// Every pin, as the screen lists them.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PinnedSendersResult {
    pub lan: Vec<LanPinInfo>,
    pub wan: Vec<WanPinInfo>,
}

/// A name on the local network and the key pinned to it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LanPinInfo {
    pub agent_id: String,
    /// A short fingerprint of the pinned key (not the key).
    pub key_fp: String,
    /// When the key was first seen, epoch seconds.
    #[ts(type = "number")]
    pub pinned_at: i64,
    /// How many UID keys are pinned beside the name.
    pub uid_pins: u32,
    /// The last other key shown for the name since srv started, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub refused: Option<RefusedKeyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RefusedKeyInfo {
    pub key_fp: String,
    #[ts(type = "number")]
    pub last_seen_at: i64,
    pub count: u32,
}

/// An agent name on a channel and the install pinned to speak for it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WanPinInfo {
    pub agent_id: String,
    pub channel: String,
    /// The pinned install, as `host~id8`.
    pub instance: String,
    #[ts(type = "number")]
    pub pinned_at: i64,
    /// The last other install whose message for the name was refused, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub refused: Option<RefusedInstanceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RefusedInstanceInfo {
    /// The install, as `host~id8`.
    pub instance: String,
    #[ts(type = "number")]
    pub last_seen_at: i64,
    pub count: u32,
}
