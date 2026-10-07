// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for pairing a device with the viewer listener (`viewer.*`,
//! agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.2).

use serde::{Deserialize, Serialize};

/// `viewer.pair-start`: the `agentmux://pair` URL the QR carries, and when its
/// code stops working (unix ms).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerPairStartResult {
    pub url: String,
    #[ts(type = "number")]
    pub expires_ms: i64,
}

/// One paired device in `viewer.devices`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerDeviceInfo {
    pub device_id: String,
    pub device_name: String,
    #[ts(type = "number")]
    pub created_ms: i64,
    /// 0 until the device's first request after pairing.
    #[ts(type = "number")]
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerDevicesResult {
    pub devices: Vec<ViewerDeviceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandViewerRevokeData {
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerRevokeResult {
    /// False when no such device was paired.
    pub revoked: bool,
    /// How many of its open feeds were closed.
    #[ts(type = "number")]
    pub feeds_closed: usize,
}

/// `viewer.agent-hidden`: read an agent's "hide from paired devices" flag, or
/// set it when `hidden` is given. `agent_id` is the definition id.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandViewerAgentHiddenData {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerAgentHiddenResult {
    pub hidden: bool,
}
