// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `workspace.importuserworkspace`: opening a saved workspace file as a new workspace. Whether its
//! terminal commands run, and its agents start, is the person's answer in the import preview, so only the person's own
//! window may call it; an agent's connection, or any connection that is not the window, is refused.

use serde::{Deserialize, Serialize};

/// Data for `COMMAND_WORKSPACE_IMPORT_USER_WORKSPACE`.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandWorkspaceImportUserWorkspaceData {
    /// The file to import, as the window's file dialog returned it.
    pub path: String,
    /// The person's answer in the preview: run the file's terminal commands. Left out, the file's trust decides
    /// (a file saved by this install runs, any other does not).
    #[serde(default)]
    #[ts(optional)]
    pub run_commands: Option<bool>,
    /// The window that imports, and switches to the new workspace.
    pub window_id: String,
}

/// A window the switch closed, for the host to close.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WorkspaceClosedWindow {
    pub window_id: String,
    pub label: String,
}

/// What the import did: the switch, and what the person should know about the file.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WorkspaceImportUserWorkspaceResult {
    pub id: String,
    pub name: String,
    /// The windows that closed, for the host to close.
    pub closed: Vec<WorkspaceClosedWindow>,
    /// How many windows wait to open beside the one that asked.
    pub beside: u32,
    /// What the person should know: commands left off, agents in closed windows.
    pub notes: Vec<String>,
}
