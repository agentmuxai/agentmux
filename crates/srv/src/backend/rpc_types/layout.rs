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

/// Read a layout file and describe what opening it would do, without
/// opening anything (spec §3.5 "Trust": the preview).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandLayoutPreviewData {
    /// Absolute path to a `*.agentmux-layout.json` file.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LayoutPreviewTab {
    pub name: String,
    /// One line per pane tab, e.g. "Agent: Reviewer", "Terminal in ~/src".
    pub panes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LayoutPreviewResult {
    pub name: String,
    /// Saved by this install and in its layouts folder: its terminal commands
    /// may run without asking. Anything else asks first.
    pub trusted: bool,
    pub tabs: Vec<LayoutPreviewTab>,
    /// Terminal commands in the file.
    pub commands: Vec<String>,
    /// What can't be reproduced here, or was left out.
    pub notes: Vec<String>,
}

/// Open a layout file's tabs: added as new tabs to an existing window
/// (nothing that's open is replaced), or into a new workspace for a new
/// window to show.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandLayoutOpenData {
    pub path: String,
    /// The window to add the tabs to. Not used with `new_window`.
    pub window_id: String,
    /// Start the file's terminal commands. Defaults to whether the file is
    /// trusted (see `LayoutPreviewResult::trusted`).
    #[serde(default)]
    #[ts(optional)]
    pub run_commands: Option<bool>,
    /// Build the tabs in a new workspace of their own instead; the caller
    /// then opens a window onto `LayoutOpenResult::workspace_id`.
    #[serde(default)]
    #[ts(optional)]
    pub new_window: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LayoutOpenResult {
    /// The workspace the tabs were added to.
    pub workspace_id: String,
    /// The tabs that were created, in order.
    pub tab_ids: Vec<String>,
    /// What couldn't be reproduced, including agents that didn't start.
    pub notes: Vec<String>,
}
