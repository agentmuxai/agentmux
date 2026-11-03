// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The LAN discovery events' payloads (`mps_events`: `laninstances`, `:error`, `:health`).

use serde::Serialize;

use super::LanInstance;

/// `laninstances`: the LAN peers this instance sees now.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LanInstanceList(pub Vec<LanInstance>);

/// `laninstances:error`: why LAN discovery couldn't start, or a rebuild failed.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LanDiscoveryError {
    pub error: String,
}

/// `laninstances:health`: whether this instance is discoverable on every LAN interface.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LanDiscoverability {
    #[ts(type = r#""healthy" | "degraded" | "undiscoverable" | "off""#)]
    pub state: String,
    /// IPv4 addresses the service was expected on but never announced on.
    pub missing: Vec<String>,
    /// Daemon rebuilds the watchdog has already tried.
    pub rebuilds: u32,
}
