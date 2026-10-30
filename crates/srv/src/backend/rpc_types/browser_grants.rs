// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `browsergrants.*`: the person's standing grants (an agent's
//! powers on a site, until revoked) for the Settings screen. Only the user's
//! own window may call any of them.

use serde::{Deserialize, Serialize};

/// Data for `COMMAND_BROWSER_GRANTS_LIST`: nothing to ask.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandBrowserGrantsListData {}

/// Data for `COMMAND_BROWSER_GRANTS_SET`: give an agent powers on one site.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandBrowserGrantsSetData {
    pub agent: String,
    /// One site: `shell.example.com` or `https://shell.example.com:8443`.
    pub site: String,
    /// Powers beyond committing clicks (which every grant has): `terminal`.
    pub powers: Vec<String>,
}

/// Data for `COMMAND_BROWSER_GRANTS_PRESET`: turn on a console for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandBrowserGrantsPresetData {
    pub preset: String,
    pub agent: String,
}

/// Data for `COMMAND_BROWSER_GRANTS_REVOKE`: end one standing grant.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandBrowserGrantsRevokeData {
    pub id: String,
}

/// Data for `COMMAND_BROWSER_GRANTS_POWERS`: change what one grant carries.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandBrowserGrantsPowersData {
    pub id: String,
    pub powers: Vec<String>,
}

/// One standing grant, as the screen lists it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BrowserGrantRow {
    pub id: String,
    pub agent: String,
    /// The sites it covers: the one it is named by, then the frames that belong to it.
    pub origins: Vec<String>,
    pub powers: Vec<String>,
    /// Epoch milliseconds.
    #[ts(type = "number")]
    pub granted_at_ms: i64,
}

/// A console the screen offers to switch on.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BrowserGrantPreset {
    pub id: String,
    pub label: String,
    pub sites: Vec<String>,
    pub note: String,
}

/// Everything the screen shows: the grants, the presets, and the powers a row can carry.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BrowserGrantsResult {
    pub rows: Vec<BrowserGrantRow>,
    pub presets: Vec<BrowserGrantPreset>,
    pub powers: Vec<String>,
}
