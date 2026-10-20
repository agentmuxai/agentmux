// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.save` — docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md §6.1.

use serde::{Deserialize, Serialize};

/// Save one window's layout to a `*.agentmux-layout.json` file.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandLayoutPreviewData {
    /// Absolute path to a `*.agentmux-layout.json` file.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LayoutPreviewTab {
    pub name: String,
    /// One line per pane tab, e.g. "Agent: Reviewer", "Terminal in ~/src".
    pub panes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LayoutOpenResult {
    /// The workspace the tabs were added to.
    pub workspace_id: String,
    /// The tabs that were created, in order.
    pub tab_ids: Vec<String>,
    /// What couldn't be reproduced, including agents that didn't start.
    pub notes: Vec<String>,
}

/// Request for `layout.applyIntent`: one layout edit a window made to a tab,
/// for srv to compute on its own tree.
/// SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandLayoutIntentData {
    pub tab_id: String,
    /// The sender's id for this edit. Nodes srv creates for it get ids
    /// derived from it, and resending it is a no-op.
    pub action_id: String,
    pub intent: agentmux_common::LayoutIntent,
    /// The sender's focused and magnified pane after the edit; empty for none.
    pub focused_node_id: String,
    pub magnified_node_id: String,
}

/// Response from `layout.applyIntent`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LayoutIntentResult {
    /// False when srv refused the edit; `error` says why. The stored layout
    /// is then unchanged, and the sender shows that instead of its own.
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// The layout row's version once the edit is stored. A row update with
    /// this version or later includes it.
    #[ts(type = "number")]
    pub version: u64,
}

/// Request for `layout.otherBuilds`: the layouts other builds of AgentMux
/// saved when their last window closed (`session_other_builds`).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandOtherBuildLayoutsData {
    /// The window asking; decides `OtherBuildLayoutsResult::offer`.
    pub window_id: String,
    /// Look only when the window has a layout to offer over: nothing is
    /// read otherwise.
    #[serde(default)]
    #[ts(optional)]
    pub offer_only: Option<bool>,
}

/// One build's saved layout, described without opening it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS, PartialEq)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct OtherBuildLayout {
    /// The build's data channel, e.g. `stable` or a local build's own.
    pub channel: String,
    pub version: String,
    /// When it was saved, in ms since the epoch.
    #[ts(type = "number")]
    pub saved_at_ms: i64,
    /// False when the record has no time of its own and `saved_at_ms` is
    /// when that build last wrote its database.
    pub saved_at_exact: bool,
    pub windows: u32,
    pub tabs: u32,
    pub panes: u32,
    /// The agents in its panes, by name, each once.
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct OtherBuildLayoutsResult {
    /// The window still shows the default layout a cold start seeded because
    /// this build had nothing to restore, so offering one is welcome.
    pub offer: bool,
    /// Newest first.
    pub layouts: Vec<OtherBuildLayout>,
}

/// Request for `layout.restoreOtherBuild`: rebuild another build's saved
/// layout in this window. Its database is only read.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandRestoreOtherBuildLayoutData {
    pub window_id: String,
    pub channel: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RestoreOtherBuildLayoutResult {
    /// The window's workspace, which the tabs were added to.
    pub workspace_id: String,
    /// The tabs created in this window, in order.
    pub tab_ids: Vec<String>,
    /// The untouched default tab was removed, so the window shows only the
    /// restored layout.
    pub replaced_default: bool,
    /// The layout's other windows, rebuilt and waiting for this window to
    /// open them (`window.TakeRestoredWindows`).
    pub other_windows: u32,
}
