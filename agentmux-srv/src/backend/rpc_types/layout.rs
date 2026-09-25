// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.save` — docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md §6.1.

use serde::{Deserialize, Serialize};

/// Save one window's layout to a `*.agentmux-layout.json` file.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandLayoutSaveData {
    /// The window whose tabs are saved.
    pub window_id: String,
    /// The layout's display name, written into the file.
    pub name: String,
    /// Absolute destination, normally from the host's Save dialog. Must end
    /// in `.agentmux-layout.json`.
    pub path: String,
    /// Write machine-local resume references (agent session ids). Off by
    /// default — spec §3.3.
    #[serde(default)]
    #[ts(optional)]
    pub include_resume: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LayoutSaveResult {
    /// Where the file was written.
    pub path: String,
    /// Things the user should know before sharing the file (spec §3.4) —
    /// e.g. a terminal command that looks like it holds a credential.
    pub warnings: Vec<String>,
}
